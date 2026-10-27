//! The open vault's catalog (`vault.md#catalog`) on a thread of its own, for
//! the app's life: `Vault` queues every change of the index there in the
//! order it changed the index, so what is asked of the catalog after them
//! -- the scan's cached rows, a search -- sees them all.

use std::path::PathBuf;
use std::sync::mpsc;

use av_store::catalog::{Catalog, FileRow};
use av_utils::error::Result;

/// What the catalog's thread runs, in the order queued.
type Job = Box<dyn FnOnce(&mut CatalogState) + Send>;

// ----------------------------------------------------------------------------
/// What the catalog's thread holds: the open vault's catalog, if it could be
/// opened, and whether a write to it failed since.
#[derive(Default)]
pub struct CatalogState {
    catalog: Option<Catalog>,
    behind: bool,
}

// ----------------------------------------------------------------------------
/// Why the catalog can't answer for the vault.
pub enum Unavailable {
    /// It couldn't be opened.
    Missing,
    /// A write failed since it opened: rows may be missing or old.
    Behind,
}

impl CatalogState {
    // ------------------------------------------------------------------------
    /// The catalog, while it holds every change asked of it.
    pub fn current(&self) -> std::result::Result<&Catalog, Unavailable> {
        match &self.catalog {
            None => Err(Unavailable::Missing),
            Some(_) if self.behind => Err(Unavailable::Behind),
            Some(catalog) => Ok(catalog),
        }
    }
}

// ----------------------------------------------------------------------------
/// The way to the catalog's thread. A clone is another way to the same
/// thread, which ends once the last one is dropped and what was queued has
/// run.
#[derive(Clone)]
pub struct CatalogThread {
    jobs: mpsc::Sender<Job>,
}

impl CatalogThread {
    // ------------------------------------------------------------------------
    pub fn start() -> Self {
        let (jobs, queued) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("avelor-catalog".to_string())
            .spawn(move || {
                let mut state = CatalogState::default();
                for job in queued {
                    job(&mut state);
                }
            });
        if let Err(e) = spawned {
            log::error!("Could not start the catalog's thread: {e}");
        }
        Self { jobs }
    }

    // ------------------------------------------------------------------------
    /// Queue `job`; without the thread it is dropped.
    pub(crate) fn queue(&self, job: impl FnOnce(&mut CatalogState) + Send + 'static) {
        let _ = self.jobs.send(Box::new(job));
    }

    // ------------------------------------------------------------------------
    /// Open the catalog of the vault at `root` in `dir` in place of the one
    /// open, emptied for `rebuild`. One that can't be opened is logged, and
    /// the vault goes without.
    pub fn open(&self, dir: PathBuf, root: PathBuf, rebuild: bool) {
        self.queue(move |state| {
            // The old one first: it may be the same file.
            *state = CatalogState::default();
            match Catalog::open(&dir, &root, rebuild) {
                Ok(opened) => state.catalog = Some(opened),
                Err(err) => log::warn!("{}: no catalog: {err}", root.display()),
            }
        });
    }

    // ------------------------------------------------------------------------
    /// Queue `write`, unless the vault has no catalog. A failure is logged,
    /// and leaves the catalog behind until the vault opens again, which sets
    /// it right: a row left stale doesn't have its file's stamp.
    fn write(&self, write: impl FnOnce(&mut Catalog) -> Result<()> + Send + 'static) {
        self.queue(move |state| {
            if let Some(catalog) = &mut state.catalog
                && let Err(err) = write(catalog)
            {
                log::warn!("The catalog couldn't be written: {err}");
                state.behind = true;
            }
        });
    }

    // ------------------------------------------------------------------------
    pub fn put(&self, rows: Vec<FileRow>) {
        if !rows.is_empty() {
            self.write(move |catalog| catalog.put(&rows));
        }
    }

    // ------------------------------------------------------------------------
    pub fn forget(&self, locals: Vec<String>) {
        if !locals.is_empty() {
            self.write(move |catalog| catalog.forget(locals.iter().map(String::as_str)));
        }
    }

    // ------------------------------------------------------------------------
    pub fn keep_only(&self, locals: Vec<String>) {
        self.write(move |catalog| catalog.keep_only(&locals.iter().map(String::as_str).collect()));
    }

    // ------------------------------------------------------------------------
    /// For a worker, never the UI thread: run `f` on the catalog's thread
    /// once what was queued before has run, and wait for its answer; `None`
    /// without the thread.
    pub fn ask<T: Send + 'static>(
        &self,
        f: impl FnOnce(Option<&Catalog>) -> T + Send + 'static,
    ) -> Option<T> {
        let (answer, answered) = mpsc::sync_channel(1);
        self.queue(move |state| {
            let _ = answer.send(f(state.catalog.as_ref()));
        });
        answered.recv().ok()
    }

    // ------------------------------------------------------------------------
    /// Close the catalog once what was queued has run, and wait for it: a
    /// test's folder can go then.
    #[cfg(test)]
    pub fn close(&self) {
        let (closed, waited) = mpsc::sync_channel(1);
        self.queue(move |state| {
            *state = CatalogState::default();
            let _ = closed.send(());
        });
        let _ = waited.recv();
    }

    // ------------------------------------------------------------------------
    /// The paths of the catalog's rows, sorted, once what was queued has run.
    #[cfg(test)]
    pub fn locals(&self) -> Vec<String> {
        let mut locals = self
            .ask(|catalog| catalog.map(|c| c.locals().unwrap()).unwrap_or_default())
            .unwrap();
        locals.sort();
        locals
    }

    // ------------------------------------------------------------------------
    /// The paths of the cached rows, those with a stamp, sorted.
    #[cfg(test)]
    pub fn cached(&self) -> Vec<String> {
        let cached = self
            .ask(|catalog| catalog.map(|c| c.cached().unwrap()).unwrap_or_default())
            .unwrap();
        let mut locals: Vec<String> = cached.into_iter().map(|file| file.local).collect();
        locals.sort();
        locals
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use av_utils::error::Error;

    // ------------------------------------------------------------------------
    /// Whether the catalog answers, once what was queued has run.
    fn state_of(thread: &CatalogThread) -> &'static str {
        let (answer, answered) = mpsc::sync_channel(1);
        thread.queue(move |state| {
            let _ = answer.send(match state.current() {
                Ok(_) => "current",
                Err(Unavailable::Missing) => "missing",
                Err(Unavailable::Behind) => "behind",
            });
        });
        answered.recv().unwrap()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_failed_write_leaves_the_catalog_behind_until_it_opens_again() {
        let dir =
            std::env::temp_dir().join(format!("avelor-catalog-behind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let thread = CatalogThread::start();
        assert_eq!(state_of(&thread), "missing");
        thread.open(dir.join("catalog"), dir.join("vault"), false);
        assert_eq!(state_of(&thread), "current");
        thread.write(|_| Err(Error::NotFound));
        assert_eq!(state_of(&thread), "behind");
        thread.open(dir.join("catalog"), dir.join("vault"), false);
        assert_eq!(state_of(&thread), "current");
        thread.close();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
