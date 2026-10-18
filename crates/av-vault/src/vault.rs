//! Vault scanning and the in-memory index.
//!
//! The vault is a folder tree; entities are markdown documents with a
//! YAML frontmatter and standalone YAML records, each with an `id` and
//! any number of role blocks.
//! Membership (project → documents, kind → entities) is always computed
//! here, never stored on disk. This structure is rebuilt by a full scan
//! (fast at personal-vault scale).

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::Metadata;
use std::fs::{self, DirEntry};
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::change::Change;
use crate::document::{Document, FileFormat};
use crate::envelope::Conversation;
use crate::envelope::DocumentType;
use crate::envelope::Thread;
use crate::envelope::{Empty, Project, Roles, Source};
use crate::filename;
use crate::schema::layout;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::fs::write_atomic;
use av_utils::id::Id;
use av_utils::text::renormalize;
use av_utils::text::{LineEnding, normalize};
use serde::Deserialize;

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

    pub fn modified(&self) -> SystemTime {
        self.modified
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
/// ([`Index::reload`]) or overwrite it.
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

impl SaveOutcome {
    /// The outcome of this write followed by `next`: a conflict wins, then
    /// a write.
    fn and(self, next: SaveOutcome) -> SaveOutcome {
        match (self, next) {
            (conflict @ SaveOutcome::Conflict(_), _) | (_, conflict @ SaveOutcome::Conflict(_)) => {
                conflict
            }
            (SaveOutcome::Saved, _) | (_, SaveOutcome::Saved) => SaveOutcome::Saved,
            _ => SaveOutcome::Unchanged,
        }
    }
}

// ----------------------------------------------------------------------------
/// What [`Index::refresh`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refresh {
    /// The file is as the app last read or wrote it.
    Unchanged,
    /// The file changed or went away outside the app, and the app had only
    /// read it: taken in. The id the file has now; `None` if it is gone.
    TakenIn(Option<Id>),
    /// The file changed or went away outside the app after the app wrote
    /// its own edits to it: nothing was taken in.
    Conflict,
}

// ----------------------------------------------------------------------------
#[derive(Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub local: PathBuf,
    pub readonly: bool,
    pub bom: String,
    pub crlf: LineEnding,
    /// Whether the app wrote the file since it last read it: a change made
    /// outside the app afterwards replaced the app's own edits
    /// ([`Index::refresh`]).
    pub written: bool,
    pub doc: Document,
}

impl Entry {
    /// An entry for an entity created in the app that has no file yet (see
    /// [`Document::create`]): no path, so [`Index::save`] refuses it.
    pub fn unsaved(doc: Document) -> Self {
        Self {
            path: PathBuf::new(),
            local: PathBuf::new(),
            readonly: false,
            bom: String::new(),
            crlf: LineEnding::default(),
            written: false,
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
/// What a structural operation (thread create/join/leave) did, so the
/// caller can push exactly the changed entities. Filled in as the operation
/// goes, so it is accurate even when the operation fails part-way.
#[derive(Debug, Default)]
pub struct Changes {
    /// An entity new in the index: made in the app, written to its own new
    /// file, or restored from the trash ([`Index::restore`]).
    pub created: Option<Id>,
    /// Entities whose file was rewritten.
    pub saved: Vec<Id>,
    /// Entities whose file was moved to `.trash` and which left the index.
    pub trashed: Vec<Id>,
    /// Entities of `saved` whose body the write changed, so a view showing
    /// the old body can load the new one.
    pub content: Vec<Id>,
}

// ----------------------------------------------------------------------------
/// An entity to make in the app (New, [`Index::create_new`]): its role
/// blocks, which decide its type, and the well-known fields it starts with.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct NewEntity {
    pub roles: Roles,
    pub title: String,
    /// Not given: now; an event's is its `start`.
    pub date: Option<DateTime>,
    pub projects: Vec<Id>,
    pub tags: Vec<String>,
    /// The entity it is linked to (`linked-to`).
    pub linked_to: Option<Id>,
    /// The markdown body; a list or a record has none.
    pub content: String,
}

// ----------------------------------------------------------------------------
/// A file in the trash ([`Index::trashed`], vault.md#trash): where it
/// is, relative to `.trash` -- its vault-relative path, unless a clash gave
/// it a prefix -- and what it holds, or why it couldn't be read.
#[derive(Debug)]
pub struct Trashed {
    pub local: PathBuf,
    pub doc: std::result::Result<Document, String>,
}

impl Trashed {
    /// The date of what it holds, if it could be read and has one.
    pub fn date(&self) -> Option<DateTime> {
        self.doc.as_ref().ok().and_then(Document::date)
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
    /// last read or write is a conflict and isn't touched, unless
    /// `overwrite` (which also recreates a deleted file).
    fn save(&mut self, id: &Id, overwrite: bool) -> Result<SaveOutcome> {
        let Some(&i) = self.by_id.get(id) else {
            return Err(Error::NotFound);
        };
        let entry = &self.documents[i];
        if entry.is_unsaved() {
            return Err(Error::Unsaved);
        }
        if !overwrite && entry.changed_on_disk()? {
            return Ok(SaveOutcome::Conflict(*id));
        }
        let text = renormalize(&entry.bom, entry.crlf, entry.doc.save());
        write_atomic(&entry.path, text.as_bytes())?;
        // Without a stamp the next save is a conflict: the safe side.
        let stamp = FileStamp::read(&entry.path).ok().flatten();

        let entry = &mut self.documents[i];
        entry.written = true;
        entry.doc.written(stamp);
        Ok(SaveOutcome::Saved)
    }

    /// Set `id`'s body content and write it to disk -- but only if that
    /// actually changed anything (`Document::is_dirty`); an unchanged
    /// document is never rewritten. On a conflict (see [`Index::save`]) or
    /// a failure, the in-memory content stays what the file holds.
    pub fn save_content(
        &mut self,
        id: &Id,
        content: String,
        overwrite: bool,
    ) -> Result<SaveOutcome> {
        let (outcome, _) = self.write_change(id, overwrite, |doc| doc.set_content(content))?;
        Ok(outcome)
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
        if entry.is_unsaved() {
            return Err(Error::Unsaved);
        }
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

    /// Look at `id`'s file and take in a change made outside the app when
    /// that loses nothing of the app's (saving.md#file-checks):
    /// if the app had only read the file, it is read again
    /// ([`Index::reload`]); if the app had written its own edits to it,
    /// which the change replaced, it is left alone for the user to decide.
    /// For a document without unsaved edits; those go through a save.
    pub fn refresh(&mut self, id: &Id) -> Result<Refresh> {
        let Some(entry) = self.entry(id) else {
            return Err(Error::NotFound);
        };
        if entry.is_unsaved() || !entry.changed_on_disk()? {
            return Ok(Refresh::Unchanged);
        }
        if entry.written {
            return Ok(Refresh::Conflict);
        }
        Ok(Refresh::TakenIn(self.reload(id)?))
    }

    // ------------------------------------------------------------------------
    // Structural operations: each validates everything it will touch first
    // -- including that no file to rewrite changed outside the app, so a
    // conflict writes nothing -- then changes and saves entity by entity,
    // recording each file it wrote in `changes`. A failed save restores that
    // entity's in-memory state, so the index always matches the disk.

    /// Create a new thread titled `title` with `members` as its members;
    /// its id is `changes.created`. It has no projects of its own: its
    /// projects are its members' (vault.md#thread). The thread's file gets
    /// its generated name (`filename::generated_name`) in the first
    /// member's folder, and is never written over an existing file.
    pub fn create_thread(
        &mut self,
        title: String,
        members: &[Id],
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        let Some(first) = members.first() else {
            return Err(Error::InvalidArgument {
                arg: "members".into(),
            });
        };
        for id in members {
            self.check_writable(id)?;
        }
        if let Some(changed) = self.first_changed(members)? {
            return Ok(SaveOutcome::Conflict(changed));
        }
        let folder = self.entry(first).and_then(|e| e.local.parent());
        let folder = folder.map(Path::to_path_buf).unwrap_or_default();

        let roles = Roles {
            thread: Some(Thread {}),
            ..Default::default()
        };
        let doc = Document::create(
            roles,
            Source::Created(Empty {}),
            title,
            Vec::new(),
            FileFormat::YAML,
        );
        let id = self.create(doc, &folder, changes)?;

        for member in members {
            if let conflict @ SaveOutcome::Conflict(_) =
                self.update(member, changes, |doc| doc.join_thread(id))?
            {
                return Ok(conflict);
            }
        }
        Ok(SaveOutcome::Saved)
    }

    /// Create a new project titled `title`. Its file gets its generated
    /// name (`filename::generated_name`) in `Projects/`, the conventional
    /// folder (vault.md#vault-layout), and is markdown, so its body
    /// can serve as the project's readme.
    pub fn create_project(&mut self, title: String, changes: &mut Changes) -> Result<Id> {
        let roles = Roles {
            project: Some(Project {}),
            ..Default::default()
        };
        let doc = Document::create(
            roles,
            Source::Created(Empty {}),
            title,
            Vec::new(),
            FileFormat::Markdown,
        );
        self.create(doc, Path::new(layout::PROJECTS), changes)
    }

    /// Make the entity `new` (New in the app): a document, task, event,
    /// contact, list or record, as its roles derive -- the other types have
    /// flows of their own, and are refused. It is created here (`created:
    /// {}`), a list or record as YAML without a body, the others as
    /// markdown, in the conventional folder of its type
    /// ([`layout`]). Its `date` is an event's start, else the one given,
    /// else now. Nothing is written if the title is empty, an event has no
    /// start, one of `projects` isn't a project, or `linked_to` isn't in the
    /// vault.
    pub fn create_new(&mut self, new: NewEntity, changes: &mut Changes) -> Result<Id> {
        let invalid = |arg: &str| Error::InvalidArgument { arg: arg.into() };
        let (format, folder) = match new.roles.derived_type() {
            Some(DocumentType::Document) => (FileFormat::Markdown, layout::DOCUMENTS),
            Some(DocumentType::Task) => (FileFormat::Markdown, layout::TASKS),
            Some(DocumentType::Event) => (FileFormat::Markdown, layout::CREATED_EVENTS),
            Some(DocumentType::Contact) => (FileFormat::Markdown, layout::CONTACTS),
            Some(DocumentType::List) => (FileFormat::YAML, layout::LISTS),
            Some(DocumentType::Record) => (FileFormat::YAML, layout::RECORDS),
            _ => return Err(invalid("roles")),
        };
        let title = new.title.trim().to_string();
        if title.is_empty() {
            return Err(invalid("title"));
        }
        let date = match &new.roles.event {
            Some(event) => Some(event.start.ok_or_else(|| invalid("event.start"))?),
            None => new.date,
        };
        let is_project = |id: &Id| {
            self.get(id)
                .is_some_and(|d| d.kind() == DocumentType::Project)
        };
        if !new.projects.iter().all(is_project) {
            return Err(invalid("projects"));
        }
        if new.linked_to.is_some_and(|to| self.get(&to).is_none()) {
            return Err(invalid("linked_to"));
        }

        let mut doc = Document::create(
            new.roles,
            Source::Created(Empty {}),
            title,
            new.projects,
            format,
        );
        if let Some(date) = date {
            doc.set_date(date);
        }
        doc.set_tags(clean_labels(new.tags));
        doc.set_link(new.linked_to);
        let body = new.content.trim_end();
        if format == FileFormat::Markdown && !body.is_empty() {
            doc.set_content(format!("{body}\n"));
        }
        self.create(doc, Path::new(folder), changes)
    }

    /// Write the new entity `doc` to a file with its generated name in
    /// `folder` (vault-relative), never over an existing file, and add it to
    /// the index. Without an open vault (no root) it is `InvalidPath`.
    pub fn create(&mut self, doc: Document, folder: &Path, changes: &mut Changes) -> Result<Id> {
        if self.root.as_os_str().is_empty() {
            // no vault is open
            return Err(Error::InvalidPath {
                path: PathBuf::new(),
            });
        }
        let local = folder.join(filename::generated_name(&doc));
        let path = self.root.join(&local);
        write_new(&path, &doc.save())?;
        let stamp = FileStamp::read(&path).ok().flatten();

        let id = doc.id();
        let mut entry = Entry {
            path,
            local,
            readonly: false,
            bom: String::new(),
            crlf: LineEnding::default(),
            written: true,
            doc,
        };
        entry.doc.written(stamp);
        self.insert(entry);

        changes.created = Some(id);
        Ok(id)
    }

    pub fn join_thread(
        &mut self,
        doc: &Id,
        thread: &Id,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_thread(thread)?;
        self.check_writable(doc)?;
        self.update(doc, changes, |d| d.join_thread(*thread))
    }

    /// Remove `doc` from the thread `thread`. A thread left with fewer than
    /// two members is dissolved: the remaining member leaves as well, and
    /// the thread's file is moved to `.trash` ([`Index::trash`]).
    pub fn leave_thread(
        &mut self,
        doc: &Id,
        thread: &Id,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_thread(thread)?;
        self.check_writable(doc)?;
        let dissolve = self.dissolving(thread, doc)?;
        let rewritten = [
            std::slice::from_ref(doc),
            dissolve.as_deref().unwrap_or_default(),
        ];
        if let Some(changed) = self.first_changed(&rewritten.concat())? {
            return Ok(SaveOutcome::Conflict(changed));
        }

        let outcome = self.update(doc, changes, |d| d.leave_thread(thread))?;
        match dissolve {
            Some(remaining) if !matches!(outcome, SaveOutcome::Conflict(_)) => {
                Ok(outcome.and(self.dissolve(thread, &remaining, changes)?))
            }
            _ => Ok(outcome),
        }
    }

    pub fn join_project(
        &mut self,
        doc: &Id,
        project: &Id,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        match self.get(project) {
            Some(p) if p.kind() == DocumentType::Project => {}
            Some(_) => return Err(Error::WrongKind),
            None => return Err(Error::NotFound),
        }
        self.check_writable(doc)?;
        self.update(doc, changes, |d| d.join_project(*project))
    }

    pub fn leave_project(
        &mut self,
        doc: &Id,
        project: &Id,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        match self.get(project) {
            Some(p) if p.kind() == DocumentType::Project => {}
            Some(_) => return Err(Error::WrongKind),
            None => return Err(Error::NotFound),
        }
        self.check_writable(doc)?;
        self.update(doc, changes, |d| d.leave_project(project))
    }

    /// Set the title of `id`, which may be any entity. An empty title removes
    /// it ([`Document::set_title`]). Only the file's content changes; the file
    /// is not renamed to match (vault.md#filenames).
    pub fn set_title(
        &mut self,
        id: &Id,
        title: String,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_title(title))
    }

    /// Set the labels (`tags`) of `id`, which may be any entity: each one
    /// trimmed, empty ones dropped, a repeated one kept once where it first
    /// appears. No labels removes the key.
    pub fn set_tags(
        &mut self,
        id: &Id,
        tags: Vec<String>,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        let labels = clean_labels(tags);
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_tags(labels))
    }

    /// Link `id`, which may be any entity, to `to`, another entity in the
    /// vault, replacing an older link; `None` removes the `linked-to` key
    /// ([`Document::set_link`]). Deleting an entity leaves the links to it
    /// in place, so restoring it brings them back.
    pub fn set_link(
        &mut self,
        id: &Id,
        to: Option<Id>,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_link(id, to.as_ref())?;
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_link(to))
    }

    /// Star or unstar `id`, which may be any entity; unstarring removes the
    /// key ([`Document::set_starred`]). Nothing is written if it already is.
    pub fn set_starred(
        &mut self,
        id: &Id,
        starred: bool,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_starred(starred))
    }

    /// Archive `id`, which may be any entity, or take it out of the archive;
    /// unarchiving removes the key ([`Document::set_archived`]). Nothing is
    /// written if it already is.
    pub fn set_archived(
        &mut self,
        id: &Id,
        archived: bool,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_archived(archived))
    }

    /// Replace the markdown body of `id`, keeping its frontmatter.
    pub fn set_content(
        &mut self,
        id: &Id,
        content: String,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_content(content))
    }

    /// Apply `change` to `id`, which may be any entity, as one write: for
    /// edits that touch several properties at once (e.g. a synced event
    /// taking a remote version). Nothing is written if nothing changed.
    pub fn edit(
        &mut self,
        id: &Id,
        changes: &mut Changes,
        change: impl FnOnce(&mut Document),
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, change)
    }

    /// Write `change` to each of `ids` as its own write: the outcome per
    /// id, in their order. An entity the change doesn't fit ([`Change::fits`])
    /// is refused (`InvalidArgument`), one that can't be written as by
    /// [`Index::edit`]; either way the others are still written. `now` is
    /// when a task is marked done.
    pub fn edit_entities(
        &mut self,
        ids: &[Id],
        change: &Change,
        now: DateTime,
        changes: &mut Changes,
    ) -> Vec<(Id, Result<SaveOutcome>)> {
        ids.iter()
            .map(|id| {
                let outcome = match self.get(id) {
                    None => Err(Error::NotFound),
                    Some(doc) if !change.fits(doc.kind()) => Err(Error::InvalidArgument {
                        arg: "change".into(),
                    }),
                    Some(_) => self.edit(id, changes, |d| change.apply(d, now)),
                };
                (*id, outcome)
            })
            .collect()
    }

    /// Replace the `conversation` block of `id`, which may be any entity;
    /// `None` drops the role ([`Document::set_conversation`]).
    pub fn set_conversation(
        &mut self,
        id: &Id,
        conversation: Option<Conversation>,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_conversation(conversation))
    }

    /// Delete the entity `id`: its file moves to `.trash` ([`Index::trash`])
    /// unchanged, so a restore ([`Index::restore`]) brings it back as it was
    /// -- with any change made outside the app. Every thread it leaves behind with
    /// fewer than two members is dissolved, as in [`Index::leave_thread`];
    /// a project's members leave it first, so nothing refers to it any
    /// more. The files rewritten must be writable and unchanged on disk, or
    /// nothing is written. Threads aren't deleted this way -- they dissolve
    /// when their members leave.
    pub fn delete(&mut self, id: &Id, changes: &mut Changes) -> Result<SaveOutcome> {
        let Some(entry) = self.entry(id) else {
            return Err(Error::NotFound);
        };
        let kind = entry.doc.kind();
        if kind == DocumentType::Thread {
            return Err(Error::InvalidArgument { arg: "id".into() });
        }
        if !entry.is_deletable() {
            return Err(Error::NotWritable {
                path: entry.local.clone(),
            });
        }
        let threads = entry
            .doc
            .roles()
            .message
            .as_ref()
            .map(|m| m.threads.clone())
            .unwrap_or_default();
        let mut dissolve = Vec::new();
        for thread in threads {
            if let Some(remaining) = self.dissolving(&thread, id)? {
                dissolve.push((thread, remaining));
            }
        }
        let leaving: Vec<Id> = match kind {
            DocumentType::Project => self.members_of(id).iter().map(|e| e.doc.id()).collect(),
            _ => Vec::new(),
        };
        for member in &leaving {
            self.check_writable(member)?;
        }
        let rewritten: Vec<Id> = dissolve
            .iter()
            .flat_map(|(_, r)| r.iter().copied())
            .chain(leaving.iter().copied())
            .collect();
        if let Some(changed) = self.first_changed(&rewritten)? {
            return Ok(SaveOutcome::Conflict(changed));
        }

        for member in &leaving {
            if let conflict @ SaveOutcome::Conflict(_) =
                self.update(member, changes, |d| d.leave_project(id))?
            {
                return Ok(conflict);
            }
        }
        for (thread, remaining) in &dissolve {
            if let conflict @ SaveOutcome::Conflict(_) =
                self.dissolve(thread, remaining, changes)?
            {
                return Ok(conflict);
            }
        }
        self.trash(id, changes)?;
        Ok(SaveOutcome::Saved)
    }

    /// The members `thread` would be left with once `leaving` is gone, if
    /// that is fewer than two and so dissolves it; `None` if it survives. A
    /// dissolve rewrites those members, so they must be writable. A thread
    /// not in the index (a dangling reference) is never dissolved.
    fn dissolving(&self, thread: &Id, leaving: &Id) -> Result<Option<Vec<Id>>> {
        if self.get(thread).is_none() {
            return Ok(None);
        }
        let remaining: Vec<Id> = self
            .thread_members(thread)
            .iter()
            .map(|e| e.doc.id())
            .filter(|id| id != leaving)
            .collect();
        if remaining.len() >= 2 {
            return Ok(None);
        }
        for id in &remaining {
            self.check_writable(id)?;
        }
        Ok(Some(remaining))
    }

    /// Dissolve `thread`: `remaining` leave it, then its file is trashed.
    /// Stops at the first conflict, leaving the thread in place.
    fn dissolve(
        &mut self,
        thread: &Id,
        remaining: &[Id],
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        for id in remaining {
            if let conflict @ SaveOutcome::Conflict(_) =
                self.update(id, changes, |d| d.leave_thread(thread))?
            {
                return Ok(conflict);
            }
        }
        self.trash(thread, changes)?;
        Ok(SaveOutcome::Saved)
    }

    /// Move `id`'s file to `<root>/.trash/<its vault-relative path>` and drop
    /// it from the index. The scan skips dot-folders, so it stays gone until
    /// it is restored ([`Index::restore`]). An already trashed file of the
    /// same name is kept: the new one gets a timestamp prefix.
    pub fn trash(&mut self, id: &Id, changes: &mut Changes) -> Result<()> {
        let Some(entry) = self.entry(id) else {
            return Err(Error::NotFound);
        };
        if entry.is_unsaved() {
            return Err(Error::Unsaved);
        }
        let mut target = self.root.join(layout::TRASH).join(&entry.local);
        if target.exists() {
            let name = entry
                .local
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let stamp = DateTime::now().as_timestamp();
            target.set_file_name(format!("{stamp}.{name}"));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&entry.path, &target)?;
        self.remove(id);
        changes.trashed.push(*id);
        Ok(())
    }

    /// The files in the trash, newest first: the markdown and YAML files
    /// under `<root>/.trash`, read as the scan reads the vault.
    pub fn trashed(&self) -> Vec<Trashed> {
        let mut files: Vec<Trashed> = self
            .trash_files()
            .into_iter()
            .map(|(path, local, readonly, format)| Trashed {
                doc: load(path, local.clone(), readonly, format)
                    .map(|entry| entry.doc)
                    .map_err(|err| err.to_string()),
                local,
            })
            .collect();
        files.sort_by(|a, b| b.date().cmp(&a.date()).then_with(|| a.local.cmp(&b.local)));
        files
    }

    /// Move the trashed file `local` (relative to `.trash`, as
    /// [`Index::trashed`] lists it) back to where it was -- its name without
    /// the prefix [`Index::trash`] adds on a clash -- and add it to the
    /// index; returns its id. Refused with [`Error::FileExists`] when a file is
    /// in that place, or an entity with the file's `id` is in the vault.
    /// The trash folders this leaves empty are removed.
    pub fn restore(&mut self, local: &Path, changes: &mut Changes) -> Result<Id> {
        let (from, format) = self.trashed_file(local)?;
        let target = untrashed(local);
        let to = self.root.join(&target);
        if to.try_exists()? {
            return Err(Error::FileExists { path: target });
        }
        let readonly = fs::metadata(&from)?.permissions().readonly();
        // Read where it is: the move keeps its stamp (time and size).
        let mut entry = load(from.clone(), target.clone(), readonly, format)?;
        let id = entry.doc.id();
        if entry.doc.adopted()
            && let Some(other) = self.entry(&id)
        {
            return Err(Error::FileExists {
                path: other.local.clone(),
            });
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&from, &to)?;
        prune_empty(&self.root.join(layout::TRASH), from.parent());
        entry.path = to;
        self.insert(entry);
        changes.created = Some(id);
        Ok(id)
    }

    /// Apply `change` to the document of the trashed file `local` and write
    /// it back if that changed anything -- e.g. a calendar sync marking an
    /// event it deleted in Google. The file stays in the trash; a read-only
    /// one, or one whose frontmatter didn't parse, isn't written
    /// ([`Error::NotWritable`]).
    pub fn edit_trashed(
        &self,
        local: &Path,
        change: impl FnOnce(&mut Document),
    ) -> Result<SaveOutcome> {
        let (path, format) = self.trashed_file(local)?;
        let readonly = fs::metadata(&path)?.permissions().readonly();
        let mut entry = load(path, local.to_path_buf(), readonly, format)?;
        if !entry.is_writable() {
            return Err(Error::NotWritable {
                path: local.to_path_buf(),
            });
        }
        change(&mut entry.doc);
        if !entry.doc.is_dirty() {
            return Ok(SaveOutcome::Unchanged);
        }
        let text = renormalize(&entry.bom, entry.crlf, entry.doc.save());
        write_atomic(&entry.path, text.as_bytes())?;
        Ok(SaveOutcome::Saved)
    }

    /// Delete the trashed file `local` for good, also a read-only one, and
    /// the trash folders this leaves empty.
    pub fn delete_trashed(&self, local: &Path) -> Result<()> {
        let (path, _) = self.trashed_file(local)?;
        let mut permissions = fs::metadata(&path)?.permissions();
        if permissions.readonly() {
            // Only the trash's own file, about to be deleted.
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(&path, permissions)?;
        }
        fs::remove_file(&path)?;
        prune_empty(&self.root.join(layout::TRASH), path.parent());
        Ok(())
    }

    /// Delete every file [`Index::trashed`] lists for good, and the folders
    /// this leaves empty; anything else in the trash stays. Returns how many
    /// files were deleted, and stops at the first that can't be.
    pub fn empty_trash(&self) -> Result<usize> {
        let files = self.trash_files();
        for (_, local, _, _) in &files {
            self.delete_trashed(local)?;
        }
        Ok(files.len())
    }

    /// The markdown and YAML files under `<root>/.trash`, as the scan's walk
    /// finds them: `(path, local, readonly, format)`, `local` relative to
    /// the trash. None without a vault.
    fn trash_files(&self) -> Vec<(PathBuf, PathBuf, bool, FileFormat)> {
        let mut files = Vec::new();
        if self.root.as_os_str().is_empty() {
            return files;
        }
        walk(
            &self.root.join(layout::TRASH),
            Path::new(""),
            &mut |path, local, readonly, format| files.push((path, local, readonly, format)),
        );
        files
    }

    /// The trashed file `local` and its format: `local` must be a relative
    /// path that stays inside the trash ([`Error::InvalidPath`]) and name a
    /// markdown or YAML file ([`Error::InvalidFileType`]) that is there
    /// ([`Error::FileNotFound`]). Without a vault, every path is invalid.
    fn trashed_file(&self, local: &Path) -> Result<(PathBuf, FileFormat)> {
        let inside = local
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
        if self.root.as_os_str().is_empty() || local.as_os_str().is_empty() || !inside {
            return Err(Error::InvalidPath {
                path: local.to_path_buf(),
            });
        }
        let Some(format) = file_format(local) else {
            return Err(Error::InvalidFileType);
        };
        let path = self.root.join(layout::TRASH).join(local);
        if !path.is_file() {
            return Err(Error::FileNotFound {
                path: local.to_path_buf(),
            });
        }
        Ok((path, format))
    }

    /// Apply `change` to `id`'s document and save it if that changed
    /// anything ([`Index::write_change`]), recording a write in `changes`.
    fn update(
        &mut self,
        id: &Id,
        changes: &mut Changes,
        change: impl FnOnce(&mut Document),
    ) -> Result<SaveOutcome> {
        let (outcome, body_changed) = self.write_change(id, false, change)?;
        if outcome == SaveOutcome::Saved {
            changes.saved.push(*id);
            if body_changed {
                changes.content.push(*id);
            }
        }
        Ok(outcome)
    }

    /// Apply `change` to `id`'s document and save it if that changed
    /// anything; also whether the body changed. On a conflict or a failed
    /// save the document's envelope and body -- all it holds -- are put back
    /// as they were, so memory keeps matching the file.
    fn write_change(
        &mut self,
        id: &Id,
        overwrite: bool,
        change: impl FnOnce(&mut Document),
    ) -> Result<(SaveOutcome, bool)> {
        let Some(&i) = self.by_id.get(id) else {
            return Err(Error::NotFound);
        };
        let doc = &mut self.documents[i].doc;
        let before = doc.snapshot();
        change(doc);
        let body_changed = doc.content() != before.1;
        // An overwrite writes even an unchanged document: the file is what
        // differs (Use ours after the app's own save was written over).
        if !doc.is_dirty() && !overwrite {
            return Ok((SaveOutcome::Unchanged, false));
        }
        let saved = self.save(id, overwrite);
        if !matches!(saved, Ok(SaveOutcome::Saved)) {
            self.documents[i].doc.restore(before);
        }
        Ok((saved?, body_changed))
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

    /// The first of `ids` whose file changed outside the app since it was
    /// last read or written, so rewriting it would be a conflict.
    fn first_changed(&self, ids: &[Id]) -> Result<Option<Id>> {
        for id in ids {
            let Some(entry) = self.entry(id) else {
                return Err(Error::NotFound);
            };
            if entry.changed_on_disk()? {
                return Ok(Some(*id));
            }
        }
        Ok(None)
    }

    /// `id` may link to `to`: another entity in the vault.
    fn check_link(&self, id: &Id, to: Option<&Id>) -> Result<()> {
        match to {
            Some(to) if to == id || self.get(to).is_none() => {
                Err(Error::InvalidArgument { arg: "to".into() })
            }
            _ => Ok(()),
        }
    }

    fn check_thread(&self, id: &Id) -> Result<()> {
        match self.get(id) {
            Some(doc) if doc.kind() == DocumentType::Thread => Ok(()),
            Some(_) => Err(Error::WrongKind),
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
    /// The first entity whose `remote:` source block has the property
    /// `key: value` (e.g. `message-id`), from `account` if one is given.
    pub fn find_remote(&self, account: Option<&Id>, key: &str, value: &str) -> Option<Id> {
        self.documents
            .iter()
            .find(|e| remote_values(&e.doc, account, key).any(|v| v == value))
            .map(|e| e.doc.id())
    }

    // ------------------------------------------------------------------------
    /// Every value of the `remote:` property `key` in the vault, from
    /// `account` if one is given.
    pub fn remote_values(&self, account: Option<&Id>, key: &str) -> HashSet<String> {
        self.documents
            .iter()
            .flat_map(|e| remote_values(&e.doc, account, key))
            .map(str::to_string)
            .collect()
    }

    // ------------------------------------------------------------------------
    /// The files imported from the folder watch `watch` (the `origin`s of
    /// every `scan:` block naming it), each with the entity it went into.
    pub fn scan_origins(&self, watch: &Id) -> HashMap<String, Id> {
        let mut origins = HashMap::new();
        for entry in &self.documents {
            let Some(Source::Scan(scan)) = entry.doc.source() else {
                continue;
            };
            if scan.watch.as_ref() == Some(watch) {
                for origin in scan.origin.as_slice() {
                    origins.insert(origin.clone(), entry.doc.id());
                }
            }
        }
        origins
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

/// The values of the property `key` in `doc`'s `remote:` source block
/// (vault.md#remote: a list of `{key: value}` maps), if it came
/// from `account` (or any account, for `None`).
fn remote_values<'a>(
    doc: &'a Document,
    account: Option<&Id>,
    key: &'a str,
) -> impl Iterator<Item = &'a str> + 'a {
    doc.remote_from(account)
        .into_iter()
        .flat_map(move |remote| remote.values(key))
}

// ----------------------------------------------------------------------------
/// Labels (`tags`) as written: each one trimmed, empty ones dropped, a
/// repeated one kept once where it first appears.
fn clean_labels(tags: Vec<String>) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if !tag.is_empty() && !labels.iter().any(|t| t == tag) {
            labels.push(tag.to_string());
        }
    }
    labels
}

// ----------------------------------------------------------------------------
/// Write `text` to a new file at `path`, creating its folder; fails rather
/// than replace a file that already exists. A failure is
/// `Error::FileWrite` with `path`.
fn write_new(path: &Path, text: &str) -> Result<()> {
    let written = (|| {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(text.as_bytes())
    })();
    written.map_err(|err| Error::FileWrite {
        path: path.to_path_buf(),
        err,
    })
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
        written: false,
        doc,
    })
}

// ----------------------------------------------------------------------------
/// Where the trashed file `local` goes back to: its vault-relative path,
/// without the `YYYYMMDD_hhmmss.` prefix [`Index::trash`] adds on a clash.
/// The names the app gives files start `YYYYMMDDThhmm`, never like that.
fn untrashed(local: &Path) -> PathBuf {
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    match name.split_once('.') {
        Some((stamp, rest)) if is_trash_stamp(stamp) && !rest.is_empty() => {
            local.with_file_name(rest)
        }
        _ => local.to_path_buf(),
    }
}

/// Whether `text` is a timestamp as [`DateTime::as_timestamp`] writes one
/// for a moment: `YYYYMMDD_hhmmss`.
fn is_trash_stamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 15
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 8 {
                *b == b'_'
            } else {
                b.is_ascii_digit()
            }
        })
}

// ----------------------------------------------------------------------------
/// Remove the folder `dir` and its parents below `top` while they are
/// empty; `fs::remove_dir` fails on one that isn't, which ends it.
fn prune_empty(top: &Path, mut dir: Option<&Path>) {
    while let Some(d) = dir {
        if d == top || !d.starts_with(top) || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
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
    fn created_thread_and_membership_survive_a_rescan() {
        let (root, ids) = thread_vault();
        let mut index = Index::scan(&root);

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

        let rescanned = Index::scan(&root);
        assert_eq!(rescanned.get(&thread).unwrap().kind(), DocumentType::Thread);
        assert_eq!(rescanned.thread_members(&thread).len(), 3);
        assert_eq!(
            rescanned.get(&ids[0]).unwrap().content(),
            "Body 0\n",
            "body kept"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn created_project_survives_a_rescan() {
        let (root, _) = thread_vault();
        let mut index = Index::scan(&root);

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

        let rescanned = Index::scan(&root);
        let doc = rescanned.get(&project).unwrap();
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
    fn a_new_task_lands_in_tasks_and_survives_a_rescan() {
        let (root, ids) = thread_vault();
        let mut index = Index::scan(&root);
        let new = NewEntity {
            roles: roles("task: {status: todo, due: 2026-10-09}"),
            title: " Call the landlord ".into(),
            tags: vec![" home ".into(), "home".into(), "".into()],
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

        let rescanned = Index::scan(&root);
        let doc = rescanned.get(&id).unwrap();
        assert_eq!(doc.kind(), DocumentType::Task);
        assert_eq!(doc.title(), "Call the landlord");
        assert_eq!(doc.tags(), ["home"]);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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

        let rescanned = Index::scan(&root);
        assert!(rescanned.get(&thread).is_none());
        for id in &ids {
            assert!(threads_of(&rescanned, id).is_empty());
            let doc = rescanned.get(id).unwrap();
            assert!(doc.roles().message.is_none(), "empty message role dropped");
        }

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_read_only_member_refuses_the_whole_operation() {
        let (root, ids) = thread_vault();
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
            Index::scan(&root).get(&ids[0]).is_none(),
            ".trash isn't scanned"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn deleting_a_member_dissolves_only_threads_it_leaves_below_two() {
        let (root, ids) = thread_vault();
        let mut index = Index::scan(&root);
        let mut changes = Changes::default();
        let big = new_thread(&mut index, "Big", &ids, &mut changes);
        let small = new_thread(&mut index, "Small", &ids[0..2], &mut changes);
        let trashed_member =
            fs::read_to_string(index.entry(&ids[0]).unwrap().path.clone()).unwrap();

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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        assert!(Index::scan(&root).members_of(&project).is_empty());

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn deleting_a_project_writes_nothing_unless_every_member_can_leave() {
        let (root, ids) = thread_vault();
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
    fn starring_rewrites_any_entity() {
        let (root, ids) = thread_vault();
        let project = Id::generate();
        fs::write(
            root.join("p.yaml"),
            format!("id: {project}\nproject: {{}}\ntitle: P\n"),
        )
        .unwrap();
        let mut index = Index::scan(&root);

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
        let rescanned = Index::scan(&root);
        assert!(rescanned.get(&project).unwrap().starred());
        assert!(!rescanned.get(&ids[1]).unwrap().starred());

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
        let mut index = Index::scan(&root);

        let mut changes = Changes::default();
        assert_eq!(
            index.set_archived(&project, true, &mut changes).unwrap(),
            SaveOutcome::Saved
        );
        assert_eq!(changes.saved, [project]);
        assert!(Index::scan(&root).get(&project).unwrap().archived());

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
        let mut index = Index::scan(&root);
        assert_eq!(index.members_of(&project).len(), 1);

        let mut changes = Changes::default();
        let err = index.leave_project(&ids[0], &ids[1], &mut changes);
        assert!(matches!(err, Err(Error::WrongKind)), "not a project");
        let left = index.leave_project(&ids[0], &project, &mut changes);
        assert_eq!(left.unwrap(), SaveOutcome::Saved);
        assert_eq!(changes.saved, [ids[0]]);
        assert!(Index::scan(&root).members_of(&project).is_empty());

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
        let mut index = Index::scan(&root);

        let mut changes = Changes::default();
        let err = index.join_project(&ids[0], &ids[1], &mut changes);
        assert!(matches!(err, Err(Error::WrongKind)), "not a project");
        let joined = index.join_project(&ids[0], &project, &mut changes);
        assert_eq!(joined.unwrap(), SaveOutcome::Saved);
        assert_eq!(changes.saved, [ids[0]]);
        assert_eq!(Index::scan(&root).members_of(&project).len(), 1);

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
        let mut index = Index::scan(&root);
        let mut changes = Changes::default();
        let tags = [" bill ", "", "paid", "bill", "  "]
            .map(String::from)
            .to_vec();
        let set = index.set_tags(&ids[0], tags, &mut changes);
        assert_eq!(set.unwrap(), SaveOutcome::Saved);
        assert_eq!(index.get(&ids[0]).unwrap().tags(), ["bill", "paid"]);

        let set = index.set_tags(&ids[0], Vec::new(), &mut changes);
        assert_eq!(set.unwrap(), SaveOutcome::Saved);
        let text = fs::read_to_string(root.join("Notes/n0.md")).unwrap();
        assert!(!text.contains("tags"), "{text}");

        index.documents[index.by_id[&ids[1]]].readonly = true;
        let refused = index.set_tags(&ids[1], vec!["x".into()], &mut changes);
        assert!(matches!(refused, Err(Error::NotWritable { .. })));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_link_is_set_replaced_and_removed_on_any_entity() {
        let (root, ids) = thread_vault();
        let mut index = Index::scan(&root);
        let mut changes = Changes::default();
        let set = index.set_link(&ids[0], Some(ids[1]), &mut changes);
        assert_eq!(set.unwrap(), SaveOutcome::Saved);
        let set = index.set_link(&ids[0], Some(ids[2]), &mut changes);
        assert_eq!(set.unwrap(), SaveOutcome::Saved);
        assert_eq!(changes.saved, [ids[0], ids[0]]);
        let rescanned = Index::scan(&root);
        assert_eq!(rescanned.get(&ids[0]).unwrap().linked_to(), Some(ids[2]));

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

    #[test]
    fn remote_properties_find_imported_entities() {
        use crate::envelope::{Email, Remote};
        let root = std::env::temp_dir().join(format!("av-vault-test-{}", Id::generate()));
        let mut index = Index::default();
        index.begin(&root);
        let (account, other) = (Id::generate(), Id::generate());

        let mut remote = Remote {
            account: Some(account),
            remote: Vec::new(),
        };
        remote.set("message-id", "<a@x>");
        remote.set("gmail-id", "g1");
        let remote = Source::Remote(remote);
        let roles = Roles {
            email: Some(Email::default()),
            ..Default::default()
        };
        let mut doc = Document::create(
            roles,
            remote,
            "Mail".into(),
            Vec::new(),
            FileFormat::Markdown,
        );
        doc.set_date(DateTime::from_unix_millis(1_790_503_530_000));
        doc.set_content("Body\n".into());
        let mut changes = Changes::default();
        let id = index.create(doc, Path::new("Mail"), &mut changes).unwrap();
        assert!(index.entry(&id).unwrap().local.starts_with("Mail"));
        assert!(
            fs::read_to_string(root.join(&index.entry(&id).unwrap().local))
                .unwrap()
                .contains("date: 2026-09-27T10:05:30Z")
        );

        assert_eq!(
            index.find_remote(Some(&account), "gmail-id", "g1"),
            Some(id)
        );
        assert_eq!(index.find_remote(Some(&other), "gmail-id", "g1"), None);
        assert_eq!(index.find_remote(None, "message-id", "<a@x>"), Some(id));
        assert_eq!(index.find_remote(None, "message-id", "<b@x>"), None);
        assert_eq!(
            index.remote_values(Some(&account), "gmail-id"),
            HashSet::from(["g1".to_string()])
        );
        assert!(index.remote_values(Some(&other), "gmail-id").is_empty());

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
        let rescanned = Index::scan(&root);
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
            rescanned.scan_origins(&watch),
            expected,
            "read back from the files"
        );
    }

    #[test]
    fn editing_writes_several_properties_at_once() {
        use crate::envelope::{Event, Remote};
        let (root, ids) = thread_vault();
        let mut index = Index::scan(&root);
        let start = DateTime::from_unix_millis(1_790_503_530_000);

        let mut changes = Changes::default();
        let edited = index.edit(&ids[0], &mut changes, |doc| {
            doc.set_title("Dentist".into());
            doc.set_date(start);
            doc.set_tags(vec!["conflict".into()]);
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

        let again = Index::scan(&root);
        let doc = again.get(&ids[0]).unwrap();
        assert_eq!(doc.title(), "Dentist");
        assert_eq!(doc.date(), Some(start));
        assert_eq!(doc.tags(), ["conflict"]);
        assert_eq!(doc.kind(), DocumentType::Event);
        assert!(
            matches!(&doc.source(), Some(Source::Remote(r)) if r.get("event-id") == Some("e1"))
        );
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
        let mut index = Index::scan(&root);

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
        };
        let roles = Roles {
            conversation: Some(conversation.clone()),
            ..Default::default()
        };
        let mut doc = Document::create(
            roles,
            Source::Ai(Empty {}),
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

        let again = Index::scan(&root);
        fs::remove_dir_all(&root).unwrap();
        let doc = again.get(&id).expect("scanned back");
        assert_eq!(doc.kind(), DocumentType::Conversation);
        assert_eq!(doc.source(), Some(&Source::Ai(Empty {})));
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
        let mut index = Index::scan(&root);
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
    fn a_conflict_puts_back_everything_an_edit_changed() {
        use crate::envelope::Event;
        let (root, ids) = thread_vault();
        let mut index = Index::scan(&root);
        let before = index.get(&ids[0]).unwrap().save();
        edit_outside(&root, 0, &ids[0], "Theirs, longer\n");

        let mut changes = Changes::default();
        let edited = index.edit(&ids[0], &mut changes, |doc| {
            doc.set_title("Mine".into());
            doc.set_tags(vec!["mine".into()]);
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

        let saved = index.save_content(&ids[0], "A longer body than before\n".into(), false);
        assert_eq!(saved.unwrap(), SaveOutcome::Saved);
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

        let saved = index.save_content(&ids[0], "Mine\n".into(), false);
        assert_eq!(saved.unwrap(), SaveOutcome::Conflict(ids[0]));
        assert_eq!(fs::read_to_string(&path).unwrap(), text);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_deleted_file_is_a_conflict_until_overwritten() {
        let (root, ids) = thread_vault();
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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
        let mut index = Index::scan(&root);
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

        // Use ours writes it back, although memory didn't change; then the
        // file is the app's again.
        let saved = index.save_content(&ids[0], "Mine\n".into(), true);
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

    #[test]
    fn an_edit_writes_each_entity_it_fits_and_refuses_the_others() {
        use crate::change::TaskChange;
        let (root, ids) = thread_vault();
        let mut index = Index::scan(&root);
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
        let doc = Index::scan(&root);
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
}
