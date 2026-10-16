use super::trash::untrashed;
use super::*;
use crate::envelope::DocumentType;
use crate::envelope::{Roles, Source};
use crate::schema::layout;
use av_utils::datetime::DateTime;
use av_utils::id::Id;

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
            "---\nid: {doc_id}\ndocument:\n  genre: note\ntitle: Plan\nprojects: [{proj_id}]\ntags: [home]\n---\nBody\n"
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
        plan.roles().document.as_ref().unwrap().genre,
        Some(crate::shared::Genre::Note)
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
    index_vault(&root, |event| {
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
    index_vault(&root, |event| {
        events += 1;
        match event {
            IndexEvent::Entry { .. } => IndexControl::Stop,
            _ => IndexControl::Continue,
        }
    });
    assert_eq!(events, 2, "the start and the first file");

    fs::remove_dir_all(&root).ok();
}

#[test]
fn added_unsaved_entity_is_indexed_but_not_saved() {
    use crate::envelope::{Empty, Roles, Thread};

    let mut index = Index::default();
    let roles = Roles {
        thread: Some(Thread {}),
        ..Default::default()
    };
    let doc = Document::create(
        roles,
        Source::Created(Empty {}),
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
    use crate::envelope::{Empty, Roles};

    let mut index = Index::default();
    let ids: Vec<Id> = (0..3)
        .map(|i| {
            let doc = Document::create(
                Roles::default(),
                Source::Created(Empty {}),
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

    let saved = index.save_content(&ids[0], "Mine\n".into(), false);
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
    let saved = index.save_content(&ids[0], "Mine\n".into(), true);
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("title: Note 0") && text.ends_with("Mine\n"),
        "{text}"
    );
    let saved = index.save_content(&ids[0], "Mine again\n".into(), false);
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);
    let saved = index.save_content(&ids[0], "Mine again\n".into(), false);
    assert_eq!(saved.unwrap(), SaveOutcome::Unchanged);

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

    fs::remove_dir_all(&root).ok();
}

#[test]
fn the_stamp_comes_from_the_read_and_follows_each_write() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let path = root.join("Notes/n0.md");
    let stamp = |index: &Index| index.entry(&ids[0]).unwrap().doc.stamp();
    assert_eq!(stamp(&index), FileStamp::read(&path).unwrap());

    let saved = index.save_content(&ids[0], "A longer body than before\n".into(), false);
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

    let saved = index.save_content(&ids[0], "Mine\n".into(), false);
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

    let saved = index.save_content(&ids[0], "Mine\n".into(), false);
    assert_eq!(saved.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert!(!path.exists());

    let saved = index.save_content(&ids[0], "Mine\n".into(), true);
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
    let saved = index.save_content(&ids[0], "Mine\n".into(), false);
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
    let saved = index.save_content(&ids[0], "Mine\n".into(), false);
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

    // Use ours writes it back, although memory didn't change.
    let saved = index.save_content(&ids[0], "Mine\n".into(), true);
    assert_eq!(saved.unwrap(), SaveOutcome::Saved);
    let text = fs::read_to_string(root.join("Notes/n0.md")).unwrap();
    assert!(text.ends_with("---\nMine\n"), "{text}");

    // After a reload, the app has only read the file again.
    index.reload(&ids[0]).unwrap();
    edit_outside(&root, 0, &ids[0], "Theirs again\n");
    assert_eq!(
        index.refresh(&ids[0]).unwrap(),
        Refresh::TakenIn(Some(ids[0]))
    );

    fs::remove_dir_all(&root).ok();
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
