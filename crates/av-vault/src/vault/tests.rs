use super::trash::{trash_prefix, untrashed};
use super::*;
use crate::change::Change;
use crate::envelope::Conversation;
use crate::envelope::DocumentType;
use crate::envelope::Transaction;
use crate::envelope::{Bare, Roles};
use crate::schema::layout;
use av_utils::datetime::DateTime;
use av_utils::id::Id;
use std::time::{Duration, SystemTime};

fn fixture_vault() -> (PathBuf, Id, Id, Id, Id, Id, Id) {
    let vault_id = Id::generate();
    let root = std::env::temp_dir().join(format!("av-vault-test-{vault_id}"));
    let write = |rel: &str, content: &str| {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    };

    let proj_id = Id::generate();
    write(
        "Projects/kitchen.yaml",
        &format!("id: {proj_id}\nproject: {{}}\ntitle: Kitchen renovation\n"),
    );

    let doc_id = Id::generate();
    write(
        "Notes/plan.md",
        &format!(
            "---\nid: {doc_id}\ndocument:\n  type: note\ntitle: Plan\nprojects: [{proj_id}]\ntags: [home]\n---\nBody\n"
        ),
    );

    let task_id = Id::generate();
    write(
        "Anywhere/nested/task.md",
        &format!(
            "---\nid: {task_id}\ntitle: Order countertop\ntask:\n  status: in-progress\nprojects: [{proj_id}]\n---\nNotes\n"
        ),
    );

    let cont_id = Id::generate();
    write(
        "Contacts/jane.yaml",
        &format!("id: {cont_id}\ntitle: Jane Doe\ncontact:\n  email: [jane@example.com]\n"),
    );

    let rec_id = Id::generate();
    write(
        "Records/no-type.yaml",
        &format!("id: {rec_id}\nfields: {{a: 1}}\n"),
    );

    write("Notes/unmanaged.md", "No frontmatter here.\n");
    write("Notes/broken.md", "---\n: [ not yaml\n---\nBody\n");

    let obs_id = Id::generate();
    write(
        ".obsidian/config.yaml",
        &format!("id: {obs_id}\nproject: {{}}\n"),
    );

    (root, proj_id, doc_id, task_id, cont_id, rec_id, obs_id)
}

#[test]
fn indexes_a_vault() {
    let (root, proj_id, doc_id, task_id, cont_id, rec_id, obs_id) = fixture_vault();

    let index = Index::build(&root);

    let project = index.get(&proj_id).unwrap();
    assert!(project.roles().project.is_some());
    assert_eq!(project.kind(), DocumentType::Project);
    assert_eq!(project.title(), "Kitchen renovation");

    let plan = index.get(&doc_id).unwrap();
    assert_eq!(
        plan.roles().document.as_ref().unwrap().doc_type,
        Some(crate::shared::DocType::Note)
    );

    let contact = index.get(&cont_id).unwrap();
    assert_eq!(
        contact.roles().contact.as_ref().unwrap().email,
        ["jane@example.com"]
    );

    let record = index.get(&rec_id).unwrap();
    assert_eq!(
        record.kind(),
        DocumentType::Record,
        "role-less YAML is a record"
    );

    let members = index.members_of(&proj_id);
    assert_eq!(members.len(), 2);
    assert!(members.iter().any(|d| d.doc.id() == doc_id));
    assert!(members.iter().any(|d| d.doc.id() == task_id));

    let task = index.get(&task_id).unwrap();
    let fields = task.roles().task.as_ref().expect("task role");
    assert_eq!(fields.status.as_deref(), Some("in-progress"));

    assert!(index.get(&obs_id).is_none(), ".obsidian must be skipped");

    let broken = index.broken();
    assert_eq!(
        broken.len(),
        1,
        "invalid frontmatter must be flagged, not hidden"
    );
    assert!(broken[0].path.ends_with("broken.md"));

    assert_eq!(index.all().len(), 7);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn an_indexed_entry_the_index_already_has_is_skipped() {
    let (root, proj_id, doc_id, ..) = fixture_vault();
    let mut index = Index::default();
    index.begin(&root);
    // Written by the app while the indexing walked the vault.
    let text = format!("---\nid: {doc_id}\ntitle: Plan, as the app wrote it\n---\n");
    index.add(Entry::unsaved(Document::load(text, FileFormat::Markdown)));

    let mut added = Vec::new();
    index_vault(&root, &mut HashMap::new(), |event| {
        added.extend(index.apply(event));
        IndexControl::Continue
    });

    assert!(added.contains(&proj_id));
    assert!(!added.contains(&doc_id));
    assert_eq!(added.len(), 6);
    assert_eq!(
        index.get(&doc_id).unwrap().title(),
        "Plan, as the app wrote it"
    );
    assert_eq!(index.all().len(), 7);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn indexing_ends_where_it_is_told_to_stop() {
    let (root, ..) = fixture_vault();
    let mut events = 0;
    index_vault(&root, &mut HashMap::new(), |event| {
        events += 1;
        match event {
            IndexEvent::Entry { .. } => IndexControl::Stop,
            _ => IndexControl::Continue,
        }
    });
    assert_eq!(events, 2, "the start and the first file");

    fs::remove_dir_all(&root).ok();
}

#[cfg(unix)]
#[test]
fn a_file_whose_name_isnt_unicode_fails_to_load() {
    use std::os::unix::ffi::OsStrExt;
    let (root, ..) = fixture_vault();
    let name = std::ffi::OsStr::from_bytes(b"Notes/K\xf6ln.md");
    fs::write(root.join(name), "Text").unwrap();
    let mut failed = Vec::new();
    index_vault(&root, &mut HashMap::new(), |event| {
        if let IndexEvent::Failed { path, error } = event {
            failed.push((path, error));
        }
        IndexControl::Continue
    });
    assert_eq!(
        failed,
        [(root.join(name), "its name isn't valid Unicode".to_string())]
    );

    fs::remove_dir_all(&root).ok();
}

/// An indexing of `root` given `known`: its entries, each with what it read.
fn index_known(
    root: &Path,
    known: &mut HashMap<PathBuf, FileText>,
) -> Vec<(Entry, Option<FileText>)> {
    let mut entries = Vec::new();
    index_vault(root, known, |event| {
        if let IndexEvent::Entry { entry, read, .. } = event {
            entries.push((entry, read));
        }
        IndexControl::Continue
    });
    entries
}

/// What an entry shows, without its id: a file without one gets a new
/// one per indexing.
fn shown(
    entry: &Entry,
) -> (
    PathBuf,
    String,
    String,
    String,
    LineEnding,
    Option<FileStamp>,
) {
    let doc = &entry.doc;
    let (title, content) = (doc.title().to_string(), doc.content().to_string());
    (
        entry.local.clone(),
        title,
        content,
        entry.bom.clone(),
        entry.crlf,
        doc.stamp(),
    )
}

#[test]
fn a_known_text_with_the_files_stamp_is_loaded_instead_of_the_file() {
    let (root, _, doc_id, ..) = fixture_vault();
    let local = Path::new("Notes").join("plan.md");
    let stamp = FileStamp::read(&root.join(&local)).unwrap().unwrap();
    let text = format!("---\nid: {doc_id}\ntitle: Kept\n---\nKept body\n");
    let mut known = HashMap::from([(local.clone(), FileText { stamp, text })]);

    let entries = index_known(&root, &mut known);
    let (plan, read) = entries.iter().find(|(e, _)| e.local == local).unwrap();
    assert_eq!(plan.doc.title(), "Kept");
    assert_eq!(plan.doc.content(), "Kept body\n");
    assert_eq!(plan.doc.stamp(), Some(stamp));
    assert!(read.is_none());
    assert!(known.is_empty());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn indexing_given_its_own_reads_loads_the_same_without_reading() {
    let (root, ..) = fixture_vault();
    let id = Id::generate();
    let text = format!("\u{feff}---\r\nid: {id}\r\ntitle: Windows\r\n---\r\nLine\r\n");
    fs::write(root.join("Notes/crlf.md"), text).unwrap();

    let first = index_known(&root, &mut HashMap::new());
    let mut known: HashMap<PathBuf, FileText> = first
        .iter()
        .map(|(entry, read)| (entry.local.clone(), read.clone().unwrap()))
        .collect();
    let second = index_known(&root, &mut known);

    // The stamps from the directory listing are those of the open files.
    assert!(second.iter().all(|(_, read)| read.is_none()));
    assert!(known.is_empty());
    let shown_by = |entries: &[(Entry, Option<FileText>)]| {
        entries.iter().map(|(e, _)| shown(e)).collect::<Vec<_>>()
    };
    assert_eq!(shown_by(&second), shown_by(&first));
    let crlf = second.iter().find(|(e, _)| e.doc.title() == "Windows");
    let (crlf, _) = crlf.unwrap();
    assert_eq!(
        (crlf.bom.as_str(), crlf.crlf),
        ("\u{feff}", LineEnding::CRLF)
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_changed_stamp_has_the_file_read_again() {
    let (root, ..) = fixture_vault();
    let local = Path::new("Notes").join("plan.md");
    let stamp = FileStamp::read(&root.join(&local)).unwrap().unwrap();
    let older = FileStamp::new(stamp.modified() - Duration::from_secs(1), stamp.size());
    let kept = FileText {
        stamp: older,
        text: "---\ntitle: Kept\n---\n".to_string(),
    };
    let mut known = HashMap::from([(local.clone(), kept)]);

    let entries = index_known(&root, &mut known);
    let (plan, read) = entries.iter().find(|(e, _)| e.local == local).unwrap();
    assert_eq!(plan.doc.title(), "Plan");
    assert_eq!(read.as_ref().unwrap().stamp, stamp);
    assert!(known.is_empty());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn after_indexing_the_gone_and_unreadable_files_are_left_known() {
    let (root, ..) = fixture_vault();
    fs::write(root.join("Notes/latin1.md"), b"caf\xe9\n").unwrap();
    let any = FileText {
        stamp: FileStamp::new(SystemTime::UNIX_EPOCH, 1),
        text: String::new(),
    };
    let paths = ["gone.md", "latin1.md", "plan.md"].map(|n| Path::new("Notes").join(n));
    let mut known: HashMap<PathBuf, FileText> =
        paths.iter().map(|p| (p.clone(), any.clone())).collect();

    let mut done = false;
    index_vault(&root, &mut known, |event| {
        done |= matches!(event, IndexEvent::Done);
        IndexControl::Continue
    });
    assert!(done);
    let mut left: Vec<PathBuf> = known.into_keys().collect();
    left.sort();
    assert_eq!(left, paths[..2]);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_stopped_indexing_leaves_what_it_didnt_reach_known() {
    let (root, ..) = fixture_vault();
    let first = index_known(&root, &mut HashMap::new());
    let mut known: HashMap<PathBuf, FileText> = first
        .iter()
        .map(|(entry, read)| (entry.local.clone(), read.clone().unwrap()))
        .collect();

    index_vault(&root, &mut known, |event| match event {
        IndexEvent::Entry { .. } => IndexControl::Stop,
        _ => IndexControl::Continue,
    });
    assert_eq!(known.len(), first.len() - 1);
    assert!(!known.contains_key(&first[0].0.local));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn added_unsaved_entity_is_indexed_but_not_saved() {
    use crate::envelope::{Bare, Roles, Thread};

    let mut index = Index::default();
    let roles = Roles {
        thread: Some(Thread::default()),
        ..Default::default()
    };
    let doc = Document::create(
        roles,
        Source::Created(Bare::default()),
        "New thread".into(),
        Vec::new(),
        FileFormat::YAML,
    );
    let id = doc.id();

    let entry = index.add(Entry::unsaved(doc));
    assert!(entry.is_unsaved());
    assert_eq!(entry.doc.format(), FileFormat::YAML);
    assert_eq!(index.get(&id).unwrap().kind(), DocumentType::Thread);
    assert!(matches!(index.save(&id, false), Err(Error::Unsaved)));
}

#[test]
fn remove_keeps_the_other_entries_findable() {
    use crate::envelope::{Bare, Roles};

    let mut index = Index::default();
    let ids: Vec<Id> = (0..3)
        .map(|i| {
            let doc = Document::create(
                Roles::default(),
                Source::Created(Bare::default()),
                format!("doc {i}"),
                Vec::new(),
                FileFormat::Markdown,
            );
            index.add(Entry::unsaved(doc)).doc.id()
        })
        .collect();

    let removed = index.remove(&ids[0]).expect("entry");
    assert_eq!(removed.doc.id(), ids[0]);
    assert!(index.get(&ids[0]).is_none());
    assert!(index.remove(&ids[0]).is_none(), "second remove is a no-op");

    // The last entry was moved into the freed slot; it must still resolve.
    assert_eq!(index.get(&ids[1]).unwrap().title(), "doc 1");
    assert_eq!(index.get(&ids[2]).unwrap().title(), "doc 2");
    assert_eq!(index.all().len(), 2);
}

fn thread_vault() -> (PathBuf, Vec<Id>) {
    let root = std::env::temp_dir().join(format!("av-vault-test-{}", Id::generate()));
    let ids: Vec<Id> = (0..3)
        .map(|i| {
            let id = Id::generate();
            let p = root.join(format!("Notes/n{i}.md"));
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(
                p,
                format!("---\nid: {id}\ndocument: {{}}\ntitle: Note {i}\n---\nBody {i}\n"),
            )
            .unwrap();
            id
        })
        .collect();
    (root, ids)
}

/// Create a thread of `members`, which must succeed; its id.
fn new_thread(index: &mut Index, title: &str, members: &[Id], changes: &mut Changes) -> Id {
    let created = index.create_thread(title.into(), members, changes);
    assert_eq!(created.unwrap(), SaveOutcome::Saved);
    changes.created.expect("created thread")
}

fn threads_of(index: &Index, id: &Id) -> Vec<Id> {
    let doc = index.get(id).unwrap();
    doc.roles()
        .message
        .as_ref()
        .map(|m| m.threads.clone())
        .unwrap_or_default()
}

#[test]
fn created_thread_and_membership_survive_reindexing() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);

    let mut changes = Changes::default();
    let thread = new_thread(&mut index, "Note 0", &ids[0..2], &mut changes);
    assert_eq!(changes.saved, ids[0..2]);

    let entry = index.entry(&thread).unwrap();
    assert!(entry.local.starts_with("Notes"), "next to the first member");
    let name = entry
        .local
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(
        name.contains(".note-0.") && name.ends_with(".thd.yaml"),
        "{name}"
    );

    let mut changes = Changes::default();
    let joined = index.join_thread(&ids[2], &thread, &mut changes);
    assert_eq!(joined.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [ids[2]]);
    let mut changes = Changes::default();
    let joined = index.join_thread(&ids[2], &thread, &mut changes);
    assert_eq!(joined.unwrap(), SaveOutcome::Unchanged);
    assert!(changes.saved.is_empty(), "joining twice writes nothing");

    let reindexed = Index::build(&root);
    assert_eq!(reindexed.get(&thread).unwrap().kind(), DocumentType::Thread);
    assert_eq!(reindexed.thread_members(&thread).len(), 3);
    assert_eq!(
        reindexed.get(&ids[0]).unwrap().content(),
        "Body 0\n",
        "body kept"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn created_project_survives_reindexing() {
    let (root, _) = thread_vault();
    let mut index = Index::build(&root);

    let mut changes = Changes::default();
    let project = index
        .create_project("New project".into(), &mut changes)
        .unwrap();
    assert_eq!(changes.created, Some(project));
    assert!(changes.saved.is_empty());

    let entry = index.entry(&project).unwrap();
    assert!(entry.local.starts_with("Projects"));
    let name = entry
        .local
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(
        name.contains(".new-project.") && name.ends_with(".prj.md"),
        "{name}"
    );
    assert!(entry.is_writable());

    let reindexed = Index::build(&root);
    let doc = reindexed.get(&project).unwrap();
    assert_eq!(doc.kind(), DocumentType::Project);
    assert_eq!(doc.title(), "New project");

    fs::remove_dir_all(&root).ok();
}

/// The new entity `new`, which must be created; its id and its file's
/// vault-relative path and text.
fn create_new(index: &mut Index, new: NewEntity) -> (Id, PathBuf, String) {
    let mut changes = Changes::default();
    let id = index.create_new(new, &mut changes).unwrap();
    assert_eq!(changes.created, Some(id));
    let entry = index.entry(&id).unwrap();
    let text = fs::read_to_string(&entry.path).unwrap();
    (id, entry.local.clone(), text)
}

/// The roles of `yaml`, a frontmatter fragment.
fn roles(yaml: &str) -> Roles {
    serde_yaml_ng::from_str(yaml).unwrap()
}

#[test]
fn a_new_task_lands_in_tasks_and_survives_reindexing() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let new = NewEntity {
        roles: roles("task: {status: todo, due: 2026-10-09}"),
        title: " Call the landlord ".into(),
        labels: vec![" home ".into(), "home".into(), "".into()],
        linked_to: Some(ids[0]),
        content: "Ask about the heating.".into(),
        ..Default::default()
    };
    let (id, local, text) = create_new(&mut index, new);

    assert!(local.starts_with(layout::TASKS), "{}", local.display());
    let name = local.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        name.contains(".call-the-landlord.") && name.ends_with(".tsk.md"),
        "{name}"
    );
    assert!(text.contains("created: {}"), "{text}");
    assert!(text.ends_with("---\nAsk about the heating.\n"), "{text}");

    let reindexed = Index::build(&root);
    let doc = reindexed.get(&id).unwrap();
    assert_eq!(doc.kind(), DocumentType::Task);
    assert_eq!(doc.title(), "Call the landlord");
    assert_eq!(doc.labels(), ["home"]);
    assert_eq!(doc.linked_to(), Some(ids[0]));
    let task = doc.roles().task.clone().unwrap();
    assert_eq!(task.status.as_deref(), Some("todo"));
    assert_eq!(
        task.due,
        Some(DateTime::from_iso8601("2026-10-09").unwrap())
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_new_list_is_yaml_with_its_items_and_no_body() {
    let (root, _) = thread_vault();
    let mut index = Index::build(&root);
    let new = NewEntity {
        roles: roles("list: {items: [Bank, Insurance]}"),
        title: "Change of address".into(),
        content: "ignored".into(),
        ..Default::default()
    };
    let (_, local, text) = create_new(&mut index, new);
    assert!(local.starts_with(layout::LISTS), "{}", local.display());
    assert!(local.to_string_lossy().ends_with(".lst.yaml"));
    assert!(text.contains("items:\n  - Bank\n  - Insurance\n"), "{text}");
    assert!(!text.contains("ignored") && !text.contains("---"), "{text}");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_new_event_is_dated_by_its_start() {
    let (root, _) = thread_vault();
    let mut index = Index::build(&root);
    let new = NewEntity {
        roles: roles("event: {start: 2026-10-07T07:30:00Z, end: 2026-10-07T08:30:00Z}"),
        title: "Dentist".into(),
        date: Some(DateTime::from_iso8601("2020-01-01").unwrap()),
        ..Default::default()
    };
    let (id, local, _) = create_new(&mut index, new);
    assert!(
        local.starts_with(layout::CREATED_EVENTS),
        "{}",
        local.display()
    );
    let name = local.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.starts_with("20261007T0730.dentist."), "{name}");
    let date = index.get(&id).unwrap().date();
    assert_eq!(
        date,
        Some(DateTime::from_iso8601("2026-10-07T07:30:00Z").unwrap())
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn new_refuses_other_types_empty_titles_and_unknown_references() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let refused = [
        (roles("project: {}"), "Move", vec![], "roles"),
        (roles("thread: {}"), "Trip", vec![], "roles"),
        (Roles::default(), "Nothing", vec![], "roles"),
        (roles("task: {}"), "  ", vec![], "title"),
        (roles("event: {}"), "No start", vec![], "event.start"),
        (roles("task: {}"), "Not a project", vec![ids[0]], "projects"),
    ];
    for (roles, title, projects, arg) in refused {
        let new = NewEntity {
            roles,
            title: title.into(),
            projects,
            ..Default::default()
        };
        let mut changes = Changes::default();
        match index.create_new(new, &mut changes) {
            Err(Error::InvalidArgument { arg: got }) => assert_eq!(got, arg, "{title}"),
            other => panic!("{title}: {other:?}"),
        }
        assert_eq!(changes.created, None, "{title}");
    }
    let unknown = NewEntity {
        roles: roles("task: {}"),
        title: "Linked to nothing".into(),
        linked_to: Some(Id::generate()),
        ..Default::default()
    };
    let refused = index.create_new(unknown, &mut Changes::default());
    assert!(matches!(refused, Err(Error::InvalidArgument { ref arg }) if arg == "linked_to"));
    assert_eq!(index.all().len(), 3, "nothing written");
    for folder in [layout::TASKS, layout::PROJECTS, layout::CREATED_EVENTS] {
        assert!(!root.join(folder).exists(), "{folder}");
    }

    fs::remove_dir_all(&root).ok();
}

#[test]
fn leaving_dissolves_a_thread_below_two_members() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    let thread = new_thread(&mut index, "T", &ids, &mut changes);
    let thread_file = index.entry(&thread).unwrap().local.clone();

    let mut changes = Changes::default();
    let left = index.leave_thread(&ids[2], &thread, &mut changes);
    assert_eq!(left.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [ids[2]]);
    assert!(changes.trashed.is_empty(), "two members left");

    let mut changes = Changes::default();
    let left = index.leave_thread(&ids[0], &thread, &mut changes);
    assert_eq!(left.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [ids[0], ids[1]]);
    assert_eq!(changes.trashed, [thread]);
    assert!(index.get(&thread).is_none());
    assert!(root.join(".trash").join(&thread_file).exists());

    let reindexed = Index::build(&root);
    assert!(reindexed.get(&thread).is_none());
    for id in &ids {
        assert!(threads_of(&reindexed, id).is_empty());
        let doc = reindexed.get(id).unwrap();
        assert!(doc.roles().message.is_none(), "empty message role dropped");
    }

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_read_only_member_refuses_the_whole_operation() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    let thread = new_thread(&mut index, "T", &ids[0..2], &mut changes);

    // Leaving would dissolve the thread and rewrite ids[1] -- read-only.
    let path = index.entry(&ids[1]).unwrap().path.clone();
    index.documents[index.by_id[&ids[1]]].readonly = true;
    let before = fs::read_to_string(&index.entry(&ids[0]).unwrap().path).unwrap();

    let mut changes = Changes::default();
    let err = index.leave_thread(&ids[0], &thread, &mut changes);
    assert!(matches!(err, Err(Error::NotWritable { .. })));
    assert!(changes.saved.is_empty() && changes.trashed.is_empty());
    assert_eq!(
        fs::read_to_string(&index.entry(&ids[0]).unwrap().path).unwrap(),
        before
    );
    assert_eq!(threads_of(&index, &ids[0]), [thread]);
    assert!(path.exists() && index.get(&thread).is_some());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn thread_ops_refuse_non_threads_and_unknown_ids() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();

    let err = index.join_thread(&ids[0], &ids[1], &mut changes);
    assert!(
        matches!(err, Err(Error::WrongKind)),
        "a document is not a thread"
    );
    let err = index.leave_thread(&ids[0], &Id::generate(), &mut changes);
    assert!(matches!(err, Err(Error::NotFound)));
    let err = index.create_thread("T".into(), &[ids[0], Id::generate()], &mut changes);
    assert!(matches!(err, Err(Error::NotFound)));
    assert!(changes.created.is_none() && changes.saved.is_empty());
    assert_eq!(
        fs::read_dir(root.join("Notes")).unwrap().count(),
        3,
        "no file written"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn deleting_a_standalone_entity_trashes_it_unchanged() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let before = fs::read_to_string(root.join("Notes/n0.md")).unwrap();

    let mut changes = Changes::default();
    assert_eq!(
        index.delete(&ids[0], &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(changes.trashed, [ids[0]]);
    assert!(changes.saved.is_empty());
    assert!(index.get(&ids[0]).is_none());
    assert!(!root.join("Notes/n0.md").exists());
    assert_eq!(
        fs::read_to_string(root.join(".trash/Notes/n0.md")).unwrap(),
        before
    );
    assert!(
        Index::build(&root).get(&ids[0]).is_none(),
        ".trash isn't indexed"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn deleting_a_member_dissolves_only_threads_it_leaves_below_two() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    let big = new_thread(&mut index, "Big", &ids, &mut changes);
    let small = new_thread(&mut index, "Small", &ids[0..2], &mut changes);
    let trashed_member = fs::read_to_string(index.entry(&ids[0]).unwrap().path.clone()).unwrap();

    let mut changes = Changes::default();
    assert_eq!(
        index.delete(&ids[0], &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(
        changes.saved,
        [ids[1]],
        "only the dissolved thread's last member"
    );
    assert_eq!(changes.trashed, [small, ids[0]]);
    assert_eq!(threads_of(&index, &ids[1]), [big]);
    assert_eq!(index.thread_members(&big).len(), 2);
    assert_eq!(
        fs::read_to_string(root.join(".trash/Notes/n0.md")).unwrap(),
        trashed_member,
        "the deleted file keeps its threads"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn deleting_refuses_read_only_files_and_threads() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    let thread = new_thread(&mut index, "T", &ids[0..2], &mut changes);

    let mut changes = Changes::default();
    let err = index.delete(&thread, &mut changes);
    assert!(matches!(err, Err(Error::InvalidArgument { .. })));

    // Deleting ids[0] would rewrite ids[1] to dissolve the thread.
    index.documents[index.by_id[&ids[1]]].readonly = true;
    let err = index.delete(&ids[0], &mut changes);
    assert!(matches!(err, Err(Error::NotWritable { .. })));
    index.documents[index.by_id[&ids[2]]].readonly = true;
    let err = index.delete(&ids[2], &mut changes);
    assert!(matches!(err, Err(Error::NotWritable { .. })));
    assert!(changes.saved.is_empty() && changes.trashed.is_empty());
    assert!(root.join("Notes/n0.md").exists() && root.join("Notes/n2.md").exists());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn deleting_a_project_takes_it_out_of_its_members_first() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    let project = index.create_project("P".into(), &mut changes).unwrap();
    let joined = index.join_project(&ids[0], &project, &mut changes);
    assert_eq!(joined.unwrap(), SaveOutcome::Saved);
    let created = index.create_thread("T".into(), &ids[1..3], &mut changes);
    assert_eq!(created.unwrap(), SaveOutcome::Saved);
    let thread = changes.created.unwrap();
    // a thread file written before threads lost their own projects
    let joined = index.join_project(&thread, &project, &mut changes);
    assert_eq!(joined.unwrap(), SaveOutcome::Saved);
    let local = index.entry(&project).unwrap().local.clone();

    let mut changes = Changes::default();
    assert_eq!(
        index.delete(&project, &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(changes.saved, [ids[0], thread], "a thread is a member too");
    assert_eq!(changes.trashed, [project]);
    assert!(index.get(&project).is_none());
    assert!(index.members_of(&project).is_empty());
    assert!(root.join(".trash").join(&local).exists());
    assert!(Index::build(&root).members_of(&project).is_empty());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn deleting_a_project_writes_nothing_unless_every_member_can_leave() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    let project = index.create_project("P".into(), &mut changes).unwrap();
    for id in &ids[0..2] {
        let joined = index.join_project(id, &project, &mut changes);
        assert_eq!(joined.unwrap(), SaveOutcome::Saved);
    }

    let mut changes = Changes::default();
    index.documents[index.by_id[&ids[1]]].readonly = true;
    let err = index.delete(&project, &mut changes);
    assert!(matches!(err, Err(Error::NotWritable { .. })));
    index.documents[index.by_id[&ids[1]]].readonly = false;

    let path = index.entry(&ids[1]).unwrap().path.clone();
    let theirs = fs::read_to_string(&path).unwrap() + "Changed outside\n";
    fs::write(&path, theirs).unwrap();
    let outcome = index.delete(&project, &mut changes).unwrap();
    assert_eq!(outcome, SaveOutcome::Conflict(ids[1]));

    assert!(changes.saved.is_empty() && changes.trashed.is_empty());
    assert_eq!(index.members_of(&project).len(), 2);
    assert!(index.entry(&project).unwrap().path.exists());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_deleted_entity_is_listed_in_the_trash_and_restored_as_it_was() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let before = fs::read_to_string(root.join("Notes/n0.md")).unwrap();
    let mut changes = Changes::default();
    let deleted = index.delete(&ids[0], &mut changes);
    assert_eq!(deleted.unwrap(), SaveOutcome::Saved);

    let trashed = index.trashed();
    assert_eq!(trashed.len(), 1);
    assert_eq!(trashed[0].local, Path::new("Notes").join("n0.md"));
    let doc = trashed[0].doc.as_ref().unwrap();
    assert_eq!((doc.id(), doc.title()), (ids[0], "Note 0"));

    let mut changes = Changes::default();
    let id = index.restore(&trashed[0].local, &mut changes).unwrap();
    assert_eq!(id, ids[0]);
    assert_eq!(changes.created, Some(ids[0]));
    assert_eq!(
        fs::read_to_string(root.join("Notes/n0.md")).unwrap(),
        before
    );
    assert_eq!(index.entry(&id).unwrap().local, Path::new("Notes/n0.md"));
    assert!(
        !root.join(".trash/Notes").exists(),
        "emptied folder removed"
    );
    assert!(index.trashed().is_empty());

    // Its stamp came along: the next edit is no conflict.
    let retitled = index.set_title(&id, "Back".into(), &mut changes);
    assert_eq!(retitled.unwrap(), SaveOutcome::Saved);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_restore_drops_the_prefix_of_a_clash() {
    let (root, _) = thread_vault();
    let mut index = Index::build(&root);
    let id = Id::generate();
    let trashed = root.join(".trash/Notes/20261005_142301.n9.md");
    fs::create_dir_all(trashed.parent().unwrap()).unwrap();
    fs::write(&trashed, format!("---\nid: {id}\ndocument: {{}}\n---\n")).unwrap();

    let local = Path::new("Notes/20261005_142301.n9.md");
    let restored = index.restore(local, &mut Changes::default()).unwrap();
    assert_eq!(restored, id);
    assert!(root.join("Notes/n9.md").exists() && !trashed.exists());

    // A generated name isn't such a prefix.
    let name = Path::new("Notes/20260915T0340.ad.80c3.doc.md");
    assert_eq!(untrashed(name), name);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_restore_is_refused_when_its_place_or_id_is_taken() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    let deleted = index.delete(&ids[0], &mut changes);
    assert_eq!(deleted.unwrap(), SaveOutcome::Saved);

    fs::write(root.join("Notes/n0.md"), "a new file\n").unwrap();
    let err = index.restore(Path::new("Notes/n0.md"), &mut changes);
    assert!(matches!(err, Err(Error::FileExists { path }) if path == Path::new("Notes/n0.md")));
    assert!(
        root.join(".trash/Notes/n0.md").exists(),
        "left in the trash"
    );

    let copy = root.join(".trash/Copies/n1.md");
    fs::create_dir_all(copy.parent().unwrap()).unwrap();
    fs::copy(root.join("Notes/n1.md"), &copy).unwrap();
    let err = index.restore(Path::new("Copies/n1.md"), &mut changes);
    assert!(matches!(err, Err(Error::FileExists { path }) if path == Path::new("Notes/n1.md")));
    assert!(copy.exists() && !root.join("Copies").exists());

    let err = index.restore(Path::new("../Notes/n1.md"), &mut changes);
    assert!(matches!(err, Err(Error::InvalidPath { .. })));
    let err = index.restore(&root.join("Notes/n1.md"), &mut changes);
    assert!(matches!(err, Err(Error::InvalidPath { .. })));
    let err = index.restore(Path::new("Notes/n0.txt"), &mut changes);
    assert!(matches!(err, Err(Error::InvalidFileType)));
    let err = index.restore(Path::new("Notes/gone.md"), &mut changes);
    assert!(matches!(err, Err(Error::FileNotFound { .. })));
    assert!(changes.created.is_none());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_trashed_file_is_rewritten_where_it_is() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    for id in &ids[..2] {
        let deleted = index.delete(id, &mut changes);
        assert_eq!(deleted.unwrap(), SaveOutcome::Saved);
    }
    let local = Path::new("Notes/n0.md");
    let edited = index.edit_trashed(local, |doc| doc.set_title("Gone".into()));
    assert_eq!(edited.unwrap(), SaveOutcome::Saved);
    let text = fs::read_to_string(root.join(".trash/Notes/n0.md")).unwrap();
    assert!(
        text.contains("title: Gone") && text.contains("Body 0"),
        "{text}"
    );
    assert!(index.entry(&ids[0]).is_none(), "still in the trash");
    let unchanged = index.edit_trashed(local, |doc| doc.set_title("Gone".into()));
    assert_eq!(unchanged.unwrap(), SaveOutcome::Unchanged);

    let read_only = root.join(".trash/Notes/n1.md");
    let mut permissions = fs::metadata(&read_only).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&read_only, permissions).unwrap();
    let err = index.edit_trashed(Path::new("Notes/n1.md"), |doc| doc.set_title("x".into()));
    assert!(matches!(err, Err(Error::NotWritable { .. })));
    let err = index.edit_trashed(Path::new("../Notes/n2.md"), |_| {});
    assert!(matches!(err, Err(Error::InvalidPath { .. })));
    let err = index.edit_trashed(Path::new("Notes/n2.md"), |_| {});
    assert!(matches!(err, Err(Error::FileNotFound { .. })));

    index.empty_trash().unwrap();
    fs::remove_dir_all(&root).ok();
}

#[test]
fn trashed_files_are_deleted_one_by_one_or_all_at_once() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    for id in &ids {
        let deleted = index.delete(id, &mut changes);
        assert_eq!(deleted.unwrap(), SaveOutcome::Saved);
    }
    let other = root.join(".trash/notes.txt");
    fs::write(&other, "not an entity\n").unwrap();
    let read_only = root.join(".trash/Notes/n2.md");
    let mut permissions = fs::metadata(&read_only).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&read_only, permissions).unwrap();

    index.delete_trashed(Path::new("Notes/n0.md")).unwrap();
    assert!(!root.join(".trash/Notes/n0.md").exists());
    assert_eq!(index.trashed().len(), 2);

    assert_eq!(index.empty_trash().unwrap(), 2, "the read-only one too");
    assert!(index.trashed().is_empty());
    assert!(
        !root.join(".trash/Notes").exists(),
        "emptied folder removed"
    );
    assert!(other.exists(), "what isn't listed stays");
    let err = index.delete_trashed(Path::new("Notes/n1.md"));
    assert!(matches!(err, Err(Error::FileNotFound { .. })));

    fs::remove_dir_all(&root).ok();
}

/// An empty vault at a new temporary root, and a document `title` to
/// write into it.
fn original_vault(title: &str) -> (PathBuf, Index, Document) {
    let root = std::env::temp_dir().join(format!("av-vault-test-{}", Id::generate()));
    let mut index = Index::default();
    index.begin(&root);
    let roles = Roles {
        document: Some(Default::default()),
        ..Default::default()
    };
    let source = Source::Created(Bare::default());
    let doc = Document::create(
        roles,
        source,
        title.into(),
        Vec::new(),
        FileFormat::Markdown,
    );
    (root, index, doc)
}

#[test]
fn a_mails_attachments_are_the_documents_linked_to_it_with_an_original() {
    let (root, mut index, email) = original_vault("Abschlag");
    let mut changes = Changes::default();
    let folder = Path::new("Attachments");
    let email = index
        .create(email, Path::new("Email"), &mut changes)
        .unwrap();
    let document = |title: &str| original_vault(title).2;
    let mut bill = document("Rechnung");
    bill.set_link(Some(email));
    let bill = index
        .create_with_original(bill, folder, "Rechnung.pdf", b"%PDF", &mut changes)
        .unwrap();
    let mut note = document("Notiz");
    note.set_link(Some(email));
    index
        .create(note, Path::new("Notes"), &mut changes)
        .unwrap();
    let other = document("Anderes");
    index
        .create_with_original(other, folder, "Anderes.pdf", b"%PDF", &mut changes)
        .unwrap();

    let found: Vec<Id> = index.attachments_of(email).map(|e| e.doc.id()).collect();
    assert_eq!(found, [bill], "not the note, not another email's");
    assert_eq!(index.attachments_of(bill).count(), 0);
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_document_is_written_with_its_original_and_trashed_with_it() {
    let (root, mut index, doc) = original_vault("Rechnung Oktober");
    let mut changes = Changes::default();
    let folder = Path::new("Attachments");
    let pdf = b"%PDF-1.7\r\n%\xe2\xe3\xcf\xd3\r\n";
    let id = index
        .create_with_original(doc, folder, "Rechnung_Oktober.PDF", pdf, &mut changes)
        .unwrap();
    assert_eq!(changes.created, Some(id));

    let entry = index.entry(&id).unwrap();
    let path = index.original_path(&id).unwrap();
    let file = path.file_name().unwrap().to_string_lossy().to_string();
    let doc_name = entry.local.file_name().unwrap().to_string_lossy();
    assert_eq!(file, doc_name.replace(".doc.md", ".doc.pdf"));
    assert_eq!(fs::read(&path).unwrap(), pdf, "byte for byte");
    let text = fs::read_to_string(&entry.path).unwrap();
    let block = format!(
        "document:\n  original:\n    file: {file}\n    name: Rechnung_Oktober.PDF\n    size: {}\n",
        pdf.len()
    );
    assert!(text.contains(&block), "{text}");
    let reindexed = Index::build(&root);
    assert_eq!(reindexed.documents.len(), 1, "the original is no entity");
    assert_eq!(reindexed.original_path(&id), Some(path.clone()));

    // A file of the original's name in the trash: both get the prefix.
    let stray = root.join(layout::TRASH).join(folder).join(&file);
    fs::create_dir_all(stray.parent().unwrap()).unwrap();
    fs::write(&stray, "stray\n").unwrap();
    let deleted = index.delete(&id, &mut changes);
    assert_eq!(deleted.unwrap(), SaveOutcome::Saved);
    assert!(!path.exists() && index.original_path(&id).is_none());
    let trashed = index.trashed();
    assert_eq!(trashed.len(), 1, "only the document is listed");
    let local = trashed[0].local.clone();
    let prefix = trash_prefix(&local).to_string();
    assert!(!prefix.is_empty(), "{}", local.display());
    let trashed_pdf = stray.with_file_name(format!("{prefix}{file}"));
    assert_eq!(fs::read(&trashed_pdf).unwrap(), pdf);
    assert_eq!(fs::read_to_string(&stray).unwrap(), "stray\n");

    // Restored without the prefix, the original with it.
    let restored = index.restore(&local, &mut changes).unwrap();
    assert_eq!(restored, id);
    assert_eq!(index.original_path(&id), Some(path.clone()));
    assert_eq!(fs::read(&path).unwrap(), pdf);
    assert!(!trashed_pdf.exists());

    // Deleted for good with its original; the stray file stays.
    let deleted = index.delete(&id, &mut changes);
    assert_eq!(deleted.unwrap(), SaveOutcome::Saved);
    let local = index.trashed()[0].local.clone();
    index.delete_trashed(&local).unwrap();
    let left: Vec<_> = fs::read_dir(stray.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(left, [stray.file_name().unwrap()]);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_restore_is_refused_when_its_originals_place_is_taken() {
    let (root, mut index, doc) = original_vault("Photo");
    let mut changes = Changes::default();
    let folder = Path::new("Attachments");
    let id = index
        .create_with_original(doc, folder, "photo.jpg", b"jpeg", &mut changes)
        .unwrap();
    let path = index.original_path(&id).unwrap();
    let deleted = index.delete(&id, &mut changes);
    assert_eq!(deleted.unwrap(), SaveOutcome::Saved);

    fs::write(&path, "another file\n").unwrap();
    let local = index.trashed()[0].local.clone();
    let err = index.restore(&local, &mut changes);
    let name = Path::new(path.file_name().unwrap());
    assert!(matches!(err, Err(Error::FileExists { path }) if path == folder.join(name)));
    assert!(index.entry(&id).is_none());
    let trashed = root.join(layout::TRASH).join(folder);
    assert!(trashed.join(local.file_name().unwrap()).exists() && trashed.join(name).exists());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn an_undo_moves_the_original_with_its_document() {
    let (root, mut index, doc) = original_vault("Scan");
    let folder = Path::new("Attachments");
    let mut made = Changes::default();
    let id = index
        .create_with_original(doc, folder, "scan.pdf", b"%PDF", &mut made)
        .unwrap();
    let path = index.original_path(&id).unwrap();
    let undone = index.replay(&mut made.files, Op::Undo, &mut Changes::default());
    assert_eq!(undone.unwrap(), SaveOutcome::Saved);
    assert!(!path.exists() && index.entry(&id).is_none(), "both trashed");
    let redone = index.replay(&mut made.files, Op::Redo, &mut Changes::default());
    assert_eq!(redone.unwrap(), SaveOutcome::Saved);
    assert_eq!(fs::read(&path).unwrap(), b"%PDF");

    let mut deleted = Changes::default();
    assert_eq!(index.delete(&id, &mut deleted).unwrap(), SaveOutcome::Saved);
    let undone = index.replay(&mut deleted.files, Op::Undo, &mut Changes::default());
    assert_eq!(undone.unwrap(), SaveOutcome::Saved);
    assert_eq!(fs::read(&path).unwrap(), b"%PDF");
    let redone = index.replay(&mut deleted.files, Op::Redo, &mut Changes::default());
    assert_eq!(redone.unwrap(), SaveOutcome::Saved);

    // Another file in the original's place: refused, nothing moves.
    fs::write(&path, "another file\n").unwrap();
    let undone = index.replay(&mut deleted.files, Op::Undo, &mut Changes::default());
    assert_eq!(undone.unwrap(), SaveOutcome::Conflict(id));
    assert_eq!(fs::read_to_string(&path).unwrap(), "another file\n");
    assert!(index.entry(&id).is_none());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn an_original_is_only_a_plain_name_and_never_an_entity() {
    let (root, mut index, doc) = original_vault("Notes");
    let mut changes = Changes::default();
    let id = index
        .create_with_original(doc, Path::new("A"), "notes.md", b"# Notes\n", &mut changes)
        .unwrap();
    let path = index.original_path(&id).unwrap();
    assert!(
        path.to_string_lossy().ends_with(".doc.md.txt"),
        "{}",
        path.display()
    );
    assert_eq!(
        Index::build(&root).documents.len(),
        1,
        "not read as an entity"
    );

    let other = index.entry(&id).unwrap().local.clone();
    let other = other.file_name().unwrap().to_string_lossy().to_string();
    fs::write(root.join("A/x.pdf"), "x").unwrap();
    for file in ["../x.pdf", "B/x.pdf", "/x.pdf", "", &other] {
        let text = format!(
            "---\nid: {}\ndocument:\n  original:\n    file: '{file}'\n---\n",
            Id::generate()
        );
        let doc = Document::load(text, FileFormat::Markdown);
        assert_eq!(original_file(&doc), None, "{file}");
    }
    let text = "---\ndocument:\n  original:\n    file: x.pdf\n---\n";
    let doc = Document::load(text.into(), FileFormat::Markdown);
    assert_eq!(original_file(&doc), Some("x.pdf"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn setting_a_title_rewrites_any_entity() {
    let (root, ids) = thread_vault();
    let project = Id::generate();
    fs::write(
        root.join("p.yaml"),
        format!("id: {project}\nproject: {{}}\ntitle: P\n"),
    )
    .unwrap();
    let mut index = Index::build(&root);

    let mut changes = Changes::default();
    let titled = index.set_title(&project, "  Kitchen  ".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
    let titled = index.set_title(&ids[0], "Renamed".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [project, ids[0]]);
    let yaml = fs::read_to_string(root.join("p.yaml")).unwrap();
    assert!(yaml.contains("title: Kitchen\n"), "{yaml}");
    let md = fs::read_to_string(root.join("Notes/n0.md")).unwrap();
    assert!(
        md.contains("title: Renamed\n") && md.ends_with("---\nBody 0\n"),
        "{md}"
    );

    // The same title writes nothing.
    let mut changes = Changes::default();
    let titled = index.set_title(&ids[0], "Renamed".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Unchanged);
    assert!(changes.saved.is_empty());

    // An empty title removes the key; it loads back as "".
    let titled = index.set_title(&ids[1], " ".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [ids[1]]);
    let md = fs::read_to_string(root.join("Notes/n1.md")).unwrap();
    assert!(!md.contains("title"), "{md}");
    let reindexed = Index::build(&root);
    assert_eq!(reindexed.get(&ids[1]).unwrap().title(), "");
    assert_eq!(reindexed.get(&project).unwrap().title(), "Kitchen");

    let err = index.set_title(&Id::generate(), "X".into(), &mut changes);
    assert!(matches!(err, Err(Error::NotFound)));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn starring_rewrites_any_entity() {
    let (root, ids) = thread_vault();
    let project = Id::generate();
    fs::write(
        root.join("p.yaml"),
        format!("id: {project}\nproject: {{}}\ntitle: P\n"),
    )
    .unwrap();
    let mut index = Index::build(&root);

    let mut changes = Changes::default();
    assert_eq!(
        index.set_starred(&project, true, &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(
        index.set_starred(&ids[0], true, &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(changes.saved, [project, ids[0]]);
    let yaml = fs::read_to_string(root.join("p.yaml")).unwrap();
    assert!(yaml.contains("starred: true\n"), "{yaml}");
    let reindexed = Index::build(&root);
    assert!(reindexed.get(&project).unwrap().starred());
    assert!(!reindexed.get(&ids[1]).unwrap().starred());

    // Starring again writes nothing; unstarring removes the key.
    let mut changes = Changes::default();
    assert_eq!(
        index.set_starred(&project, true, &mut changes).unwrap(),
        SaveOutcome::Unchanged
    );
    assert!(changes.saved.is_empty());
    assert_eq!(
        index.set_starred(&project, false, &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(changes.saved, [project]);
    let yaml = fs::read_to_string(root.join("p.yaml")).unwrap();
    assert!(!yaml.contains("starred"), "{yaml}");

    let err = index.set_starred(&Id::generate(), true, &mut changes);
    assert!(matches!(err, Err(Error::NotFound)));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn archiving_writes_the_flag_and_unarchiving_removes_it() {
    let (root, _) = thread_vault();
    let project = Id::generate();
    let path = root.join("p.yaml");
    fs::write(&path, format!("id: {project}\nproject: {{}}\ntitle: P\n")).unwrap();
    let mut index = Index::build(&root);

    let mut changes = Changes::default();
    assert_eq!(
        index.set_archived(&project, true, &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(changes.saved, [project]);
    assert!(Index::build(&root).get(&project).unwrap().archived());

    // Archiving again writes nothing; unarchiving removes the key.
    let mut changes = Changes::default();
    assert_eq!(
        index.set_archived(&project, true, &mut changes).unwrap(),
        SaveOutcome::Unchanged
    );
    assert_eq!(
        index.set_archived(&project, false, &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("archived"), "{text}");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn leaving_a_project_rewrites_the_member() {
    let (root, ids) = thread_vault();
    let project = Id::generate();
    fs::write(
        root.join("p.yaml"),
        format!("id: {project}\nproject: {{}}\ntitle: P\n"),
    )
    .unwrap();
    let p = root.join("Notes/n0.md");
    let text = fs::read_to_string(&p).unwrap().replacen(
        "title:",
        &format!("projects: [{project}]\ntitle:"),
        1,
    );
    fs::write(&p, text).unwrap();
    let mut index = Index::build(&root);
    assert_eq!(index.members_of(&project).len(), 1);

    let mut changes = Changes::default();
    let err = index.leave_project(&ids[0], &ids[1], &mut changes);
    assert!(matches!(err, Err(Error::WrongKind)), "not a project");
    let left = index.leave_project(&ids[0], &project, &mut changes);
    assert_eq!(left.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [ids[0]]);
    assert!(Index::build(&root).members_of(&project).is_empty());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn joining_a_project_rewrites_the_member() {
    let (root, ids) = thread_vault();
    let project = Id::generate();
    fs::write(
        root.join("p.yaml"),
        format!("id: {project}\nproject: {{}}\ntitle: P\n"),
    )
    .unwrap();
    let mut index = Index::build(&root);

    let mut changes = Changes::default();
    let err = index.join_project(&ids[0], &ids[1], &mut changes);
    assert!(matches!(err, Err(Error::WrongKind)), "not a project");
    let joined = index.join_project(&ids[0], &project, &mut changes);
    assert_eq!(joined.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [ids[0]]);
    assert_eq!(Index::build(&root).members_of(&project).len(), 1);

    // Joining again changes nothing, so nothing is written.
    let mut changes = Changes::default();
    let joined = index.join_project(&ids[0], &project, &mut changes);
    assert_eq!(joined.unwrap(), SaveOutcome::Unchanged);
    assert!(changes.saved.is_empty());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn labels_are_trimmed_kept_once_and_removed_with_the_last() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    let labels = [" bill ", "", "paid", "bill", "  "]
        .map(String::from)
        .to_vec();
    let set = index.set_labels(&ids[0], labels, &mut changes);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    assert_eq!(index.get(&ids[0]).unwrap().labels(), ["bill", "paid"]);

    let set = index.set_labels(&ids[0], Vec::new(), &mut changes);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    let text = fs::read_to_string(root.join("Notes/n0.md")).unwrap();
    assert!(!text.contains("tags"), "{text}");

    index.documents[index.by_id[&ids[1]]].readonly = true;
    let refused = index.set_labels(&ids[1], vec!["x".into()], &mut changes);
    assert!(matches!(refused, Err(Error::NotWritable { .. })));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_link_is_set_replaced_and_removed_on_any_entity() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    let set = index.set_link(&ids[0], Some(ids[1]), &mut changes);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    let set = index.set_link(&ids[0], Some(ids[2]), &mut changes);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [ids[0], ids[0]]);
    let reindexed = Index::build(&root);
    assert_eq!(reindexed.get(&ids[0]).unwrap().linked_to(), Some(ids[2]));

    let set = index.set_link(&ids[0], None, &mut changes);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    let text = fs::read_to_string(root.join("Notes/n0.md")).unwrap();
    assert!(!text.contains("linked-to"), "{text}");
    let again = index.set_link(&ids[0], None, &mut changes);
    assert_eq!(again.unwrap(), SaveOutcome::Unchanged);

    for to in [ids[0], Id::generate()] {
        let refused = index.set_link(&ids[0], Some(to), &mut changes);
        assert!(
            matches!(refused, Err(Error::InvalidArgument { ref arg }) if arg == "to"),
            "{refused:?}"
        );
    }
    index.documents[index.by_id[&ids[1]]].readonly = true;
    let refused = index.set_link(&ids[1], Some(ids[0]), &mut changes);
    assert!(matches!(refused, Err(Error::NotWritable { .. })));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_transactions_link_is_set_and_removed_in_its_statement() {
    let (root, ids) = thread_vault();
    let statement = Id::generate();
    let transaction = Id::generate();
    let path = root.join("Accounts/s.txs.yaml");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            "id: {statement}
title: Statement
statement:
  transactions:
  - id: {transaction}
    title: Rent
    amount: -1150.0
  - title: Fee
    amount: -4.9
"
        ),
    )
    .unwrap();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    let set = index.set_transaction_link(&statement, &transaction, Some(ids[0]), &mut changes);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [statement]);
    let reindexed = Index::build(&root);
    let items = reindexed
        .get(&statement)
        .unwrap()
        .roles()
        .statement
        .clone()
        .unwrap();
    let links: Vec<_> = items
        .transactions
        .iter()
        .map(|item| item.linked_to)
        .collect();
    assert_eq!(links, [Some(ids[0]), None], "only that transaction");

    let set = index.set_transaction_link(&statement, &transaction, None, &mut changes);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("linked-to"), "{text}");

    for (on, item, to, arg) in [
        (statement, Id::generate(), Some(ids[0]), "transaction"),
        (ids[1], transaction, Some(ids[0]), "transaction"),
        (statement, transaction, Some(statement), "to"),
        (statement, transaction, Some(Id::generate()), "to"),
    ] {
        let refused = index.set_transaction_link(&on, &item, to, &mut changes);
        assert!(
            matches!(refused, Err(Error::InvalidArgument { arg: ref a }) if a == arg),
            "{arg}: {refused:?}"
        );
    }
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_transfer_is_paired_by_hand_in_both_statements() {
    let (root, _) = thread_vault();
    let (checking, savings) = (Id::generate(), Id::generate());
    let (out, into, more, pending) = (
        Id::generate(),
        Id::generate(),
        Id::generate(),
        Id::generate(),
    );
    let write = |name: &str, text: String| {
        let path = root.join("Accounts").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    let (a, b) = (Id::generate(), Id::generate());
    write(
        "c.txs.yaml",
        format!(
            "id: {checking}\ntitle: Checking\nstatement:\n  account: {a}\n  transactions:\n  - id: {out}\n    status: posted\n    amount: -100.0\n  - id: {more}\n    status: posted\n    amount: 100.0\n"
        ),
    );
    write(
        "s.txs.yaml",
        format!(
            "id: {savings}\ntitle: Savings\nstatement:\n  account: {b}\n  transactions:\n  - id: {into}\n    status: posted\n    amount: 100.0\n  - id: {pending}\n    status: pending\n    amount: 100.0\n"
        ),
    );
    let mut index = Index::build(&root);
    let mut changes = Changes::default();

    // a pending half, one in the same account, amounts that aren't opposite
    for (on, with, half, arg) in [
        (out, savings, pending, "transaction"),
        (out, checking, more, "other"),
        (more, savings, into, "other"),
    ] {
        let refused = index.pair_transfer(&checking, &on, &with, &half, &mut changes);
        assert!(
            matches!(refused, Err(Error::InvalidArgument { arg: ref a }) if a == arg),
            "{arg}: {refused:?}"
        );
    }
    let paired = index.pair_transfer(&checking, &out, &savings, &into, &mut changes);
    assert_eq!(paired.unwrap(), SaveOutcome::Saved);
    let half = |statement: &Id, id: &Id| {
        let doc = index.get(statement).unwrap();
        let items = &doc.roles().statement.as_ref().unwrap().transactions;
        items.iter().find(|b| b.id == Some(*id)).unwrap().transfer
    };
    assert_eq!(
        (half(&checking, &out), half(&savings, &into)),
        (Some(into), Some(out))
    );
    let again = index.pair_transfer(&checking, &out, &savings, &into, &mut changes);
    assert!(
        matches!(again, Err(Error::InvalidArgument { .. })),
        "a half already"
    );
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_transfer_is_unpaired_in_both_statements() {
    let (root, _) = thread_vault();
    let (checking, savings) = (Id::generate(), Id::generate());
    let (out, into, fee) = (Id::generate(), Id::generate(), Id::generate());
    let write = |name: &str, text: String| {
        let path = root.join("Accounts").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write(
        "c.txs.yaml",
        format!(
            "id: {checking}
title: Checking
statement:
  transactions:
  - id: {out}
    amount: -100.0
    transfer: {into}
  - id: {fee}
    amount: -4.9
"
        ),
    );
    write(
        "s.txs.yaml",
        format!(
            "id: {savings}
title: Savings
statement:
  transactions:
  - id: {into}
    amount: 100.0
    transfer: {out}
"
        ),
    );
    let mut index = Index::build(&root);
    let mut changes = Changes::default();

    let left = index.unpair_transfer(&checking, &fee, &mut changes);
    assert_eq!(left.unwrap(), SaveOutcome::Unchanged, "no transfer's half");
    let refused = index.unpair_transfer(&checking, &Id::generate(), &mut changes);
    assert!(
        matches!(refused, Err(Error::InvalidArgument { ref arg }) if arg == "transaction"),
        "{refused:?}"
    );
    index.documents[index.by_id[&savings]].readonly = true;
    let refused = index.unpair_transfer(&checking, &out, &mut changes);
    assert!(
        matches!(refused, Err(Error::NotWritable { .. })),
        "{refused:?}"
    );
    assert!(changes.saved.is_empty(), "nothing written");
    index.documents[index.by_id[&savings]].readonly = false;

    let unpaired = index.unpair_transfer(&checking, &out, &mut changes);
    assert_eq!(unpaired.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [checking, savings]);
    for name in ["c.txs.yaml", "s.txs.yaml"] {
        let text = fs::read_to_string(root.join("Accounts").join(name)).unwrap();
        assert!(!text.contains("transfer"), "{name}: {text}");
    }
    fs::remove_dir_all(&root).ok();
}

#[test]
fn transactions_join_a_recurring_payment_of_their_account() {
    let (root, _) = thread_vault();
    let (account, other) = (Id::generate(), Id::generate());
    let (statement, recurring_payment, elsewhere) =
        (Id::generate(), Id::generate(), Id::generate());
    let (rent, again, fee) = (Id::generate(), Id::generate(), Id::generate());
    let write = |name: &str, text: String| {
        let path = root.join("Accounts").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write(
        "s.txs.yaml",
        format!(
            "id: {statement}
title: Checking
statement:
  account: {account}
  transactions:
  - id: {rent}
    amount: -1150.0
  - id: {again}
    amount: -1150.0
  - id: {fee}
    amount: -4.9
"
        ),
    );
    let recurring_payment_of = |id: Id, account: Id| {
        format!("id: {id}\ntitle: Rent\nrecurring-payment:\n  account: {account}\n")
    };
    write(
        "rent.rcp.yaml",
        recurring_payment_of(recurring_payment, account),
    );
    write("other.rcp.yaml", recurring_payment_of(elsewhere, other));
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    assert_eq!(
        index.get(&recurring_payment).unwrap().kind(),
        DocumentType::RecurringPayment
    );

    let joined = index.set_transaction_recurring_payment(
        &statement,
        &[rent, again],
        Some(recurring_payment),
        &mut changes,
    );
    assert_eq!(joined.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [statement], "one write");
    let items = Index::build(&root)
        .get(&statement)
        .unwrap()
        .roles()
        .statement
        .clone();
    let names: Vec<_> = items
        .unwrap()
        .transactions
        .iter()
        .map(|item| item.recurring_payment)
        .collect();
    assert_eq!(
        names,
        [Some(recurring_payment), Some(recurring_payment), None]
    );

    let out = index.set_transaction_recurring_payment(&statement, &[again], None, &mut changes);
    assert_eq!(out.unwrap(), SaveOutcome::Saved);
    let text = fs::read_to_string(root.join("Accounts/s.txs.yaml")).unwrap();
    assert_eq!(text.matches("recurring-payment:").count(), 1, "{text}");

    for (transactions, to, arg) in [
        (vec![Id::generate()], Some(recurring_payment), "transaction"),
        (vec![fee], Some(elsewhere), "to"),
        (vec![fee], Some(statement), "to"),
        (vec![fee], Some(Id::generate()), "to"),
    ] {
        let refused =
            index.set_transaction_recurring_payment(&statement, &transactions, to, &mut changes);
        assert!(
            matches!(refused, Err(Error::InvalidArgument { arg: ref a }) if a == arg),
            "{arg}: {refused:?}"
        );
    }
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_write_keeps_the_byte_order_mark_and_the_line_endings() {
    let (root, ids) = thread_vault();
    let path = root.join("Notes/n0.md");
    let text = fs::read_to_string(&path).unwrap().replace('\n', "\r\n");
    fs::write(&path, format!("\u{feff}{text}")).unwrap();
    let mut index = Index::build(&root);

    let mut changes = Changes::default();
    let titled = index.set_title(&ids[0], "Renamed".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
    let written = fs::read_to_string(&path).unwrap();
    assert!(written.starts_with("\u{feff}---\r\n"), "{written:?}");
    assert!(written.contains("title: Renamed\r\n") && written.ends_with("Body 0\r\n"));
    assert!(!written.replace("\r\n", "").contains('\n'), "{written:?}");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn read_only_and_broken_files_are_not_written() {
    let (root, ids) = thread_vault();
    fs::write(
        root.join("Notes/broken.md"),
        "---\n: [ not yaml\n---\nBody\n",
    )
    .unwrap();
    let mut index = Index::build(&root);
    let broken = index.broken()[0].doc.id();
    index.documents[index.by_id[&ids[0]]].readonly = true;
    let before = fs::read_to_string(root.join("Notes/n0.md")).unwrap();

    let mut changes = Changes::default();
    let err = index.set_title(&ids[0], "Mine".into(), &mut changes);
    assert!(matches!(err, Err(Error::NotWritable { .. })));
    let err = index.set_title(&broken, "Mine".into(), &mut changes);
    assert!(matches!(err, Err(Error::NotWritable { .. })));
    assert!(changes.saved.is_empty());
    assert_eq!(
        fs::read_to_string(root.join("Notes/n0.md")).unwrap(),
        before
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn remote_properties_find_imported_entities() {
    use crate::envelope::{Email, Remote};
    let root = std::env::temp_dir().join(format!("av-vault-test-{}", Id::generate()));
    let mut index = Index::default();
    index.begin(&root);
    let (account, other) = (Id::generate(), Id::generate());

    let mut remote = Remote {
        account: Some(account),
        ..Remote::default()
    };
    remote.set("message-id", "<a@x>");
    remote.set("gmail-id", "g1");
    let remote = Source::Remote(remote);
    // An attachment names its email's message, and comes first here.
    let roles = Roles {
        document: Some(Default::default()),
        ..Default::default()
    };
    let mut changes = Changes::default();
    let attachment = Document::create(
        roles,
        remote.clone(),
        "a.pdf".into(),
        Vec::new(),
        FileFormat::Markdown,
    );
    let attachment = index
        .create(attachment, Path::new("Attachments"), &mut changes)
        .unwrap();
    let roles = Roles {
        email: Some(Email::default()),
        ..Default::default()
    };
    let mut doc = Document::create(
        roles,
        remote,
        "Email".into(),
        Vec::new(),
        FileFormat::Markdown,
    );
    doc.set_date(DateTime::from_unix_millis(1_790_503_530_000));
    doc.set_content("Body\n".into());
    let id = index.create(doc, Path::new("Email"), &mut changes).unwrap();
    assert!(index.entry(&id).unwrap().local.starts_with("Email"));
    assert!(
        fs::read_to_string(root.join(&index.entry(&id).unwrap().local))
            .unwrap()
            .contains("date: 2026-09-27T10:05:30Z")
    );

    let email = DocumentType::Email;
    assert_eq!(
        index.find_remote(email, Some(&account), "gmail-id", "g1"),
        Some(id)
    );
    assert_eq!(
        index.find_remote(DocumentType::Document, None, "gmail-id", "g1"),
        Some(attachment)
    );
    let find = |account, key, value| index.find_remote(email, account, key, value);
    assert_eq!(find(Some(&other), "gmail-id", "g1"), None);
    assert_eq!(find(None, "message-id", "<a@x>"), Some(id));
    assert_eq!(find(None, "message-id", "<b@x>"), None);
    assert_eq!(
        index.remote_values(email, Some(&account), "gmail-id"),
        HashSet::from(["g1".to_string()])
    );
    assert!(
        index
            .remote_values(email, Some(&other), "gmail-id")
            .is_empty()
    );
    let contacts = index.remote_values(DocumentType::Contact, None, "gmail-id");
    assert!(contacts.is_empty());

    let mut changes = Changes::default();
    let set = index.set_content(&id, "New body\n".into(), &mut changes);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [id]);
    let text = fs::read_to_string(root.join(&index.entry(&id).unwrap().local)).unwrap();
    assert!(text.ends_with("New body\n") && text.contains("gmail-id: g1"));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn scan_origins_name_the_files_imported_from_a_watch() {
    use crate::envelope::{Document as DocumentRole, Scan};
    use crate::shared::OneOrMany;
    let root = std::env::temp_dir().join(format!("av-vault-test-{}", Id::generate()));
    let mut index = Index::default();
    index.begin(&root);
    let (watch, other) = (Id::generate(), Id::generate());
    let mut create = |watch: Option<Id>, origin: &[&str]| {
        let scan = Scan {
            watch,
            origin: OneOrMany::from_vec(origin.iter().map(|o| o.to_string()).collect()),
            ..Scan::default()
        };
        let roles = Roles {
            document: Some(DocumentRole::default()),
            ..Default::default()
        };
        let doc = Document::create(
            roles,
            Source::Scan(scan),
            "Letter".into(),
            Vec::new(),
            FileFormat::Markdown,
        );
        index
            .create(doc, Path::new("Scans"), &mut Changes::default())
            .unwrap()
    };
    let letter = create(Some(watch), &["p1.jpg", "sub\\p2.jpg"]);
    let receipt = create(Some(watch), &["r.png"]);
    create(Some(other), &["x.jpg"]);
    create(None, &["y.jpg"]);

    let text = fs::read_to_string(root.join(&index.entry(&letter).unwrap().local)).unwrap();
    let reindexed = Index::build(&root);
    fs::remove_dir_all(&root).unwrap();

    assert!(
        text.contains(&format!("scan:\n  watch: {watch}\n  origin:\n  - p1.jpg\n")),
        "{text}"
    );
    let expected = HashMap::from([
        ("p1.jpg".to_string(), letter),
        ("sub\\p2.jpg".to_string(), letter),
        ("r.png".to_string(), receipt),
    ]);
    assert_eq!(index.scan_origins(&watch), expected);
    assert_eq!(
        reindexed.scan_origins(&watch),
        expected,
        "read back from the files"
    );
}

#[test]
fn editing_writes_several_properties_at_once() {
    use crate::envelope::{Event, Remote};
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let start = DateTime::from_unix_millis(1_790_503_530_000);

    let mut changes = Changes::default();
    let edited = index.edit(&ids[0], &mut changes, |doc| {
        doc.set_title("Dentist".into());
        doc.set_date(start);
        doc.set_labels(vec!["conflict".into()]);
        doc.set_event(Some(Event {
            start: Some(start),
            ..Default::default()
        }));
        let mut remote = Remote::default();
        remote.set("event-id", "e1");
        doc.set_source(Some(Source::Remote(remote)));
        doc.set_content("Moved\n".into());
    });
    assert_eq!(edited.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [ids[0]], "one write");
    assert_eq!(changes.content, [ids[0]], "the body changed too");

    let again = Index::build(&root);
    let doc = again.get(&ids[0]).unwrap();
    assert_eq!(doc.title(), "Dentist");
    assert_eq!(doc.date(), Some(start));
    assert_eq!(doc.labels(), ["conflict"]);
    assert_eq!(doc.kind(), DocumentType::Event);
    assert!(matches!(&doc.source(), Some(Source::Remote(r)) if r.get("event-id") == Some("e1")));
    assert_eq!(doc.content(), "Moved\n");

    // An edit that changes nothing writes nothing.
    let mut changes = Changes::default();
    let edited = index.edit(&ids[0], &mut changes, |doc| doc.set_title("Dentist".into()));
    assert_eq!(edited.unwrap(), SaveOutcome::Unchanged);
    assert!(changes.saved.is_empty());
    let err = index.edit(&Id::generate(), &mut changes, |_| {});
    assert!(matches!(err, Err(Error::NotFound)));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_write_records_whether_it_changed_the_body() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);

    let mut changes = Changes::default();
    let titled = index.set_title(&ids[0], "Renamed".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [ids[0]]);
    assert!(changes.content.is_empty(), "only the frontmatter changed");

    let mut changes = Changes::default();
    let replaced = index.set_content(&ids[1], "New body\n".into(), &mut changes);
    assert_eq!(replaced.unwrap(), SaveOutcome::Saved);
    assert_eq!(
        (changes.saved, changes.content),
        (vec![ids[1]], vec![ids[1]])
    );

    // The same body again writes nothing, so it records nothing.
    let mut changes = Changes::default();
    let again = index.set_content(&ids[1], "New body\n".into(), &mut changes);
    assert_eq!(again.unwrap(), SaveOutcome::Unchanged);
    assert!(changes.saved.is_empty() && changes.content.is_empty());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn an_undo_records_whether_it_changed_the_body() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut replaced = Changes::default();
    let set = index.set_content(&ids[1], "New body\n".into(), &mut replaced);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    let mut titled = Changes::default();
    let set = index.set_title(&ids[0], "Renamed".into(), &mut titled);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);

    let mut changes = Changes::default();
    let undone = index.replay(&mut titled.files, Op::Undo, &mut changes);
    assert_eq!(undone.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [ids[0]]);
    assert!(changes.content.is_empty(), "only the frontmatter went back");

    let mut changes = Changes::default();
    let undone = index.replay(&mut replaced.files, Op::Undo, &mut changes);
    assert_eq!(undone.unwrap(), SaveOutcome::Saved);
    assert_eq!(
        (changes.saved, changes.content),
        (vec![ids[1]], vec![ids[1]])
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_step_leaves_the_source_block_to_its_writer() {
    use crate::envelope::Remote;
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut titled = Changes::default();
    let set = index.set_title(&ids[0], "Renamed".into(), &mut titled);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    // A sync's bookkeeping since, after it pushed the title.
    let mut remote = Remote::default();
    remote.set("etag", "\"2\"");
    let source = Some(Source::Remote(remote));
    let pushed = index.edit(&ids[0], &mut Changes::default(), |doc| {
        doc.set_source(source.clone())
    });
    assert_eq!(pushed.unwrap(), SaveOutcome::Saved);

    let undone = index.replay(&mut titled.files, Op::Undo, &mut Changes::default());
    assert_eq!(undone.unwrap(), SaveOutcome::Saved);
    let doc = index.get(&ids[0]).unwrap();
    assert_eq!(doc.title(), "Note 0");
    assert_eq!(doc.envelope().unwrap().source, source, "kept");
    let redone = index.replay(&mut titled.files, Op::Redo, &mut Changes::default());
    assert_eq!(redone.unwrap(), SaveOutcome::Saved);

    // What else a writer changes is the conflict.
    let pulled = index.edit(&ids[0], &mut Changes::default(), |doc| {
        doc.set_title("Theirs".into())
    });
    assert_eq!(pulled.unwrap(), SaveOutcome::Saved);
    let undone = index.replay(&mut titled.files, Op::Undo, &mut Changes::default());
    assert_eq!(undone.unwrap(), SaveOutcome::Conflict(ids[0]));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn conversations_are_written_and_read_back() {
    use crate::envelope::ConversationMessage;
    let root = std::env::temp_dir().join(format!("av-vault-test-{}", Id::generate()));
    let mut index = Index::default();
    index.begin(&root);

    let (question, answer, linked) = (Id::generate(), Id::generate(), Id::generate());
    let mut conversation = Conversation {
        messages: vec![ConversationMessage {
            id: Some(question),
            content: Some("What is due?".into()),
            ..Default::default()
        }],
        active: Some(question),
        ..Conversation::default()
    };
    let roles = Roles {
        conversation: Some(conversation.clone()),
        ..Default::default()
    };
    let mut doc = Document::create(
        roles,
        Source::Ai(Bare::default()),
        "What is due?".into(),
        Vec::new(),
        FileFormat::YAML,
    );
    doc.set_link(Some(linked));
    let mut changes = Changes::default();
    let id = index
        .create(doc, Path::new("Conversations"), &mut changes)
        .unwrap();
    let local = index.entry(&id).unwrap().local.clone();
    assert!(local.starts_with("Conversations"));
    assert!(local.to_string_lossy().ends_with(".cvn.yaml"), "{local:?}");

    conversation.messages.push(ConversationMessage {
        id: Some(answer),
        parent: Some(question),
        model: Some("gpt-5-mini".into()),
        content: Some("Nothing.".into()),
        ..Default::default()
    });
    conversation.active = Some(answer);
    let mut changes = Changes::default();
    let set = index.set_conversation(&id, Some(conversation.clone()), &mut changes);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [id]);

    let again = Index::build(&root);
    fs::remove_dir_all(&root).unwrap();
    let doc = again.get(&id).expect("indexed back");
    assert_eq!(doc.kind(), DocumentType::Conversation);
    assert_eq!(doc.source(), Some(&Source::Ai(Bare::default())));
    assert_eq!(doc.roles().conversation.as_ref(), Some(&conversation));
    assert_eq!(doc.linked_to(), Some(linked));
}

/// Replace `id`'s file in `thread_vault` as another program would.
fn edit_outside(root: &Path, i: usize, id: &Id, body: &str) -> PathBuf {
    let path = root.join(format!("Notes/n{i}.md"));
    let text = format!("---\nid: {id}\ndocument: {{}}\ntitle: Outside\n---\n{body}");
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn a_file_changed_outside_is_not_overwritten() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let path = edit_outside(&root, 0, &ids[0], "Theirs, longer\n");
    let theirs = fs::read_to_string(&path).unwrap();

    let saved = index.save_content(&ids[0], "Mine\n".into(), false, &mut Changes::default());
    assert_eq!(saved.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert_eq!(fs::read_to_string(&path).unwrap(), theirs);
    let doc = index.get(&ids[0]).unwrap();
    assert_eq!(doc.content(), "Body 0\n", "memory still matches the file");
    assert!(!doc.is_dirty());

    let mut changes = Changes::default();
    let titled = index.set_title(&ids[0], "Mine".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert!(changes.saved.is_empty());
    assert_eq!(index.get(&ids[0]).unwrap().title(), "Note 0");
    assert_eq!(fs::read_to_string(&path).unwrap(), theirs);

    // Overwriting writes; after that, our own saves don't conflict.
    let saved = index.save_content(&ids[0], "Mine\n".into(), true, &mut Changes::default());
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("title: Note 0") && text.ends_with("Mine\n"),
        "{text}"
    );
    let saved = index.save_content(
        &ids[0],
        "Mine again\n".into(),
        false,
        &mut Changes::default(),
    );
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);
    let saved = index.save_content(
        &ids[0],
        "Mine again\n".into(),
        false,
        &mut Changes::default(),
    );
    assert_eq!(saved.unwrap(), SaveOutcome::Unchanged);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_conflict_puts_back_everything_an_edit_changed() {
    use crate::envelope::Event;
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let before = index.get(&ids[0]).unwrap().save();
    edit_outside(&root, 0, &ids[0], "Theirs, longer\n");

    let mut changes = Changes::default();
    let edited = index.edit(&ids[0], &mut changes, |doc| {
        doc.set_title("Mine".into());
        doc.set_labels(vec!["mine".into()]);
        doc.set_starred(true);
        doc.set_date(DateTime::now());
        doc.set_event(Some(Event::default()));
        doc.join_project(Id::generate());
        doc.set_content("Mine\n".into());
    });
    assert_eq!(edited.unwrap(), SaveOutcome::Conflict(ids[0]));
    let doc = index.get(&ids[0]).unwrap();
    assert_eq!(doc.save(), before, "as the app last read it");
    assert!(!doc.is_dirty());
    assert!(changes.saved.is_empty());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_file_without_frontmatter_gets_one_only_when_metadata_is_set() {
    let root = std::env::temp_dir().join(format!("av-vault-test-{}", Id::generate()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("plain.md");
    fs::write(&path, "Just text\n").unwrap();
    let mut index = Index::build(&root);
    let id = index.all()[0].doc.id();
    assert!(index.get(&id).unwrap().envelope().is_none());

    let mut changes = Changes::default();
    let titled = index.set_title(&id, "Plain".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text, "---\ntitle: Plain\n---\nJust text\n");

    let mut undone = Changes::default();
    assert_eq!(
        index
            .replay(&mut changes.files, Op::Undo, &mut undone)
            .unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "Just text\n",
        "undone, none again"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_step_is_undone_and_redone() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let path = root.join("Notes/n0.md");

    let mut step = Changes::default();
    let titled = index.set_title(&ids[0], "Renamed".into(), &mut step);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
    let renamed = fs::read_to_string(&path).unwrap();
    let [
        FileStep {
            id,
            before: Place::Vault(before),
            after: Place::Vault(after),
            ..
        },
    ] = &step.files[..]
    else {
        panic!("{:?}", step.files);
    };
    assert_eq!(*id, ids[0]);
    assert!(
        before.body.is_none() && after.body.is_none(),
        "a title keeps no body"
    );

    let mut changes = Changes::default();
    let undone = index.replay(&mut step.files, Op::Undo, &mut changes);
    assert_eq!(undone.unwrap(), SaveOutcome::Saved);
    assert_eq!(changes.saved, [ids[0]]);
    let undone = fs::read_to_string(&path).unwrap();
    let keys = format!(
        "---\nid: {}\ntitle: Note 0\ndocument: {{}}\n---\nBody 0\n",
        ids[0]
    );
    assert_eq!(undone, keys, "what it held, its keys in the app's order");
    assert_eq!(index.get(&ids[0]).unwrap().title(), "Note 0");

    let mut changes = Changes::default();
    let redone = index.replay(&mut step.files, Op::Redo, &mut changes);
    assert_eq!(redone.unwrap(), SaveOutcome::Saved);
    assert_eq!(fs::read_to_string(&path).unwrap(), renamed);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_body_is_a_step_and_steps_join_into_one() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let path = root.join("Notes/n0.md");
    let mut first = Changes::default();
    let saved = index.save_content(&ids[0], "One\n".into(), false, &mut first);
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);
    let mut title = Changes::default();
    let titled = index.set_title(&ids[0], "Renamed".into(), &mut title);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
    let mut second = Changes::default();
    let saved = index.save_content(&ids[0], "Two\n".into(), false, &mut second);
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);
    let keeps_body =
        |files: &[FileStep]| matches!(&files[0].after, Place::Vault(state) if state.body.is_some());
    assert!(keeps_body(&first.files) && !keeps_body(&title.files));
    let after = fs::read_to_string(&path).unwrap();

    let files = FileStep::joined(first.files, title.files);
    let mut files = FileStep::joined(files, second.files);
    assert_eq!(files.len(), 1, "one file, one part");
    let mut changes = Changes::default();
    assert_eq!(
        index.replay(&mut files, Op::Undo, &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    let undone = format!(
        "---\nid: {}\ntitle: Note 0\ndocument: {{}}\n---\nBody 0\n",
        ids[0]
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), undone);
    assert_eq!(
        index.replay(&mut files, Op::Redo, &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), after);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_new_file_is_taken_away_and_made_again() {
    let (root, _) = thread_vault();
    let mut index = Index::build(&root);
    let mut step = Changes::default();
    let project = index.create_project("Move".into(), &mut step).unwrap();
    let mut titled = Changes::default();
    let renamed = index.set_title(&project, "Moving".into(), &mut titled);
    assert_eq!(renamed.unwrap(), SaveOutcome::Saved);
    let mut files = FileStep::joined(step.files, titled.files);
    let [
        FileStep {
            local,
            before: Place::Trash(None),
            ..
        },
    ] = &files[..]
    else {
        panic!("{files:?}");
    };
    let local_path = local.clone();
    let path = root.join(local);
    let made = fs::read_to_string(&path).unwrap();

    let mut changes = Changes::default();
    assert_eq!(
        index.replay(&mut files, Op::Undo, &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(changes.trashed, [project]);
    assert!(!path.exists() && index.get(&project).is_none());
    assert!(
        root.join(".trash").join(local_path).exists(),
        "in the trash"
    );

    let mut changes = Changes::default();
    assert_eq!(
        index.replay(&mut files, Op::Redo, &mut changes).unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), made);
    assert_eq!(index.get(&project).unwrap().title(), "Moving");

    // Changed since, it stays.
    let titled = index.set_title(&project, "Moved".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
    let undone = index.replay(&mut files, Op::Undo, &mut Changes::default());
    assert_eq!(undone.unwrap(), SaveOutcome::Conflict(project));
    assert!(path.exists());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn undoing_a_title_keeps_the_text_written_since() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut step = Changes::default();
    let titled = index.set_title(&ids[0], "Renamed".into(), &mut step);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
    let saved = index.save_content(&ids[0], "Typed\n".into(), false, &mut Changes::default());
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);

    let mut changes = Changes::default();
    assert_eq!(
        index
            .replay(&mut step.files, Op::Undo, &mut changes)
            .unwrap(),
        SaveOutcome::Saved
    );
    let doc = index.get(&ids[0]).unwrap();
    assert_eq!((doc.title(), doc.content()), ("Note 0", "Typed\n"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_file_written_twice_in_a_step_is_undone() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let first = new_thread(&mut index, "First", &ids[..2], &mut Changes::default());
    let second = new_thread(&mut index, "Second", &ids[..2], &mut Changes::default());

    // Both threads dissolve, so the member left of both is written twice.
    let mut deleted = Changes::default();
    let outcome = index.delete(&ids[0], &mut deleted);
    assert_eq!(outcome.unwrap(), SaveOutcome::Saved);
    assert!(threads_of(&index, &ids[1]).is_empty());
    let undone = index.replay(&mut deleted.files, Op::Undo, &mut Changes::default());
    assert_eq!(undone.unwrap(), SaveOutcome::Saved);
    assert_eq!(threads_of(&index, &ids[1]), [first, second]);
    assert_eq!(threads_of(&index, &ids[0]), [first, second]);
    let redone = index.replay(&mut deleted.files, Op::Redo, &mut Changes::default());
    assert_eq!(redone.unwrap(), SaveOutcome::Saved);
    assert!(threads_of(&index, &ids[1]).is_empty());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn an_entity_gone_since_is_the_conflict() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut titled = Changes::default();
    let set = index.set_title(&ids[0], "Renamed".into(), &mut titled);
    assert_eq!(set.unwrap(), SaveOutcome::Saved);
    index.trash(&ids[0], &mut Changes::default()).unwrap();

    let undone = index.replay(&mut titled.files, Op::Undo, &mut Changes::default());
    assert_eq!(undone.unwrap(), SaveOutcome::Conflict(ids[0]));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_file_without_an_id_comes_back_as_the_same_entity() {
    // No `id`, no frontmatter, frontmatter that doesn't parse.
    for text in [
        "---\ntitle: Plain\n---\nBody\n",
        "Text\n",
        "---\ntitle: [no\n---\nBody\n",
    ] {
        let root = std::env::temp_dir().join(format!("av-vault-test-{}", Id::generate()));
        let path = root.join("Notes/plain.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        let mut index = Index::build(&root);
        let id = index.all()[0].doc.id();
        let mut deleted = Changes::default();
        let outcome = index.delete(&id, &mut deleted);
        assert_eq!(outcome.unwrap(), SaveOutcome::Saved, "{text}");

        let undone = index.replay(&mut deleted.files, Op::Undo, &mut Changes::default());
        assert_eq!(undone.unwrap(), SaveOutcome::Saved, "{text}");
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert!(index.get(&id).is_some(), "{text}");
        let redone = index.replay(&mut deleted.files, Op::Redo, &mut Changes::default());
        assert_eq!(redone.unwrap(), SaveOutcome::Saved, "{text}");
        assert!(!path.exists() && index.get(&id).is_none(), "{text}");

        fs::remove_dir_all(&root).ok();
    }
}

#[test]
fn a_step_whose_file_changed_since_is_not_undone() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut step = Changes::default();
    let titled = index.set_title(&ids[0], "Renamed".into(), &mut step);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
    let mut later = Changes::default();
    let titled = index.set_title(&ids[0], "Again".into(), &mut later);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);

    // Written since by the app.
    let mut changes = Changes::default();
    let undone = index.replay(&mut step.files, Op::Undo, &mut changes);
    assert_eq!(undone.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert!(changes.saved.is_empty());
    assert_eq!(index.get(&ids[0]).unwrap().title(), "Again");

    // Written since by another program.
    let path = edit_outside(&root, 0, &ids[0], "Theirs, longer\n");
    let theirs = fs::read_to_string(&path).unwrap();
    let undone = index.replay(&mut later.files, Op::Undo, &mut changes);
    assert_eq!(undone.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert_eq!(fs::read_to_string(&path).unwrap(), theirs);

    // Redone only from what it held before.
    let redone = index.replay(&mut step.files, Op::Redo, &mut changes);
    assert_eq!(redone.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert!(changes.saved.is_empty());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn the_stamp_comes_from_the_read_and_follows_each_write() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let path = root.join("Notes/n0.md");
    let stamp = |index: &Index| index.entry(&ids[0]).unwrap().doc.stamp();
    assert_eq!(stamp(&index), FileStamp::read(&path).unwrap());

    let saved = index.save_content(
        &ids[0],
        "A longer body than before\n".into(),
        false,
        &mut Changes::default(),
    );
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);
    assert_eq!(stamp(&index), FileStamp::read(&path).unwrap());
    assert!(!index.entry(&ids[0]).unwrap().changed_on_disk().unwrap());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_same_size_change_is_found_by_its_time() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let path = root.join("Notes/n0.md");
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("Body 0", "Body X");
    fs::write(&path, &text).unwrap();
    let file = fs::File::options().write(true).open(&path).unwrap();
    file.set_modified(before + std::time::Duration::from_secs(2))
        .unwrap();
    drop(file);

    let saved = index.save_content(&ids[0], "Mine\n".into(), false, &mut Changes::default());
    assert_eq!(saved.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert_eq!(fs::read_to_string(&path).unwrap(), text);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_deleted_file_is_a_conflict_until_overwritten() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let path = root.join("Notes/n0.md");
    fs::remove_file(&path).unwrap();

    let saved = index.save_content(&ids[0], "Mine\n".into(), false, &mut Changes::default());
    assert_eq!(saved.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert!(!path.exists());

    let saved = index.save_content(&ids[0], "Mine\n".into(), true, &mut Changes::default());
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);
    assert!(fs::read_to_string(&path).unwrap().ends_with("Mine\n"));

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_changed_member_stops_the_whole_operation() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let mut changes = Changes::default();
    let thread = new_thread(&mut index, "T", &ids[0..2], &mut changes);
    let first = fs::read_to_string(root.join("Notes/n0.md")).unwrap();
    edit_outside(&root, 1, &ids[1], "Theirs\n");

    // Leaving would dissolve the thread and rewrite ids[1].
    let mut changes = Changes::default();
    let left = index.leave_thread(&ids[0], &thread, &mut changes);
    assert_eq!(left.unwrap(), SaveOutcome::Conflict(ids[1]));
    assert!(changes.saved.is_empty() && changes.trashed.is_empty());
    assert_eq!(fs::read_to_string(root.join("Notes/n0.md")).unwrap(), first);
    assert_eq!(threads_of(&index, &ids[0]), [thread]);
    assert!(index.get(&thread).is_some());

    // Nor is a thread created around it.
    let files = fs::read_dir(root.join("Notes")).unwrap().count();
    let mut changes = Changes::default();
    let created = index.create_thread("U".into(), &ids[1..3], &mut changes);
    assert_eq!(created.unwrap(), SaveOutcome::Conflict(ids[1]));
    assert!(changes.created.is_none() && changes.saved.is_empty());
    assert_eq!(fs::read_dir(root.join("Notes")).unwrap().count(), files);

    // Deleting the other member would dissolve it as well.
    let deleted = index.delete(&ids[0], &mut changes);
    assert_eq!(deleted.unwrap(), SaveOutcome::Conflict(ids[1]));
    assert!(root.join("Notes/n0.md").exists());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn reloading_takes_in_what_the_file_holds() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    edit_outside(&root, 0, &ids[0], "Theirs\n");

    assert_eq!(index.reload(&ids[0]).unwrap(), Some(ids[0]));
    let doc = index.get(&ids[0]).unwrap();
    assert_eq!((doc.title(), doc.content()), ("Outside", "Theirs\n"));
    let saved = index.save_content(&ids[0], "Mine\n".into(), false, &mut Changes::default());
    assert_eq!(
        saved.unwrap(),
        SaveOutcome::Saved,
        "no conflict once reloaded"
    );

    // A file whose `id` was changed comes back under the new one.
    let other = Id::generate();
    edit_outside(&root, 1, &other, "Renumbered\n");
    assert_eq!(index.reload(&ids[1]).unwrap(), Some(other));
    assert!(index.get(&ids[1]).is_none());
    assert_eq!(index.get(&other).unwrap().content(), "Renumbered\n");

    // A deleted file leaves the index.
    fs::remove_file(root.join("Notes/n2.md")).unwrap();
    assert_eq!(index.reload(&ids[2]).unwrap(), None);
    assert!(index.get(&ids[2]).is_none());
    assert_eq!(index.all().len(), 2);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn refreshing_takes_in_a_change_to_a_file_only_read() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    assert_eq!(index.refresh(&ids[0]).unwrap(), Refresh::Unchanged);

    edit_outside(&root, 0, &ids[0], "Theirs\n");
    assert_eq!(
        index.refresh(&ids[0]).unwrap(),
        Refresh::TakenIn(Some(ids[0]))
    );
    assert_eq!(index.get(&ids[0]).unwrap().content(), "Theirs\n");
    assert_eq!(index.refresh(&ids[0]).unwrap(), Refresh::Unchanged);

    fs::remove_file(root.join("Notes/n1.md")).unwrap();
    assert_eq!(index.refresh(&ids[1]).unwrap(), Refresh::TakenIn(None));
    assert!(index.get(&ids[1]).is_none());

    fs::remove_dir_all(&root).ok();
}

#[test]
fn refreshing_leaves_a_change_over_the_apps_own_save_alone() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let saved = index.save_content(&ids[0], "Mine\n".into(), false, &mut Changes::default());
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);
    assert_eq!(index.refresh(&ids[0]).unwrap(), Refresh::Unchanged);

    // Another program writes its older copy over the app's save.
    edit_outside(&root, 0, &ids[0], "Theirs\n");
    assert_eq!(index.refresh(&ids[0]).unwrap(), Refresh::Conflict);
    assert_eq!(
        index.get(&ids[0]).unwrap().content(),
        "Mine\n",
        "kept for Use ours"
    );

    // Use ours writes it back, although memory didn't change; then the
    // file is the app's again.
    let saved = index.save_content(&ids[0], "Mine\n".into(), true, &mut Changes::default());
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);
    let text = fs::read_to_string(root.join("Notes/n0.md")).unwrap();
    assert!(text.ends_with("---\nMine\n"), "{text}");
    let mut changes = Changes::default();
    let edited = index.edit(&ids[0], &mut changes, |doc| {
        doc.set_content("Mine 2\n".into())
    });
    assert_eq!(edited.unwrap(), SaveOutcome::Saved);

    // After a reload, the app has only read the file again.
    index.reload(&ids[0]).unwrap();
    edit_outside(&root, 0, &ids[0], "Theirs again\n");
    assert_eq!(
        index.refresh(&ids[0]).unwrap(),
        Refresh::TakenIn(Some(ids[0]))
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn the_forged_recurring_payments_hold_their_transactions() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/forged");
    let index = Index::build(&root);
    let transactions: Vec<&Transaction> = index
        .all()
        .iter()
        .filter_map(|e| e.doc.roles().statement.as_ref())
        .flat_map(|t| &t.transactions)
        .collect();
    let mut found: Vec<(String, usize, bool)> = index
        .all()
        .iter()
        .filter(|e| e.doc.kind() == DocumentType::RecurringPayment)
        .map(|e| {
            let id = Some(e.doc.id());
            let paid = transactions
                .iter()
                .filter(|b| b.recurring_payment == id)
                .count();
            let linked = e.doc.linked_to().is_some();
            (e.doc.title().to_string(), paid, linked)
        })
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            ("Mortgage".to_string(), 0, true),
            ("Rent, Unit 4B".to_string(), 3, false),
            ("Salary, Cogsworth".to_string(), 0, true),
            ("Salary, Springfield Power Station".to_string(), 4, false),
            ("Springfield Power & Light".to_string(), 4, false),
        ]
    );
}

/// The sample vault under `data/forged` follows vault.md; every
/// file must load cleanly and derive its intended type.
#[test]
fn forged_vault_loads_with_multi_roles() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/forged");
    let index = Index::build(&root);

    assert!(index.failed.is_empty(), "failed: {:?}", index.failed);
    let broken: Vec<_> = index.broken().iter().map(|e| &e.path).collect();
    assert!(broken.is_empty(), "broken: {broken:?}");

    for entry in index.all() {
        let name = entry.path.file_name().unwrap().to_string_lossy();
        let type_id = name.rsplit('.').nth(1).unwrap();
        assert_eq!(entry.doc.kind().type_id(), type_id, "{name}");
        assert!(entry.doc.source().is_some(), "{name} has no source");
    }

    let thread = index
        .all()
        .iter()
        .find(|e| e.doc.kind() == DocumentType::Thread && e.doc.title() == "ID Card Renewal")
        .expect("forged thread");
    let members = index.thread_members(&thread.doc.id());
    assert_eq!(members.len(), 7);
    assert!(
        members
            .windows(2)
            .all(|w| w[0].doc.date() <= w[1].doc.date())
    );
    let kinds: Vec<_> = members.iter().map(|e| e.doc.kind().type_id()).collect();
    assert_eq!(kinds, ["msg", "tsk", "eml", "eml", "eml", "evt", "msg"]);

    let conversation = index
        .all()
        .iter()
        .filter(|e| e.doc.title() == "Preparing the ID card appointment")
        .find_map(|e| e.doc.roles().conversation.as_ref())
        .expect("forged conversation");
    let active = conversation.active.expect("active leaf");
    assert!(conversation.messages.iter().any(|m| m.id == Some(active)));
    let roots = conversation
        .messages
        .iter()
        .filter(|m| m.parent.is_none())
        .count();
    assert_eq!(roots, 1);

    // Links are top-level, and point into the vault.
    let links: Vec<_> = index
        .all()
        .iter()
        .filter_map(|e| e.doc.linked_to())
        .collect();
    assert!(links.len() >= 20, "{} links", links.len());
    assert!(links.iter().all(|to| index.get(to).is_some()));
}

#[test]
fn an_edit_writes_each_entity_it_fits_and_refuses_the_others() {
    use crate::change::TaskChange;
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let new_task = |index: &mut Index, title: &str| {
        let new = NewEntity {
            roles: roles("task: {status: todo}"),
            title: title.into(),
            ..Default::default()
        };
        create_new(index, new).0
    };
    let task = new_task(&mut index, "Pay the bill");
    let read_only = new_task(&mut index, "Read only");
    index.documents[index.by_id[&read_only]].readonly = true;
    let unknown = Id::generate();

    let now = DateTime::from_iso8601("2026-10-12T08:30:00Z").unwrap();
    let done = Change::Task(TaskChange::Status("done".into()));
    let mut changes = Changes::default();
    let edited = index.edit_entities(
        &[ids[0], task, read_only, unknown],
        &done,
        now,
        &mut changes,
    );

    let ids_of: Vec<Id> = edited.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids_of, [ids[0], task, read_only, unknown], "in their order");
    assert!(
        matches!(edited[0].1, Err(Error::InvalidArgument { .. })),
        "not a task"
    );
    assert_eq!(edited[1].1.as_ref().unwrap(), &SaveOutcome::Saved);
    assert!(matches!(edited[2].1, Err(Error::NotWritable { .. })));
    assert!(matches!(edited[3].1, Err(Error::NotFound)));
    assert_eq!(changes.saved, [task]);

    let text = fs::read_to_string(&index.entry(&task).unwrap().path).unwrap();
    assert!(
        text.contains("status: done") && text.contains("done: 2026-10-12T08:30:00Z"),
        "{text}"
    );
    let doc = Index::build(&root);
    assert_eq!(
        doc.get(&task).unwrap().roles().task.as_ref().unwrap().done,
        Some(now)
    );

    // Done again writes nothing; another status removes the date.
    let mut changes = Changes::default();
    let again = index.edit_entities(&[task], &done, now, &mut changes);
    assert_eq!(again[0].1.as_ref().unwrap(), &SaveOutcome::Unchanged);
    let todo = Change::Task(TaskChange::Status("todo".into()));
    index.edit_entities(&[task], &todo, now, &mut changes);
    let text = fs::read_to_string(&index.entry(&task).unwrap().path).unwrap();
    assert!(
        text.contains("status: todo") && !text.contains("done"),
        "{text}"
    );

    fs::remove_dir_all(&root).ok();
}
