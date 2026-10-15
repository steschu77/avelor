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
