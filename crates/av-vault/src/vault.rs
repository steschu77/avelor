//! Vault scanning and the in-memory index.
//!
//! The vault is a folder tree; entities are markdown documents with a
//! YAML frontmatter and standalone YAML records, each with an `id` and
//! any number of role blocks.
//! Membership (project → documents, kind → entities) is always computed
//! here, never stored on disk. This structure is rebuilt by a full scan
//! (fast at personal-vault scale).

use std::collections::HashMap;
use std::fs::{self, DirEntry};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::document::{Document, FileFormat};
use av_utils::error::Result;
use av_utils::id::Id;
use av_utils::text::{LineEnding, normalize};

// ----------------------------------------------------------------------------
#[derive(Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub local: PathBuf,
    pub readonly: bool,
    pub bom: String,
    pub crlf: LineEnding,
    pub doc: Document,
}

impl Entry {
    /// An entry for an entity that has no file yet.
    pub fn unsaved(doc: Document) -> Self {
        Self {
            path: PathBuf::new(),
            local: PathBuf::new(),
            readonly: false,
            bom: String::new(),
            crlf: LineEnding::default(),
            doc,
        }
    }

    /// Whether this entry has no file yet.
    pub fn is_unsaved(&self) -> bool {
        self.path.as_os_str().is_empty()
    }

    /// Whether the WYSIWYG editor may edit this entry's content: a markdown
    /// file that is writable and whose frontmatter parsed. Broken documents
    /// and YAML records wait for the source editor.
    pub fn is_editable(&self) -> bool {
        !self.readonly && !self.doc.is_broken() && self.doc.format() == FileFormat::Markdown
    }

    /// Whether this entry's frontmatter may be rewritten, e.g. for a thread
    /// membership change: it has a file, the file is writable, and its
    /// frontmatter parsed. Unlike [`Entry::is_editable`], YAML files count.
    pub fn is_writable(&self) -> bool {
        !self.readonly && !self.doc.is_broken() && !self.is_unsaved()
    }

    /// Whether this entry's file may be moved to `.trash`: it has a file,
    /// and the file isn't read-only. Broken frontmatter doesn't matter,
    /// since the file isn't rewritten.
    pub fn is_deletable(&self) -> bool {
        !self.readonly && !self.is_unsaved()
    }
}

// ----------------------------------------------------------------------------
/// One step of a scan in progress, reported as entries are loaded. Carries
/// only owned data (no borrows into an `Index`) so it can be produced on a
/// worker thread and handed across to whoever owns the `Index`.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ScanEvent {
    Start {
        name: String,
        path: String,
        total: usize,
    },
    Entry {
        index: usize,
        total: usize,
        entry: Entry,
    },
    Failed {
        path: PathBuf,
        error: String,
    },
    Done,
}

// ----------------------------------------------------------------------------
pub enum ScanControl {
    Continue,
    Stop,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Default)]
pub struct Index {
    pub root: PathBuf,
    documents: Vec<Entry>,
    by_id: HashMap<Id, usize>,
    /// Documents which could not be loaded due to errors, show their errors here.
    failed: Vec<(PathBuf, String)>,
}

// ----------------------------------------------------------------------------
impl Index {
    /// Convenience wrapper around [`scan`] for callers that don't need
    /// progress: scans `root` and builds a complete `Index` synchronously.
    pub fn scan(root: &Path) -> Index {
        let mut index = Index::default();
        index.begin(root);
        self::scan(root, |event| {
            index.apply(event);
            ScanControl::Continue
        });
        index
    }

    /// Reset the index and set its root, ready to receive scan events via
    /// [`Index::apply`]. Call this before starting a new scan.
    pub fn begin(&mut self, root: &Path) {
        self.root = root.to_path_buf();
        self.documents.clear();
        self.by_id.clear();
        self.failed.clear();
    }

    /// Apply one `ScanEvent` produced by [`scan`]: the id of the entry it
    /// added. An entry whose id the index already has is skipped: the app
    /// made or restored that entity while the scan walked the vault, so its
    /// own entry is the newer one. `Start`/`Done` are no-ops here; callers
    /// use them to drive progress reporting.
    pub fn apply(&mut self, event: ScanEvent) -> Option<Id> {
        match event {
            ScanEvent::Entry { entry, .. } => {
                let id = entry.doc.id();
                if self.by_id.contains_key(&id) {
                    log::warn!("{:?}: {id} is in the vault already", entry.local);
                    return None;
                }
                self.insert(entry);
                Some(id)
            }
            ScanEvent::Failed { path, error } => {
                self.insert_failed(path, error);
                None
            }
            ScanEvent::Start { .. } | ScanEvent::Done => None,
        }
    }

    fn insert(&mut self, entry: Entry) {
        self.by_id.insert(entry.doc.id(), self.documents.len());
        self.documents.push(entry);
    }

    /// Add an entry that didn't come from a scan, e.g. an entity created in
    /// the app ([`Entry::unsaved`]).
    pub fn add(&mut self, entry: Entry) -> &Entry {
        self.insert(entry);
        &self.documents[self.documents.len() - 1]
    }

    fn insert_failed(&mut self, path: PathBuf, error: String) {
        self.failed.push((path, error));
    }

    // ------------------------------------------------------------------------
    pub fn get(&self, id: &Id) -> Option<&Document> {
        self.entry(id).map(|e| &e.doc)
    }

    // ------------------------------------------------------------------------
    pub fn entry(&self, id: &Id) -> Option<&Entry> {
        self.by_id.get(id).and_then(|&i| self.documents.get(i))
    }

    // ------------------------------------------------------------------------
    pub fn all(&self) -> &[Entry] {
        &self.documents
    }

    // ------------------------------------------------------------------------
    pub fn broken(&self) -> Vec<&Entry> {
        self.documents
            .iter()
            .filter(|e| e.doc.is_broken())
            .collect::<Vec<_>>()
    }

    // ------------------------------------------------------------------------
    pub fn members_of(&self, project_id: &Id) -> Vec<&Entry> {
        self.documents
            .iter()
            .filter(|e| e.doc.projects().iter().any(|p| p == project_id))
            .collect()
    }

    // ------------------------------------------------------------------------
    /// The members of `thread_id` -- entities whose `message.threads` lists
    /// it -- in thread order: by `date`, ties broken by `id` (UUIDv7).
    pub fn thread_members(&self, thread_id: &Id) -> Vec<&Entry> {
        let mut members: Vec<&Entry> = self
            .documents
            .iter()
            .filter(|e| {
                e.doc
                    .roles()
                    .message
                    .as_ref()
                    .is_some_and(|m| m.threads.contains(thread_id))
            })
            .collect();
        sort_by_date(&mut members);
        members
    }
}

// ----------------------------------------------------------------------------
/// Sort `entries` by `date`, ties broken by `id` (UUIDv7, so creation
/// order), the undated ones first: thread order.
pub(crate) fn sort_by_date(entries: &mut [&Entry]) {
    entries.sort_by_key(|e| (e.doc.date(), e.doc.id()));
}

// ----------------------------------------------------------------------------
/// Scan `root` for eligible files (`*.md` / `*.yaml`) and load them,
/// reporting progress through `on_event`. Touches no `Index` -- every event
/// carries owned, `Send` data, so this runs on a worker thread while the UI
/// thread owns the `Index` and applies the events as they arrive via
/// [`Index::apply`] (the app's `vault_scan`).
///
/// Phases: collect eligible files, emit `Start`; load them one by one,
/// emitting `Entry`/`Failed` per file; emit `Done`. `on_event` answering
/// `Stop` ends the scan there.
pub fn scan(root: &Path, mut on_event: impl FnMut(ScanEvent) -> ScanControl) {
    let mut files = Vec::new();
    walk(
        root,
        &PathBuf::new(),
        &mut |path, local, readonly, format| files.push((path, local, readonly, format)),
    );

    let total = files.len();
    let start = ScanEvent::Start {
        name: root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        path: root.to_string_lossy().to_string(),
        total,
    };
    if matches!(on_event(start), ScanControl::Stop) {
        return;
    }
    for (i, (path, local, readonly, format)) in files.into_iter().enumerate() {
        let event = match load(path.clone(), local, readonly, format) {
            Ok(entry) => ScanEvent::Entry {
                index: i + 1,
                total,
                entry,
            },
            Err(error) => ScanEvent::Failed {
                path,
                error: error.to_string(),
            },
        };
        if matches!(on_event(event), ScanControl::Stop) {
            return;
        }
    }
    on_event(ScanEvent::Done);
}

// ----------------------------------------------------------------------------
fn load(path: PathBuf, local: PathBuf, readonly: bool, doc_type: FileFormat) -> Result<Entry> {
    let mut file = fs::File::open(&path)?;
    let mut t = String::new();
    file.read_to_string(&mut t)?;
    let (bom, crlf, text) = normalize(t);

    let doc = Document::load(text, doc_type);
    if let Some(e) = doc.error() {
        log::error!("{path:?}: {e}");
    }

    Ok(Entry {
        path,
        local,
        readonly,
        bom,
        crlf,
        doc,
    })
}

// ----------------------------------------------------------------------------
/// The format of a vault file, by its extension: `.md`, `.yaml` or `.yml`;
/// `None` for any other file, which isn't an entity.
fn file_format(path: &Path) -> Option<FileFormat> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "md" => Some(FileFormat::Markdown),
        "yaml" | "yml" => Some(FileFormat::YAML),
        _ => None,
    }
}

// ----------------------------------------------------------------------------
fn visit(
    root: &Path,
    local: &Path,
    file: &DirEntry,
    cb: &mut impl FnMut(PathBuf, PathBuf, bool, FileFormat),
) {
    let path = root.join(local);
    let Some(doc_type) = file_format(&path) else {
        return;
    };

    let metadata = file.metadata();
    let readonly = metadata
        .map(|m| m.permissions().readonly())
        .unwrap_or(false);

    cb(path, local.to_path_buf(), readonly, doc_type);
}

// ----------------------------------------------------------------------------
/// Recursive walk, skipping dot-directories (`.assistant`, `.obsidian`, …).
fn walk(root: &Path, folders: &Path, cb: &mut impl FnMut(PathBuf, PathBuf, bool, FileFormat)) {
    let path = root.join(folders);
    let Ok(entries) = fs::read_dir(&path) else {
        return;
    };
    // Sorted: the index's order is the same on every file system.
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }

        let local = folders.join(entry.file_name());

        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(root, &local, cb),
            Ok(t) if t.is_file() => visit(root, &local, &entry, cb),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
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
    fn scans_and_indexes_a_vault() {
        let (root, proj_id, doc_id, task_id, cont_id, rec_id, obs_id) = fixture_vault();

        let index = Index::scan(&root);

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
    fn a_scanned_entry_the_index_already_has_is_skipped() {
        let (root, proj_id, doc_id, ..) = fixture_vault();
        let mut index = Index::default();
        index.begin(&root);
        // Written by the app while the scan walked the vault.
        let text = format!("---\nid: {doc_id}\ntitle: Plan, as the app wrote it\n---\n");
        index.add(Entry::unsaved(Document::load(text, FileFormat::Markdown)));

        let mut added = Vec::new();
        scan(&root, |event| {
            added.extend(index.apply(event));
            ScanControl::Continue
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
    fn a_scan_ends_where_it_is_told_to_stop() {
        let (root, ..) = fixture_vault();
        let mut events = 0;
        scan(&root, |event| {
            events += 1;
            match event {
                ScanEvent::Entry { .. } => ScanControl::Stop,
                _ => ScanControl::Continue,
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
        let index = Index::scan(&root);

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
}
