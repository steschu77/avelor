//! Indexing: a vault read into the index on a worker, the events it sends and
//! how it is stopped.

use std::path::{Path, PathBuf};

use super::{Entry, Index, load, walk};
use av_utils::id::Id;

// ----------------------------------------------------------------------------
/// One step of an indexing in progress, reported as entries are loaded. Carries
/// only owned data (no borrows into an `Index`) so it can be produced on a
/// worker thread and handed across to whoever owns the `Index`.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum IndexEvent {
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
pub enum IndexControl {
    Continue,
    Stop,
}

// ----------------------------------------------------------------------------
impl Index {
    /// Convenience wrapper around [`index_vault`] for callers that don't need
    /// progress: indexes `root` and builds a complete `Index` synchronously.
    pub fn build(root: &Path) -> Index {
        let mut index = Index::default();
        index.begin(root);
        self::index_vault(root, |event| {
            index.apply(event);
            IndexControl::Continue
        });
        index
    }

    /// Reset the index and set its root, ready to receive indexing events via
    /// [`Index::apply`]. Call this before starting a new indexing.
    pub fn begin(&mut self, root: &Path) {
        self.root = root.to_path_buf();
        self.documents.clear();
        self.by_id.clear();
        self.failed.clear();
    }

    /// Apply one `IndexEvent` produced by [`index_vault`]: the id of the entry it
    /// added. An entry whose id the index already has is skipped: the app
    /// made or restored that entity while the indexing walked the vault, so its
    /// own entry is the newer one. `Start`/`Done` are no-ops here; callers
    /// use them to drive progress reporting.
    pub fn apply(&mut self, event: IndexEvent) -> Option<Id> {
        match event {
            IndexEvent::Entry { entry, .. } => {
                let id = entry.doc.id();
                if self.by_id.contains_key(&id) {
                    log::warn!("{:?}: {id} is in the vault already", entry.local);
                    return None;
                }
                self.insert(entry);
                Some(id)
            }
            IndexEvent::Failed { path, error } => {
                self.insert_failed(path, error);
                None
            }
            IndexEvent::Start { .. } | IndexEvent::Done => None,
        }
    }

    fn insert_failed(&mut self, path: PathBuf, error: String) {
        self.failed.push((path, error));
    }
}

// ----------------------------------------------------------------------------
/// Index `root` for eligible files (`*.md` / `*.yaml`) and load them,
/// reporting progress through `on_event`. Touches no `Index` -- every event
/// carries owned, `Send` data, so this runs on a worker thread while the UI
/// thread owns the `Index` and applies the events as they arrive via
/// [`Index::apply`] (the app's `indexing`).
///
/// Phases: collect eligible files, emit `Start`; load them one by one,
/// emitting `Entry`/`Failed` per file; emit `Done`. `on_event` answering
/// `Stop` ends the indexing there.
pub fn index_vault(root: &Path, mut on_event: impl FnMut(IndexEvent) -> IndexControl) {
    let mut files = Vec::new();
    walk(
        root,
        &PathBuf::new(),
        &mut |path, local, readonly, format| files.push((path, local, readonly, format)),
    );

    let total = files.len();
    let start = IndexEvent::Start {
        name: root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        path: root.to_string_lossy().to_string(),
        total,
    };
    if matches!(on_event(start), IndexControl::Stop) {
        return;
    }
    for (i, (path, local, readonly, format)) in files.into_iter().enumerate() {
        let event = match load(path.clone(), local, readonly, format) {
            Ok(entry) => IndexEvent::Entry {
                index: i + 1,
                total,
                entry,
            },
            Err(error) => IndexEvent::Failed {
                path,
                error: error.to_string(),
            },
        };
        if matches!(on_event(event), IndexControl::Stop) {
            return;
        }
    }
    on_event(IndexEvent::Done);
}
