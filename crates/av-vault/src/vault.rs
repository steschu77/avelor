//! Vault scanning and the in-memory index.
//!
//! The vault is a folder tree; entities are markdown documents with a
//! YAML frontmatter and standalone YAML records, each with an `id` and
//! any number of role blocks.
//! Membership (project → documents, kind → entities) is always computed
//! here, never stored on disk. This structure is rebuilt by a full scan
//! (fast at personal-vault scale).

use std::collections::HashMap;
use std::fs::Metadata;
use std::fs::{self, DirEntry};
use std::io::ErrorKind;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::document::{Document, FileFormat};
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::fs::write_atomic;
use av_utils::id::Id;
use av_utils::text::renormalize;
use av_utils::text::{LineEnding, normalize};

// ----------------------------------------------------------------------------
/// What a file looked like when the app last read or wrote it. A file whose
/// stamp differs was changed outside the app meanwhile, and isn't
/// overwritten (saving.md#the-stamp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    modified: SystemTime,
    len: u64,
}

impl FileStamp {
    /// The stamp of a file's `metadata`.
    pub fn of(metadata: &Metadata) -> Self {
        Self {
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            len: metadata.len(),
        }
    }

    /// The stamp of the file at `path` now; `None` if there is no file.
    pub fn read(path: &Path) -> Result<Option<Self>> {
        match fs::metadata(path) {
            Ok(metadata) => Ok(Some(Self::of(&metadata))),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
            Err(err) => Err(Error::FileIo { err }),
        }
    }
}

// ----------------------------------------------------------------------------
/// What a write did. A conflict is an expected outcome, not an error: the
/// entity's file changed outside the app since it was last read or written,
/// so nothing was written, and the user decides -- reload the file
/// ([`Index::reload`]).
#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// At least one file was written.
    Saved,
    /// Nothing had changed, so nothing was written.
    Unchanged,
    /// This entity's file changed outside the app; nothing was written.
    Conflict(Id),
}

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

    /// Whether the file changed (or went away) since the app last read or
    /// wrote it.
    pub fn changed_on_disk(&self) -> Result<bool> {
        Ok(FileStamp::read(&self.path)? != self.doc.stamp())
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
/// What a structural operation did, so the caller can push exactly the
/// changed entities. Filled in as the operation goes, so it is accurate even
/// when the operation fails part-way.
#[derive(Debug, Default)]
pub struct Changes {
    /// Entities whose file was rewritten.
    pub saved: Vec<Id>,
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

    /// Drop `id`'s entry from the index and return it. Only the in-memory
    /// index changes: a file on disk stays, and comes back on the next scan.
    pub fn remove(&mut self, id: &Id) -> Option<Entry> {
        let i = self.by_id.remove(id)?;
        let entry = self.documents.swap_remove(i);
        // swap_remove moved the former last entry into slot `i`.
        if let Some(moved) = self.documents.get(i) {
            self.by_id.insert(moved.doc.id(), i);
        }
        Some(entry)
    }

    fn insert_failed(&mut self, path: PathBuf, error: String) {
        self.failed.push((path, error));
    }

    /// Write `id`'s current in-memory content/envelope to disk (BOM/CRLF
    /// preserved) through a temporary file, then record what the file holds
    /// now (`Document::written`), so `Document::is_dirty` reflects "clean"
    /// again afterward. A file changed outside the app since the
    /// last read or write is a conflict and isn't touched.
    fn save(&mut self, id: &Id, _overwrite: bool) -> Result<SaveOutcome> {
        let Some(&i) = self.by_id.get(id) else {
            return Err(Error::NotFound);
        };
        let entry = &self.documents[i];
        if entry.changed_on_disk()? {
            return Ok(SaveOutcome::Conflict(*id));
        }
        let text = renormalize(&entry.bom, entry.crlf, entry.doc.save());
        write_atomic(&entry.path, text.as_bytes())?;
        // Without a stamp the next save is a conflict: the safe side.
        let stamp = FileStamp::read(&entry.path).ok().flatten();

        self.documents[i].doc.written(stamp);
        Ok(SaveOutcome::Saved)
    }

    /// Read `id`'s file again and take in whatever it holds now: the
    /// user's answer to a conflict. Returns the id the file has now (a
    /// different one if its `id` was changed by hand), or `None` if the
    /// file is gone, in which case the entry leaves the index.
    pub fn reload(&mut self, id: &Id) -> Result<Option<Id>> {
        let Some(&i) = self.by_id.get(id) else {
            return Err(Error::NotFound);
        };
        let entry = &self.documents[i];
        let readonly = match fs::metadata(&entry.path) {
            Ok(metadata) => metadata.permissions().readonly(),
            Err(err) if err.kind() == ErrorKind::NotFound => {
                self.remove(id);
                return Ok(None);
            }
            Err(err) => return Err(Error::FileIo { err }),
        };
        let mut fresh = load(
            entry.path.clone(),
            entry.local.clone(),
            readonly,
            entry.doc.format(),
        )?;
        if !fresh.doc.adopted() {
            fresh.doc.keep_id(*id); // a file without `id` keeps its entity
        }
        let fresh_id = fresh.doc.id();
        self.by_id.remove(id);
        self.by_id.insert(fresh_id, i);
        self.documents[i] = fresh;
        Ok(Some(fresh_id))
    }

    // ------------------------------------------------------------------------
    // Structural operations: each validates everything it will touch first
    // -- including that no file to rewrite changed outside the app, so a
    // conflict writes nothing -- then changes and saves entity by entity,
    // recording each file it wrote in `changes`. A failed save restores that
    // entity's in-memory state, so the index always matches the disk.

    /// Set the title of `id`, which may be any entity. An empty title removes
    /// it ([`Document::set_title`]). Only the file's content changes; the file
    /// is not renamed to match.
    pub fn set_title(
        &mut self,
        id: &Id,
        title: String,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_title(title))
    }

    /// Apply `change` to `id`'s document and save it, recording a write in
    /// `changes`.
    fn update(
        &mut self,
        id: &Id,
        changes: &mut Changes,
        change: impl FnOnce(&mut Document),
    ) -> Result<SaveOutcome> {
        let outcome = self.write_change(id, change)?;
        if outcome == SaveOutcome::Saved {
            changes.saved.push(*id);
        }
        Ok(outcome)
    }

    /// Apply `change` to `id`'s document and save it if that changed
    /// anything. On a conflict or a failed save the document's envelope and
    /// body -- all it holds -- are put back as they were, so memory keeps
    /// matching the file.
    fn write_change(&mut self, id: &Id, change: impl FnOnce(&mut Document)) -> Result<SaveOutcome> {
        let Some(&i) = self.by_id.get(id) else {
            return Err(Error::NotFound);
        };
        let doc = &mut self.documents[i].doc;
        let before = doc.snapshot();
        change(doc);
        if !doc.is_dirty() {
            return Ok(SaveOutcome::Unchanged);
        }
        let saved = self.save(id, false);
        if !matches!(saved, Ok(SaveOutcome::Saved)) {
            self.documents[i].doc.restore(before);
        }
        saved
    }

    fn check_writable(&self, id: &Id) -> Result<()> {
        match self.entry(id) {
            Some(e) if e.is_writable() => Ok(()),
            Some(e) => Err(Error::NotWritable {
                path: e.local.clone(),
            }),
            None => Err(Error::NotFound),
        }
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
    // Stamped before reading: a change in between shows up as a conflict
    // on the next save, never as an overwrite.
    let stamp = Some(FileStamp::of(&file.metadata()?));
    let mut t = String::new();
    file.read_to_string(&mut t)?;
    let (bom, crlf, text) = normalize(t);

    let doc = Document::load(text, doc_type).stamped(stamp);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);

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
        let rescanned = Index::scan(&root);
        assert_eq!(rescanned.get(&ids[1]).unwrap().title(), "");
        assert_eq!(rescanned.get(&project).unwrap().title(), "Kitchen");

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
        let mut index = Index::scan(&root);

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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
