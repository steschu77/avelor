//! Long operations as jobs (`architecture.md#jobs`): each has a card
//! in the page's jobs panel (the `job` event, `api.md#events`), its
//! requests run under one `Cancel`, and the stoppable ones end early on
//! `cancel_job`. A vault switch stops the jobs that write into the vault and
//! opens the new vault once they have ended.

use std::collections::HashMap;
use std::path::PathBuf;

use av_utils::cancel::Cancel;
use av_utils::error::Error;
use av_utils::id::Id;
use serde::Serialize;

use crate::app::App;
use crate::email;
use crate::protocol::Req;
use crate::texts::Text;
use crate::{api, chat};

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JobKind {
    Indexing,
    EmailFetch,
    CalendarSync,
}

impl JobKind {
    // ------------------------------------------------------------------------
    /// Whether `cancel_job` can stop it: all but the indexing, as a
    /// half-indexed vault helps nobody.
    fn stoppable(self) -> bool {
        self != JobKind::Indexing
    }

    // ------------------------------------------------------------------------
    /// Whether it belongs to the open vault, so a vault switch stops it
    /// first: it writes into the vault (email fetch, calendar sync).
    fn vault_bound(self) -> bool {
        matches!(self, JobKind::EmailFetch | JobKind::CalendarSync)
    }

    // ------------------------------------------------------------------------
    /// What the log calls it.
    fn name(self) -> &'static str {
        match self {
            JobKind::Indexing => "Indexing",
            JobKind::EmailFetch => "Email fetch",
            JobKind::CalendarSync => "Calendar sync",
        }
    }
}

// ----------------------------------------------------------------------------
struct Job {
    kind: JobKind,
    label: Text,
    done: usize,
    total: usize,
    /// Every request of the job runs under it (`App::run_job`).
    cancel: Cancel,
    /// Stopped (`cancel_job`, a vault switch), not ended yet.
    stopping: bool,
}

// ----------------------------------------------------------------------------
/// The jobs running (`App::jobs`).
#[derive(Default)]
pub struct Jobs {
    running: HashMap<Id, Job>,
    /// The vault to open once the stopped jobs bound to the open vault have
    /// ended (`App::switch_vault_when_stopped`).
    pending_vault: Option<PathBuf>,
}

// ----------------------------------------------------------------------------
/// `job`: a job started, made progress, is stopping, or ended.
#[derive(Serialize)]
struct JobEvent<'a> {
    id: Id,
    label: &'a Text,
    state: &'static str,
    done: usize,
    total: usize,
    /// The summary of an ended job, its parts: what it did, or why it
    /// failed.
    summary: &'a [Text],
    stoppable: bool,
}

// ----------------------------------------------------------------------------
/// The `job` event's data for `job` as it is now.
fn event(id: Id, job: &Job, summary: &[Text], ended: bool) -> serde_json::Value {
    let state = match (ended, job.stopping) {
        (true, _) => "done",
        (false, true) => "stopping",
        (false, false) => "running",
    };
    let event = JobEvent {
        id,
        label: &job.label,
        state,
        done: job.done,
        total: job.total,
        summary,
        stoppable: !ended && !job.stopping && job.kind.stoppable(),
    };
    serde_json::to_value(event).unwrap_or_default()
}

impl App {
    // ------------------------------------------------------------------------
    /// Start a job of `kind`, shown as `label` with `total` steps.
    pub fn start_job(&mut self, kind: JobKind, label: Text, total: usize) -> Id {
        let id = Id::generate();
        let job = Job {
            kind,
            label,
            done: 0,
            total,
            cancel: Cancel::new(),
            stopping: false,
        };
        self.post_event("job", event(id, &job, &[], false));
        self.jobs.running.insert(id, job);
        id
    }

    // ------------------------------------------------------------------------
    /// The job `id` has done `done` of `total` steps.
    pub fn update_job(&mut self, id: Id, done: usize, total: usize) {
        let Some(job) = self.jobs.running.get_mut(&id) else {
            return;
        };
        (job.done, job.total) = (done, total);
        let data = event(id, job, &[], false);
        self.post_event("job", data);
    }

    // ------------------------------------------------------------------------
    /// The job `id` ended, with its `summary`'s parts, which the log keeps
    /// too. One that wasn't stopped did all its steps. Then a pending vault
    /// switch may go on.
    pub fn end_job(&mut self, id: Id, summary: Vec<Text>) {
        let Some(mut job) = self.jobs.running.remove(&id) else {
            return;
        };
        let said = serde_json::to_string(&summary).unwrap_or_default();
        log::info!("{} {id}: {said}", job.kind.name());
        if !job.stopping {
            job.done = job.total;
        }
        self.post_event("job", event(id, &job, &summary, true));
        if self.jobs.pending_vault.is_some() {
            // Not from inside the job's own continuation, which may still
            // be using the vault.
            self.ui().send_to_ui(|app| app.open_pending_vault());
        }
    }

    // ------------------------------------------------------------------------
    /// End the job `id` with a summary of its `counts`, which says
    /// whether it was stopped, or stopped at the spend limit (`limit`,
    /// `job_summary`).
    pub fn end_job_with(&mut self, id: Id, counts: Vec<Text>, limit: bool) {
        let summary = job_summary(counts, self.job_stopping(id), limit);
        self.end_job(id, summary);
    }

    // ------------------------------------------------------------------------
    /// Whether the job `id` was stopped (and hasn't ended yet).
    pub fn job_stopping(&self, id: Id) -> bool {
        self.jobs.running.get(&id).is_some_and(|job| job.stopping)
    }

    // ------------------------------------------------------------------------
    /// Run `work` on a worker under the job `id`'s `Cancel`, then `apply` its
    /// output on the UI thread -- also after the job was stopped, so it can
    /// keep what the work did before (a sync's pushes) and end itself. Each
    /// continuation checks that its job is still running.
    pub fn run_job<T: Send + 'static>(
        &self,
        id: Id,
        work: impl FnOnce(&Cancel) -> T + Send + 'static,
        apply: impl FnOnce(&mut App, T) + Send + 'static,
    ) {
        self.spawn(self.job_cancel(id), true, work, apply);
    }

    // ------------------------------------------------------------------------
    /// The `Cancel` of the job `id`; a cancelled one if the job has ended,
    /// so late work stops at once.
    fn job_cancel(&self, id: Id) -> Cancel {
        if let Some(job) = self.jobs.running.get(&id) {
            return job.cancel.clone();
        }
        let cancel = Cancel::new();
        cancel.cancel();
        cancel
    }

    // ------------------------------------------------------------------------
    /// Mark the job `id` stopping and cancel its requests: what is blocked in
    /// a request fails at once, and so does every later one.
    fn stop_job(&mut self, id: Id) {
        let Some(job) = self.jobs.running.get_mut(&id) else {
            return;
        };
        if job.stopping {
            return;
        }
        job.stopping = true;
        job.cancel.cancel();
        let data = event(id, job, &[], false);
        self.post_event("job", data);
    }

    // ------------------------------------------------------------------------
    /// Open `root` once the work bound to the open vault has stopped: the
    /// assistant's replies being written keep what they have so far (as
    /// Stop does), email fetch and calendar sync are stopped, and a sync
    /// first records what it already did. A newer switch replaces this one.
    pub fn switch_vault_when_stopped(&mut self, root: PathBuf) {
        chat::stop_all(self);
        let bound: Vec<Id> = self
            .jobs
            .running
            .iter()
            .filter(|(_, job)| job.kind.vault_bound())
            .map(|(id, _)| *id)
            .collect();
        for id in bound {
            stop(self, id);
        }
        self.jobs.pending_vault = Some(root);
        self.open_pending_vault();
    }

    // ------------------------------------------------------------------------
    /// Open the vault of a pending switch, unless a job bound to the open
    /// vault is still running.
    fn open_pending_vault(&mut self) {
        if self.jobs.running.values().any(|job| job.kind.vault_bound()) {
            return;
        }
        if let Some(root) = self.jobs.pending_vault.take() {
            api::open_vault(self, &root);
        }
    }
}

// ----------------------------------------------------------------------------
/// `cancel_job {id}`: stop the job `id`. Answered once the stop is under way;
/// the job then ends through its events.
pub fn cancel_job(app: &mut App, req: Req, id: Id) {
    let Some(job) = app.jobs.running.get(&id) else {
        return req.err(Error::NotFound);
    };
    if !job.kind.stoppable() {
        let what = format!("stopping {}", job.label.key);
        return req.err(Error::Unsupported { what });
    }
    stop(app, id);
    req.ok(());
}

// ----------------------------------------------------------------------------
/// Stop the job `id`: its requests are cancelled, and the job ends -- at once
/// (email fetch), or once it has kept what it did
/// (calendar sync, whose continuation runs with what the sync returned).
pub fn stop(app: &mut App, id: Id) {
    let Some(kind) = app.jobs.running.get(&id).map(|job| job.kind) else {
        return;
    };
    log::info!("Job {id}: stopping");
    app.stop_job(id);
    match kind {
        JobKind::EmailFetch => email::stop(app),
        JobKind::CalendarSync | JobKind::Indexing => {}
    }
}

// ----------------------------------------------------------------------------
/// A job's counts as the parts of its summary, "3 new · 1 conflict · 2
/// failed": each count that isn't zero, its text given it as `n`; `none` if
/// all are zero.
pub(crate) fn count_summary(counts: Vec<(usize, Text)>, none: Text) -> Vec<Text> {
    let parts: Vec<Text> = counts
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, text)| text.arg("n", n))
        .collect();
    if parts.is_empty() { vec![none] } else { parts }
}

// ----------------------------------------------------------------------------
/// A job's summary from its `counts`: "spend limit reached" after them if
/// the account's spend limit stopped it (`limit`), else "stopped" before
/// them if it was `stopped`.
pub(crate) fn job_summary(counts: Vec<Text>, stopped: bool, limit: bool) -> Vec<Text> {
    match (limit, stopped) {
        (true, _) => [counts, vec![Text::new("job.spend_limit")]].concat(),
        (false, true) => [vec![Text::new("job.stopped")], counts].concat(),
        (false, false) => counts,
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, shown};
    use serde_json::{Value, json};
    use std::path::Path;

    // ------------------------------------------------------------------------
    /// The data of every `ev` event among `sent`.
    fn events(sent: &[String], ev: &str) -> Vec<Value> {
        sent.iter()
            .filter_map(|text| serde_json::from_str::<Value>(text).ok())
            .filter(|message| message["ev"] == ev)
            .map(|message| message["data"].clone())
            .collect()
    }

    // ------------------------------------------------------------------------
    /// A job event as (state, done, total, stoppable).
    fn state(job: &Value) -> (&str, u64, u64, bool) {
        (
            job["state"].as_str().unwrap(),
            job["done"].as_u64().unwrap(),
            job["total"].as_u64().unwrap(),
            job["stoppable"].as_bool().unwrap(),
        )
    }

    // ------------------------------------------------------------------------
    /// The reply to the typed request `id` among `sent`.
    fn reply(sent: &[String], id: u64) -> Value {
        let replies = sent
            .iter()
            .filter_map(|text| serde_json::from_str::<Value>(text).ok());
        replies
            .into_iter()
            .find(|r| r["re"] == id)
            .expect("a reply")
    }

    // ------------------------------------------------------------------------
    fn note_file(i: usize) -> (String, String) {
        let text = format!(
            "---\nid: {}\ntitle: Note {i}\ndocument: {{}}\n---\nBody\n",
            Id::generate()
        );
        (format!("n{i}.md"), text)
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_job_reports_its_start_progress_stop_and_end() {
        let mut h = Harness::new("job-events");
        let id = h
            .app
            .start_job(JobKind::EmailFetch, Text::new("email.fetching"), 4);
        h.app.update_job(id, 1, 4);
        stop(&mut h.app, id);
        assert!(h.app.job_stopping(id));
        h.app.end_job(id, vec![Text::new("job.stopped")]);
        let jobs = events(&h.take(), "job");
        let states: Vec<_> = jobs.iter().map(state).collect();
        assert_eq!(
            states,
            [
                ("running", 0, 4, true),
                ("running", 1, 4, true),
                ("stopping", 1, 4, false),
                ("done", 1, 4, false),
            ]
        );
        assert_eq!(jobs[0]["label"]["key"], "email.fetching");
        assert_eq!(jobs[0]["id"], id.to_string());
        assert_eq!(shown(&jobs[3]["summary"]), "stopped");
        assert!(!h.app.job_stopping(id), "ended");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_job_that_ends_without_a_stop_did_all_its_steps() {
        let mut h = Harness::new("job-complete");
        let id = h
            .app
            .start_job(JobKind::CalendarSync, Text::new("email.fetching"), 1);
        h.app.end_job(id, vec![Text::new("count.new").arg("n", 3)]);
        let jobs = events(&h.take(), "job");
        assert_eq!(state(&jobs[1]), ("done", 1, 1, false));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn cancel_job_refuses_unknown_jobs_and_the_indexing() {
        let mut h = Harness::new("cancel-job");
        let unknown = json!({ "id": 1, "cmd": "cancel_job", "args": { "id": Id::generate() } });
        let sent = h.send(&unknown.to_string());
        assert_eq!(reply(&sent, 1)["err"]["code"], "not_found");

        let indexing = h
            .app
            .start_job(JobKind::Indexing, Text::new("email.fetching"), 9);
        let request = json!({ "id": 2, "cmd": "cancel_job", "args": { "id": indexing } });
        let sent = h.send(&request.to_string());
        assert_eq!(reply(&sent, 2)["err"]["code"], "unsupported");
        assert!(!h.app.job_stopping(indexing));

        let fetch = h
            .app
            .start_job(JobKind::EmailFetch, Text::new("email.fetching"), 2);
        let request = json!({ "id": 3, "cmd": "cancel_job", "args": { "id": fetch } });
        let sent = h.send(&request.to_string());
        assert_eq!(reply(&sent, 3), json!({ "re": 3, "ok": null }));
        let last = events(&sent, "job").pop().unwrap();
        assert_eq!(last["state"], "stopping");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_stopped_jobs_work_is_cancelled_and_still_applied() {
        let mut h = Harness::new("run-job");
        let id = h
            .app
            .start_job(JobKind::CalendarSync, Text::new("email.fetching"), 1);
        h.app.stop_job(id);
        h.app.run_job(
            id,
            |cancel| cancel.is_cancelled(),
            |app, cancelled| app.post_text(&format!("applied, cancelled: {cancelled}")),
        );
        let sent = h.wait_for(|sent| sent.iter().any(|t| t.starts_with("applied")));
        assert!(
            sent.contains(&"applied, cancelled: true".to_string()),
            "{sent:?}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_vault_switch_waits_for_the_jobs_bound_to_the_vault() {
        let mut h = Harness::with_vault("switch-wait", &[note_file(0)]);
        let other = h.vault().parent().unwrap().join("other");
        let (name, text) = note_file(1);
        fs_write(&other.join(name), &text);

        let sync = h
            .app
            .start_job(JobKind::CalendarSync, Text::new("email.fetching"), 1);
        h.take();
        api::switch_to(&mut h.app, other.clone());
        h.run_queue();
        let sent = h.take();
        assert_eq!(
            events(&sent, "job")[0]["state"],
            "stopping",
            "the sync was stopped"
        );
        assert!(events(&sent, "vault").is_empty(), "{sent:?}");
        assert_eq!(h.app.vault.root(), Some(h.vault().as_path()));

        h.app.end_job(sync, vec![Text::new("job.stopped")]);
        h.wait_indexed();
        let sent = h.take();
        assert!(!events(&sent, "vault").is_empty(), "{sent:?}");
        assert_eq!(h.app.vault.root(), Some(other.as_path()));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn indexing_is_a_job_that_cannot_be_stopped() {
        let mut h = Harness::new("indexing");
        let root = h.vault();
        for i in 0..2 {
            let (name, text) = note_file(i);
            fs_write(&root.join(name), &text);
        }
        api::switch_to(&mut h.app, root.clone());
        h.wait_indexed();
        let jobs = events(&h.take(), "job");
        let states: Vec<_> = jobs.iter().map(state).collect();
        assert_eq!(
            states,
            [
                ("running", 0, 2, false),
                ("running", 1, 2, false),
                ("running", 2, 2, false),
                ("done", 2, 2, false),
            ]
        );
        assert!(shown(&jobs[0]["label"]).starts_with("Indexing"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_notice_reaches_the_page_as_an_event() {
        let h = Harness::new("notice");
        h.app
            .notice(Text::new("email.fetch_failed").arg("id", "17"));
        let sent: Vec<Value> = h
            .take()
            .iter()
            .map(|t| serde_json::from_str(t).unwrap())
            .collect();
        assert_eq!(
            sent,
            [
                json!({ "ev": "notice", "data": { "level": "error", "text": { "key": "email.fetch_failed", "args": { "id": "17" } } } })
            ]
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_summary_note_says_a_stop_and_the_spend_limit() {
        let read = || vec![Text::new("count.read").arg("n", 2)];
        let words = |summary: Vec<Text>| shown(&serde_json::to_value(summary).unwrap());
        assert_eq!(words(job_summary(read(), false, false)), "2 read");
        assert_eq!(words(job_summary(read(), true, false)), "stopped · 2 read");
        assert_eq!(
            words(job_summary(read(), true, true)),
            "2 read · spend limit reached"
        );
    }

    // ------------------------------------------------------------------------
    fn fs_write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}
