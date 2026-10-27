//! The open vault's catalog (`vault.md#catalog`) on a thread of its own, for
//! the app's life: `Vault` queues every change of the index there in the
//! order it changed the index, so what is asked of the catalog after them
//! sees them all. The thread holds the catalog, or none if it couldn't be
//! opened.

use std::path::PathBuf;
use std::sync::mpsc;

use av_store::catalog::{Catalog, FileRow};
use av_utils::error::Result;

/// What the catalog's thread runs, in the order queued.
type Job = Box<dyn FnOnce(&mut Option<Catalog>) + Send>;

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
                let mut catalog = None;
                for job in queued {
                    job(&mut catalog);
                }
            });
        if let Err(e) = spawned {
            log::error!("Could not start the catalog's thread: {e}");
        }
        Self { jobs }
    }

    // ------------------------------------------------------------------------
    /// Queue `job`; without the thread it is dropped.
    fn queue(&self, job: impl FnOnce(&mut Option<Catalog>) + Send + 'static) {
        let _ = self.jobs.send(Box::new(job));
    }

    // ------------------------------------------------------------------------
    /// Open the catalog of the vault at `root` in `dir` in place of the one
    /// open. One that can't be opened is logged, and the vault goes without.
    pub fn open(&self, dir: PathBuf, root: PathBuf) {
        self.queue(move |catalog| {
            // The old one first: it may be the same file.
            *catalog = None;
            match Catalog::open(&dir, &root, false) {
                Ok(opened) => *catalog = Some(opened),
                Err(err) => log::warn!("{}: no catalog: {err}", root.display()),
            }
        });
    }

    // ------------------------------------------------------------------------
    /// Queue `write`, unless the vault has no catalog. A failure is logged;
    /// the next open sets it right, since a row left stale doesn't have its
    /// file's stamp.
    fn write(&self, write: impl FnOnce(&mut Catalog) -> Result<()> + Send + 'static) {
        self.queue(move |catalog| {
            if let Some(catalog) = catalog
                && let Err(err) = write(catalog)
            {
                log::warn!("The catalog couldn't be written: {err}");
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
        self.queue(move |catalog| {
            let _ = answer.send(f(catalog.as_ref()));
        });
        answered.recv().ok()
    }

    // ------------------------------------------------------------------------
    /// Close the catalog once what was queued has run, and wait for it: a
    /// test's folder can go then.
    #[cfg(test)]
    pub fn close(&self) {
        let (closed, waited) = mpsc::sync_channel(1);
        self.queue(move |catalog| {
            *catalog = None;
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
