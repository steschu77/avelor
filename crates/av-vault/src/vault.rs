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
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::document::{Document, FileFormat};
use av_utils::error::Result;
use av_utils::id::Id;
use av_utils::text::{LineEnding, normalize};

mod indexing;

pub use indexing::{IndexControl, IndexEvent, index_vault};

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
