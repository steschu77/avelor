//! The indexing (`vault.md#opening`): av-vault's `indexing`
//! walks and reads the vault's files on a worker, the unchanged ones from
//! the vault's catalog (`vault.md#catalog`), and sends what it read in
//! batches; the UI thread puts each batch into the index, which hands the
//! reads to the catalog, and tells the page (`vault`, `entity`,
//! `vault_error`, the indexing's `job`). A new indexing replaces the one running.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use av_store::catalog::{CachedFile, Catalog};
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_vault::vault::{self, FileText, IndexControl, IndexEvent};
use serde::Serialize;

use crate::app::{App, Progress, Task};
use crate::calendar;
use crate::catalog::CatalogThread;
use crate::email;
use crate::jobs::JobKind;
use crate::texts::Text;

/// A batch holds at most this many events ...
const BATCH_SIZE: usize = 100;
/// ... and goes once this long has passed since the one before, so a slow
/// disk shows its files as they come.
const BATCH_TIME: Duration = Duration::from_millis(50);

/// an indexing event, with the time a read was read at: the catalog decides by
/// it whether the file had settled (`av_store::catalog::SETTLED`).
type Indexed = (IndexEvent, Option<SystemTime>);

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
        let catalog = self.vault.catalog().clone();
        let task = self.run_with_progress(
            move |progress, cancel| read_files(&root, &catalog, &progress, cancel),
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
    /// A batch the indexing `id` read: each event applied in turn, then its
    /// reads kept in the catalog.
    fn on_index_batch(&mut self, id: Id, batch: Vec<Indexed>) {
        if !self.indexing.is_running(id) {
            return;
        }
        for (event, read_at) in batch {
            self.on_index_event(event, read_at);
        }
        self.vault.keep_indexed();
    }

    // ------------------------------------------------------------------------
    /// Apply one indexing event to the vault's index, with the time of its read,
    /// and tell the page: the events `vault` (the start and the end),
    /// `entity`, `vault_error` and the indexing's `job`.
    fn on_index_event(&mut self, event: IndexEvent, read_at: Option<SystemTime>) {
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
        let added = self.vault.apply_indexed(event, read_at);
        if let Some(entry) = added.and_then(|id| self.vault.index().entry(&id)) {
            self.post_entity(entry);
        }
        if let (Some(job), Some((done, total))) = (job, step) {
            self.update_job(job, done, total);
        }
    }

    // ------------------------------------------------------------------------
    /// The indexing `id` read every file: the catalog keeps the index's files
    /// only, and the vault knows its emails and events now, so the auto-syncs
    /// may run.
    fn end_indexing(&mut self, id: Id) {
        if !self.indexing.is_running(id) {
            return;
        }
        self.indexing.running = None;
        self.vault.indexing_done();
        let now = DateTime::now();
        email::auto_sync(self, now);
        calendar::auto_sync(self, now);
    }
}

// ----------------------------------------------------------------------------
/// The worker's part: av-vault's `indexing` of `root`, its events sent in
/// batches, until the indexing is done or `cancel` is set. The vault's catalog
/// gives the texts of the files whose stamps are its rows'; every other file
/// is read, and goes with the time of its read.
fn read_files(
    root: &Path,
    catalog: &CatalogThread,
    progress: &Progress<Vec<Indexed>>,
    cancel: &Cancel,
) {
    let mut known = match catalog
        .ask(|catalog| catalog.map(Catalog::cached))
        .flatten()
    {
        Some(Ok(files)) => files.into_iter().map(known_text).collect(),
        Some(Err(err)) => {
            log::warn!(
                "{}: the catalog's rows can't be read: {err}",
                root.display()
            );
            HashMap::new()
        }
        None => HashMap::new(),
    };

    let mut batch = Vec::new();
    let mut sent = Instant::now();
    let (mut total, mut unchanged) = (0, 0);
    vault::index_vault(root, &mut known, |event| {
        let mut read_at = None;
        match &event {
            IndexEvent::Start { total: files, .. } => total = *files,
            IndexEvent::Entry { read: None, .. } => unchanged += 1,
            IndexEvent::Entry { read: Some(_), .. } => read_at = Some(SystemTime::now()),
            IndexEvent::Failed { .. } => {}
            IndexEvent::Done => log::info!(
                "{}: {unchanged} of {total} files from the catalog",
                root.display()
            ),
        }
        // The start shows the vault at once; the end goes with the last files.
        let flush = matches!(event, IndexEvent::Start { .. } | IndexEvent::Done);
        batch.push((event, read_at));
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

// ----------------------------------------------------------------------------
fn known_text(file: CachedFile) -> (PathBuf, FileText) {
    (
        PathBuf::from(file.local),
        FileText {
            stamp: file.stamp,
            text: file.text,
        },
    )
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

    // ------------------------------------------------------------------------
    /// Write the note `id` titled `title` at `path`, modified at `time`.
    fn write_note(path: &Path, id: Id, title: &str, time: SystemTime) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = format!("---\nid: {id}\ntitle: {title}\ndocument: {{}}\n---\nBody\n");
        fs::write(path, text).unwrap();
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(time).unwrap();
    }

    // ------------------------------------------------------------------------
    /// Open `root` as a switch does, and wait for its indexing.
    fn open(h: &mut Harness, root: &Path) {
        crate::api::switch_to(&mut h.app, root.to_path_buf());
        h.wait_indexed();
        h.take();
    }

    // ------------------------------------------------------------------------
    /// The catalog's paths of `names` in `Notes`.
    fn notes(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| format!("Notes/{name}")).collect()
    }

    // ------------------------------------------------------------------------
    fn title(h: &Harness, id: &Id) -> String {
        h.app.vault.index().get(id).unwrap().title().to_string()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_unchanged_file_comes_from_the_catalog() {
        let mut h = Harness::new("indexing-catalog");
        let root = h.vault();
        let (id, path) = (Id::generate(), root.join("Notes/n.md"));
        // Settled long ago, so its row keeps the stamp.
        let then = SystemTime::now() - Duration::from_secs(3600);
        write_note(&path, id, "Old", then);
        open(&mut h, &root);
        assert_eq!(title(&h, &id), "Old");

        // The same size and time: the stamp can't tell, the row's text shows.
        write_note(&path, id, "New", then);
        open(&mut h, &root);
        assert_eq!(title(&h, &id), "Old");

        write_note(&path, id, "Newer", then);
        open(&mut h, &root);
        assert_eq!(title(&h, &id), "Newer");

        // Rebuild index reads every file again, and the page starts over.
        write_note(&path, id, "Final", then);
        let mut sent = h.call("rebuild_index", serde_json::json!({}));
        h.wait_indexed();
        sent.extend(parse(&h.take()));
        let reply = sent.iter().find(|m| m["re"] == 1).unwrap();
        assert_eq!(reply["ok"], Value::Null);
        let states: Vec<&str> = events(&sent, "vault")
            .iter()
            .map(|v| v["data"]["state"].as_str().unwrap())
            .collect();
        assert_eq!(states, ["indexing", "open"]);
        assert_eq!(title(&h, &id), "Final");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_file_read_before_it_settled_is_read_again() {
        let mut h = Harness::new("indexing-unsettled");
        let root = h.vault();
        let (id, path) = (Id::generate(), root.join("Notes/n.md"));
        let now = SystemTime::now();
        write_note(&path, id, "Old", now);
        open(&mut h, &root);
        assert_eq!(h.app.vault.catalog().locals(), notes(&["n.md"]));
        assert!(h.app.vault.catalog().cached().is_empty());

        write_note(&path, id, "New", now);
        open(&mut h, &root);
        assert_eq!(title(&h, &id), "New");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_file_the_app_wrote_is_read_again() {
        let mut h = Harness::new("indexing-written");
        let root = h.vault();
        let (id, path) = (Id::generate(), root.join("Notes/n.md"));
        let then = SystemTime::now() - Duration::from_secs(3600);
        write_note(&path, id, "Old", then);
        open(&mut h, &root);
        assert_eq!(h.app.vault.catalog().cached(), notes(&["n.md"]));

        let args = serde_json::json!({ "id": id.to_string(), "title": "Mid" });
        let sent = h.call("set_title", args);
        assert!(
            sent.iter().any(|m| m["re"] == 1 && m.get("ok").is_some()),
            "{sent:?}"
        );
        assert_eq!(h.app.vault.catalog().locals(), notes(&["n.md"]));
        assert!(h.app.vault.catalog().cached().is_empty(), "no stamp");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn after_indexing_the_catalog_holds_the_files_of_the_index() {
        let mut h = Harness::new("indexing-mirror");
        let root = h.vault();
        let ids = write_notes(&root, "Note", 3);
        open(&mut h, &root);
        let all = notes(&["n0.md", "n1.md", "n2.md"]);
        assert_eq!(h.app.vault.catalog().locals(), all);

        // A file gone, and one with another's id, which the index leaves out.
        fs::remove_file(root.join("Notes/n2.md")).unwrap();
        let twin = format!("---\nid: {}\ntitle: Twin\n---\n", ids[0]);
        fs::write(root.join("Notes/twin.md"), twin).unwrap();
        open(&mut h, &root);
        assert_eq!(h.app.vault.catalog().locals(), notes(&["n0.md", "n1.md"]));

        let sent = h.call(
            "delete_entity",
            serde_json::json!({ "id": ids[1].to_string() }),
        );
        assert!(
            sent.iter().any(|m| m["re"] == 1 && m.get("ok").is_some()),
            "{sent:?}"
        );
        assert_eq!(h.app.vault.catalog().locals(), notes(&["n0.md"]));
        let sent = h.call(
            "restore_entity",
            serde_json::json!({ "path": "Notes/n1.md" }),
        );
        assert!(
            sent.iter().any(|m| m["re"] == 1 && m.get("ok").is_some()),
            "{sent:?}"
        );
        assert_eq!(h.app.vault.catalog().locals(), notes(&["n0.md", "n1.md"]));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn without_its_folder_every_file_is_read() {
        let mut h = Harness::new("indexing-no-catalog");
        fs::write(h.dir().join("file"), "x").unwrap();
        h.app
            .vault
            .set_catalog_dir(h.dir().join("file").join("catalog"));
        let root = h.vault();
        let (id, path) = (Id::generate(), root.join("Notes/n.md"));
        let then = SystemTime::now() - Duration::from_secs(3600);
        write_note(&path, id, "Old", then);
        open(&mut h, &root);
        assert_eq!(title(&h, &id), "Old");

        write_note(&path, id, "New", then);
        open(&mut h, &root);
        assert_eq!(title(&h, &id), "New");
        assert!(h.app.vault.catalog().locals().is_empty());
    }
}
