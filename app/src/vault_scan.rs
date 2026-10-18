//! The vault scan (`vault.md#opening`): av-vault's `scan`
//! walks and reads the vault's files on a worker and sends what it read in
//! batches; the UI thread puts each batch into the index and tells the page
//! (`vault`, `entity`, `vault_error`, the scan's `job`). A new scan
//! replaces the one running.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_vault::vault::{self, ScanControl, ScanEvent};
use serde::Serialize;

use crate::app::{App, Progress, Task};
use crate::calendar;
use crate::jobs::JobKind;
use crate::mail;

/// A batch holds at most this many events ...
const BATCH_SIZE: usize = 100;
/// ... and goes once this long has passed since the one before, so a slow
/// disk shows its files as they come.
const BATCH_TIME: Duration = Duration::from_millis(50);

// ----------------------------------------------------------------------------
/// The vault scan running, if any (`App::run_scan`).
#[derive(Default)]
pub struct VaultScan {
    running: Option<Scan>,
}

impl VaultScan {
    // ------------------------------------------------------------------------
    /// Whether `id` is the scan running: a scan another one replaced is
    /// over, and what it queued is dropped.
    fn is_running(&self, id: Id) -> bool {
        self.running.as_ref().is_some_and(|scan| scan.id == id)
    }
}

// ----------------------------------------------------------------------------
struct Scan {
    /// Names the scan in its continuations.
    id: Id,
    /// The worker, aborted when another scan replaces this one.
    task: Task,
    /// Shown as a job once it knows how many files there are
    /// (`ScanEvent::Start`).
    job: Option<Id>,
    /// The files that couldn't be read so far (`ScanEvent::Failed`).
    failed: usize,
    /// The vault as the scan's start found it (`ScanEvent::Start`).
    vault: Option<VaultInfo>,
    started: Instant,
}

// ----------------------------------------------------------------------------
/// The vault being opened, as the page sees it (the `vault` event): its
/// folder's name and path, and the files the scan found.
#[derive(Clone, Serialize)]
struct VaultInfo {
    /// `scanning`, then `open` once every file was read.
    state: &'static str,
    name: String,
    path: String,
    total: usize,
}

impl App {
    // ------------------------------------------------------------------------
    /// Whether the vault scan still runs: until it ends, the index doesn't
    /// know every entity.
    pub fn scanning(&self) -> bool {
        self.scan.running.is_some()
    }

    // ------------------------------------------------------------------------
    /// Start scanning `root` on a worker, replacing the scan running: its
    /// worker is aborted, and what it queued is dropped.
    pub fn run_scan(&mut self, root: PathBuf) {
        let id = Id::generate();
        let task = self.run_with_progress(
            move |progress, cancel| read_files(&root, &progress, cancel),
            move |app, batch| app.on_scan_batch(id, batch),
            move |app, ()| app.end_scan(id),
        );
        let scan = Scan {
            id,
            task,
            job: None,
            failed: 0,
            vault: None,
            started: Instant::now(),
        };
        if let Some(replaced) = self.scan.running.replace(scan) {
            replaced.task.abort();
            if let Some(job) = replaced.job {
                // Its job card would wait for an end that never comes.
                self.end_job(job, "replaced by another scan");
            }
        }
    }

    // ------------------------------------------------------------------------
    /// A batch the scan `id` read: each event applied in turn.
    fn on_scan_batch(&mut self, id: Id, batch: Vec<ScanEvent>) {
        if !self.scan.is_running(id) {
            return;
        }
        for event in batch {
            self.on_scan_event(event);
        }
    }

    // ------------------------------------------------------------------------
    /// Apply one scan event to the vault's index and tell the page: the
    /// events `vault` (the start and the end), `entity`, `vault_error` and
    /// the scan's `job`.
    fn on_scan_event(&mut self, event: ScanEvent) {
        let Some(scan) = &mut self.scan.running else {
            return;
        };
        let job = scan.job;
        let mut step = None;
        match &event {
            ScanEvent::Start { name, path, total } => {
                let vault = VaultInfo {
                    state: "scanning",
                    name: name.clone(),
                    path: path.clone(),
                    total: *total,
                };
                self.post_event("vault", &vault);
                let label = format!("Scanning {path}");
                let job = self.start_job(JobKind::VaultScan, label, *total);
                if let Some(scan) = &mut self.scan.running {
                    scan.job = Some(job);
                    scan.vault = Some(vault);
                }
            }
            ScanEvent::Entry { index, total, .. } => step = Some((*index, *total)),
            ScanEvent::Failed { path, error } => {
                scan.failed += 1;
                let path = path.display().to_string();
                let data = serde_json::json!({ "path": path, "error": error });
                self.post_event("vault_error", data);
            }
            ScanEvent::Done => {
                let (failed, started) = (scan.failed, scan.started);
                if let Some(vault) = scan.vault.clone() {
                    let ms = started.elapsed().as_millis();
                    let (path, total) = (&vault.path, vault.total);
                    log::info!("Scanned {path}: {total} files, {failed} unreadable, in {ms} ms");
                    let open = VaultInfo {
                        state: "open",
                        ..vault
                    };
                    self.post_event("vault", open);
                }
                // A card with a note stays long enough to be read.
                let note = match failed {
                    0 => String::new(),
                    1 => "1 file couldn't be read".to_string(),
                    n => format!("{n} files couldn't be read"),
                };
                if let Some(job) = job {
                    self.end_job(job, &note);
                }
            }
        }
        let added = self.vault.apply_scanned(event);
        if let Some(entry) = added.and_then(|id| self.vault.index().entry(&id)) {
            self.post_entity(entry);
        }
        if let (Some(job), Some((done, total))) = (job, step) {
            self.update_job(job, done, total);
        }
    }

    // ------------------------------------------------------------------------
    /// The scan `id` read every file: the vault knows its mails and events
    /// now, so the auto-syncs may run.
    fn end_scan(&mut self, id: Id) {
        if !self.scan.is_running(id) {
            return;
        }
        self.scan.running = None;
        let now = DateTime::now();
        mail::auto_sync(self, now);
        calendar::auto_sync(self, now);
    }
}

// ----------------------------------------------------------------------------
/// The worker's part: av-vault's `scan` of `root`, its events sent in
/// batches, until the scan is done or `cancel` is set.
fn read_files(root: &Path, progress: &Progress<Vec<ScanEvent>>, cancel: &Cancel) {
    let mut batch = Vec::new();
    let mut sent = Instant::now();
    vault::scan(root, |event| {
        // The start shows the vault at once; the end goes with the last files.
        let flush = matches!(event, ScanEvent::Start { .. } | ScanEvent::Done);
        batch.push(event);
        if flush || batch.len() >= BATCH_SIZE || sent.elapsed() >= BATCH_TIME {
            progress.send(std::mem::take(&mut batch));
            sent = Instant::now();
        }
        if cancel.is_cancelled() {
            ScanControl::Stop
        } else {
            ScanControl::Continue
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
        let mut h = Harness::new("scan-batches");
        let root = h.vault();
        let ids = write_notes(&root, "Note", 2 * BATCH_SIZE + 50);
        crate::api::switch_to(&mut h.app, root.clone());
        h.wait_scanned();
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
        let mut h = Harness::new("scan-replaced");
        let first = h.vault();
        let second = first.with_file_name("second");
        write_notes(&first, "First", 3);
        let ids = write_notes(&second, "Second", 2);

        crate::api::switch_to(&mut h.app, first.clone());
        // The first scan's worker queues its batches and its end meanwhile.
        let deadline = Instant::now() + Duration::from_secs(5);
        while h.ui.queued() < 3 {
            assert!(Instant::now() < deadline, "the first scan queued nothing");
            std::thread::sleep(Duration::from_millis(1));
        }
        crate::api::switch_to(&mut h.app, second.clone());
        h.wait_scanned();
        let sent = parse(&h.take());

        let path = second.to_str().unwrap();
        let vault = events(&sent, "vault");
        assert!(vault.iter().all(|v| v["data"]["path"] == path), "{vault:?}");
        assert_eq!(entity_ids(&sent), sorted(&ids));
        assert_eq!(h.app.vault.index().all().len(), ids.len());
        assert!(!h.app.scanning());
    }
}
