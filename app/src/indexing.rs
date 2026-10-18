//! The indexing (`vault.md#opening`): av-vault's `indexing`
//! walks and reads the vault's files on a worker and sends what it read in
//! batches; the UI thread puts each batch into the index and tells the page
//! (`vault`, `entity`, `vault_error`, the indexing's `job`). A new indexing
//! replaces the one running.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_vault::vault::{self, IndexControl, IndexEvent};
use serde::Serialize;

use crate::app::{App, Progress, Task};
use crate::email;
use crate::jobs::JobKind;
use crate::texts::Text;

/// A batch holds at most this many events ...
const BATCH_SIZE: usize = 100;
/// ... and goes once this long has passed since the one before, so a slow
/// disk shows its files as they come.
const BATCH_TIME: Duration = Duration::from_millis(50);

// ----------------------------------------------------------------------------
/// The indexing running, if any (`App::run_indexing`).
#[derive(Default)]
pub struct Indexing {
    running: Option<Run>,
}

impl Indexing {
    // ------------------------------------------------------------------------
    /// Whether `id` is the indexing running: an indexing another one replaced is
    /// over, and what it queued is dropped.
    fn is_running(&self, id: Id) -> bool {
        self.running.as_ref().is_some_and(|run| run.id == id)
    }
}

// ----------------------------------------------------------------------------
struct Run {
    /// Names the indexing in its continuations.
    id: Id,
    /// The worker, aborted when another indexing replaces this one.
    task: Task,
    /// Shown as a job once it knows how many files there are
    /// (`IndexEvent::Start`).
    job: Option<Id>,
    /// The files that couldn't be read so far (`IndexEvent::Failed`).
    failed: usize,
    /// The vault as the indexing's start found it (`IndexEvent::Start`).
    vault: Option<VaultInfo>,
    started: Instant,
}

// ----------------------------------------------------------------------------
/// The vault being opened, as the page sees it (the `vault` event): its
/// folder's name and path, and the files the indexing found.
#[derive(Clone, Serialize)]
struct VaultInfo {
    /// `indexing`, then `open` once every file was read.
    state: &'static str,
    name: String,
    path: String,
    total: usize,
}

impl App {
    // ------------------------------------------------------------------------
    /// Whether the indexing still runs: until it ends, the index doesn't
    /// know every entity.
    pub fn indexing(&self) -> bool {
        self.indexing.running.is_some()
    }

    // ------------------------------------------------------------------------
    /// Start indexing `root` on a worker, replacing the indexing running: its
    /// worker is aborted, and what it queued is dropped.
    pub fn run_indexing(&mut self, root: PathBuf) {
        let id = Id::generate();
        let task = self.run_with_progress(
            move |progress, cancel| read_files(&root, &progress, cancel),
            move |app, batch| app.on_index_batch(id, batch),
            move |app, ()| app.end_indexing(id),
        );
        let run = Run {
            id,
            task,
            job: None,
            failed: 0,
            vault: None,
            started: Instant::now(),
        };
        if let Some(replaced) = self.indexing.running.replace(run) {
            replaced.task.abort();
            if let Some(job) = replaced.job {
                // Its job card would wait for an end that never comes.
                self.end_job(job, vec![Text::new("indexing.replaced")]);
            }
        }
    }

    // ------------------------------------------------------------------------
    /// A batch the indexing `id` read: each event applied in turn.
    fn on_index_batch(&mut self, id: Id, batch: Vec<IndexEvent>) {
        if !self.indexing.is_running(id) {
            return;
        }
        for event in batch {
            self.on_index_event(event);
        }
    }

    // ------------------------------------------------------------------------
    /// Apply one indexing event to the vault's index and tell the page: the
    /// events `vault` (the start and the end), `entity`, `vault_error` and
    /// the indexing's `job`.
    fn on_index_event(&mut self, event: IndexEvent) {
        let Some(run) = &mut self.indexing.running else {
            return;
        };
        let job = run.job;
        let mut step = None;
        match &event {
            IndexEvent::Start { name, path, total } => {
                let vault = VaultInfo {
                    state: "indexing",
                    name: name.clone(),
                    path: path.clone(),
                    total: *total,
                };
                self.post_event("vault", &vault);
                let label = Text::new("indexing.label").arg("path", path.as_str());
                let job = self.start_job(JobKind::Indexing, label, *total);
                if let Some(run) = &mut self.indexing.running {
                    run.job = Some(job);
                    run.vault = Some(vault);
                }
            }
            IndexEvent::Entry { index, total, .. } => step = Some((*index, *total)),
            IndexEvent::Failed { path, error } => {
                run.failed += 1;
                let path = path.display().to_string();
                let data = serde_json::json!({ "path": path, "error": error });
                self.post_event("vault_error", data);
            }
            IndexEvent::Done => {
                let (failed, started) = (run.failed, run.started);
                if let Some(vault) = run.vault.clone() {
                    let ms = started.elapsed().as_millis();
                    let (path, total) = (&vault.path, vault.total);
                    log::info!("Indexed {path}: {total} files, {failed} unreadable, in {ms} ms");
                    let open = VaultInfo {
                        state: "open",
                        ..vault
                    };
                    self.post_event("vault", open);
                }
                // A card with a summary stays long enough to be read.
                let summary = match failed {
                    0 => Vec::new(),
                    n => vec![Text::new("indexing.unread").arg("n", n)],
                };
                if let Some(job) = job {
                    self.end_job(job, summary);
                }
            }
        }
        let added = self.vault.apply_indexed(event);
        if let Some(entry) = added.and_then(|id| self.vault.index().entry(&id)) {
            self.post_entity(entry);
        }
        if let (Some(job), Some((done, total))) = (job, step) {
            self.update_job(job, done, total);
        }
    }

    // ------------------------------------------------------------------------
    /// The indexing `id` read every file: the vault knows its emails now, so
    /// the auto-syncs may run.
    fn end_indexing(&mut self, id: Id) {
        if !self.indexing.is_running(id) {
            return;
        }
        self.indexing.running = None;
        let now = DateTime::now();
        email::auto_sync(self, now);
    }
}

// ----------------------------------------------------------------------------
/// The worker's part: av-vault's `indexing` of `root`, its events sent in
/// batches, until the indexing is done or `cancel` is set.
fn read_files(root: &Path, progress: &Progress<Vec<IndexEvent>>, cancel: &Cancel) {
    let mut batch = Vec::new();
    let mut sent = Instant::now();
    vault::index_vault(root, |event| {
        // The start shows the vault at once; the end goes with the last files.
        let flush = matches!(event, IndexEvent::Start { .. } | IndexEvent::Done);
        batch.push(event);
        if flush || batch.len() >= BATCH_SIZE || sent.elapsed() >= BATCH_TIME {
            progress.send(std::mem::take(&mut batch));
            sent = Instant::now();
        }
        if cancel.is_cancelled() {
            IndexControl::Stop
        } else {
            IndexControl::Continue
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, parse};
    use serde_json::Value;
    use std::fs;

    // ------------------------------------------------------------------------
    /// `n` notes in `root`'s `Notes`, titled `<title> <i>`; their ids.
    fn write_notes(root: &Path, title: &str, n: usize) -> Vec<Id> {
        fs::create_dir_all(root.join("Notes")).unwrap();
        (0..n)
            .map(|i| {
                let id = Id::generate();
                let text =
                    format!("---\nid: {id}\ntitle: {title} {i}\ndocument: {{}}\n---\nBody\n");
                fs::write(root.join(format!("Notes/n{i}.md")), text).unwrap();
                id
            })
            .collect()
    }

    // ------------------------------------------------------------------------
    /// The events `ev` among what the page got.
    fn events<'a>(sent: &'a [Value], ev: &str) -> Vec<&'a Value> {
        sent.iter().filter(|m| m["ev"] == ev).collect()
    }

    // ------------------------------------------------------------------------
    /// The ids of the `entity` events among what the page got, sorted.
    fn entity_ids(sent: &[Value]) -> Vec<String> {
        let mut ids: Vec<String> = events(sent, "entity")
            .iter()
            .map(|m| m["data"]["id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids
    }

    // ------------------------------------------------------------------------
    fn sorted(ids: &[Id]) -> Vec<String> {
        let mut ids: Vec<String> = ids.iter().map(Id::to_string).collect();
        ids.sort();
        ids
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_scan_of_several_batches_arrives_whole_and_in_order() {
        let mut h = Harness::new("indexing-batches");
        let root = h.vault();
        let ids = write_notes(&root, "Note", 2 * BATCH_SIZE + 50);
        crate::api::switch_to(&mut h.app, root.clone());
        h.wait_indexed();
        let sent = parse(&h.take());

        assert_eq!(entity_ids(&sent), sorted(&ids));
        assert_eq!(h.app.vault.index().all().len(), ids.len());
        let at = |m: &Value| sent.iter().position(|s| s == m).unwrap();
        let (entities, vault) = (events(&sent, "entity"), events(&sent, "vault"));
        assert_eq!(vault.len(), 2);
        assert!(at(vault[0]) < at(entities[0]), "the start first");
        assert!(
            at(entities[entities.len() - 1]) < at(vault[1]),
            "the end last"
        );

        let jobs = events(&sent, "job");
        let done: Vec<u64> = jobs
            .iter()
            .map(|j| j["data"]["done"].as_u64().unwrap())
            .collect();
        assert!(done.windows(2).all(|d| d[0] <= d[1]), "{done:?}");
        assert_eq!(
            jobs.len(),
            ids.len() + 2,
            "a start, a step per file, an end"
        );
        assert_eq!(jobs[jobs.len() - 1]["data"]["state"], "done");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_scan_replaced_before_its_batches_were_applied_leaves_nothing_behind() {
        let mut h = Harness::new("indexing-replaced");
        let first = h.vault();
        let second = first.with_file_name("second");
        write_notes(&first, "First", 3);
        let ids = write_notes(&second, "Second", 2);

        crate::api::switch_to(&mut h.app, first.clone());
        // The first indexing's worker queues its batches and its end meanwhile.
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.ui.queued() < 3 {
            assert!(
                Instant::now() < deadline,
                "the first indexing queued nothing"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        crate::api::switch_to(&mut h.app, second.clone());
        h.wait_indexed();
        let sent = parse(&h.take());

        let path = second.to_str().unwrap();
        let vault = events(&sent, "vault");
        assert!(vault.iter().all(|v| v["data"]["path"] == path), "{vault:?}");
        assert_eq!(entity_ids(&sent), sorted(&ids));
        assert_eq!(h.app.vault.index().all().len(), ids.len());
        assert!(!h.app.indexing());
    }
}
