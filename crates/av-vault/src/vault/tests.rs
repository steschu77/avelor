use super::*;
use crate::envelope::DocumentType;
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

#[test]
fn remove_keeps_the_other_entries_findable() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let first = index.all()[0].doc.id();

    let removed = index.remove(&first).unwrap();
    assert_eq!(removed.doc.id(), first);
    assert!(index.get(&first).is_none());
    assert_eq!(index.all().len(), 2);
    for id in ids.iter().filter(|id| **id != first) {
        assert_eq!(index.get(id).unwrap().id(), *id);
    }

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

    let mut changes = Changes::default();
    let titled = index.set_title(&ids[0], "Mine".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert!(changes.saved.is_empty());
    let doc = index.get(&ids[0]).unwrap();
    assert_eq!(doc.title(), "Note 0", "memory still matches the file");
    assert!(!doc.is_dirty());
    assert_eq!(fs::read_to_string(&path).unwrap(), theirs);

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
        index.replay(&changes.files, Op::Undo, &mut undone).unwrap(),
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
    let [FileStep { id, before, after }] = &step.files[..] else {
        panic!("{:?}", step.files);
    };
    assert_eq!(*id, ids[0]);
    assert!(
        before.body.is_none() && after.body.is_none(),
        "a title keeps no body"
    );

    let mut changes = Changes::default();
    let undone = index.replay(&step.files, Op::Undo, &mut changes);
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
    let redone = index.replay(&step.files, Op::Redo, &mut changes);
    assert_eq!(redone.unwrap(), SaveOutcome::Saved);
    assert_eq!(fs::read_to_string(&path).unwrap(), renamed);

    fs::remove_dir_all(&root).ok();
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
    let undone = index.replay(&step.files, Op::Undo, &mut changes);
    assert_eq!(undone.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert!(changes.saved.is_empty());
    assert_eq!(index.get(&ids[0]).unwrap().title(), "Again");

    // Written since by another program.
    let path = edit_outside(&root, 0, &ids[0], "Theirs, longer\n");
    let theirs = fs::read_to_string(&path).unwrap();
    let undone = index.replay(&later.files, Op::Undo, &mut changes);
    assert_eq!(undone.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert_eq!(fs::read_to_string(&path).unwrap(), theirs);

    // Redone only from what it held before.
    let redone = index.replay(&step.files, Op::Redo, &mut changes);
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

    let mut changes = Changes::default();
    let titled = index.set_title(&ids[0], "A longer title than before".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Saved);
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

    let mut changes = Changes::default();
    let titled = index.set_title(&ids[0], "Mine".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert_eq!(fs::read_to_string(&path).unwrap(), text);

    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_deleted_file_is_a_conflict() {
    let (root, ids) = thread_vault();
    let mut index = Index::build(&root);
    let path = root.join("Notes/n0.md");
    fs::remove_file(&path).unwrap();

    let mut changes = Changes::default();
    let titled = index.set_title(&ids[0], "Mine".into(), &mut changes);
    assert_eq!(titled.unwrap(), SaveOutcome::Conflict(ids[0]));
    assert!(!path.exists());

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
    let mut changes = Changes::default();
    let titled = index.set_title(&ids[0], "Mine".into(), &mut changes);
    assert_eq!(
        titled.unwrap(),
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
