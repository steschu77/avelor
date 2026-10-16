//! Indexing and the in-memory index.
//!
//! The vault is a folder tree; entities are markdown documents with a
//! YAML frontmatter and standalone YAML records, each with an `id` and
//! any number of role blocks.
//! Membership (project → documents, kind → entities) is always computed
//! here, never stored on disk. This structure is rebuilt by a full scan
//! (fast at personal-vault scale).

use std::collections::HashMap;
use std::fs::{self, DirEntry};
use std::io::ErrorKind;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::document::{Document, FileFormat};
use crate::envelope::DocumentType;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::fs::{FileStamp, write_atomic};
use av_utils::id::Id;
use av_utils::text::renormalize;
use av_utils::text::{LineEnding, normalize};

mod create;
mod indexing;
mod properties;
mod structure;

pub use indexing::{IndexControl, IndexEvent, index_vault};

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
/// What a structural operation did, so the caller can push exactly the
/// changed entities. Filled in as the operation goes, so it is accurate even
/// when the operation fails part-way.
#[derive(Debug, Default)]
pub struct Changes {
    /// An entity new in the index: made in the app, written to its own new
    /// file.
    pub created: Option<Id>,
    /// Entities whose file was rewritten.
    pub saved: Vec<Id>,
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
    fn insert(&mut self, entry: Entry) {
        self.by_id.insert(entry.doc.id(), self.documents.len());
        self.documents.push(entry);
    }

    /// Add an entry that didn't come from an indexing, e.g. an entity created in
    /// the app ([`Entry::unsaved`]).
    pub fn add(&mut self, entry: Entry) -> &Entry {
        self.insert(entry);
        &self.documents[self.documents.len() - 1]
    }

    /// Drop `id`'s entry from the index and return it. Only the in-memory
    /// index changes: a file on disk stays, and comes back on the next indexing.
    pub fn remove(&mut self, id: &Id) -> Option<Entry> {
        let i = self.by_id.remove(id)?;
        let entry = self.documents.swap_remove(i);
        // swap_remove moved the former last entry into slot `i`.
        if let Some(moved) = self.documents.get(i) {
            self.by_id.insert(moved.doc.id(), i);
        }
        Some(entry)
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
        self.write_change(id, overwrite, |doc| doc.set_content(content))
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

    /// Apply `change` to `id`'s document and save it, recording a write in
    /// `changes`.
    fn update(
        &mut self,
        id: &Id,
        changes: &mut Changes,
        change: impl FnOnce(&mut Document),
    ) -> Result<SaveOutcome> {
        let outcome = self.write_change(id, false, change)?;
        if outcome == SaveOutcome::Saved {
            changes.saved.push(*id);
        }
        Ok(outcome)
    }

    /// Apply `change` to `id`'s document and save it if that changed
    /// anything. On a conflict or a failed save the document's envelope and
    /// body -- all it holds -- are put back as they were, so memory keeps
    /// matching the file.
    fn write_change(
        &mut self,
        id: &Id,
        overwrite: bool,
        change: impl FnOnce(&mut Document),
    ) -> Result<SaveOutcome> {
        let Some(&i) = self.by_id.get(id) else {
            return Err(Error::NotFound);
        };
        let doc = &mut self.documents[i].doc;
        let before = doc.snapshot();
        change(doc);
        // An overwrite writes even an unchanged document: the file is what
        // differs (Use ours after the app's own save was written over).
        if !doc.is_dirty() && !overwrite {
            return Ok(SaveOutcome::Unchanged);
        }
        let saved = self.save(id, overwrite);
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

    /// `id` may link to `to`: another entity in the vault.
    fn check_link(&self, id: &Id, to: Option<&Id>) -> Result<()> {
        match to {
            Some(to) if to == id || self.get(to).is_none() => {
                Err(Error::InvalidArgument { arg: "to".into() })
            }
            _ => Ok(()),
        }
    }

    /// The open vault's root: `InvalidPath` without one, as an index without
    /// a vault has an empty root.
    fn open_root(&self) -> Result<&Path> {
        if self.root.as_os_str().is_empty() {
            return Err(Error::InvalidPath {
                path: PathBuf::new(),
            });
        }
        Ok(&self.root)
    }

    fn check_project(&self, id: &Id) -> Result<()> {
        match self.get(id) {
            Some(doc) if doc.kind() == DocumentType::Project => Ok(()),
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
fn visit(
    root: &Path,
    local: &Path,
    file: &DirEntry,
    cb: &mut impl FnMut(PathBuf, PathBuf, bool, FileFormat),
) {
    let path = root.join(local);
    let Some(doc_type) = FileFormat::of_path(&path) else {
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
mod tests;
