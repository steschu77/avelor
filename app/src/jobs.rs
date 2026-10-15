//! Long operations as jobs (`architecture.md#jobs`): each has a card
//! in the page's jobs panel (the `job` event, `api.md#events`).

use std::collections::HashMap;

use av_utils::id::Id;
use serde::Serialize;

use crate::app::App;

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JobKind {
    VaultScan,
}

impl JobKind {
    // ------------------------------------------------------------------------
    /// Whether it can be stopped: not the vault scan, as a half-scanned
    /// vault helps nobody.
    fn stoppable(self) -> bool {
        self != JobKind::VaultScan
    }
}

// ----------------------------------------------------------------------------
struct Job {
    kind: JobKind,
    label: String,
    done: usize,
    total: usize,
}

// ----------------------------------------------------------------------------
/// The jobs running (`App::jobs`).
#[derive(Default)]
pub struct Jobs {
    running: HashMap<Id, Job>,
}

// ----------------------------------------------------------------------------
/// `job`: a job started, made progress, or ended.
#[derive(Serialize)]
struct JobEvent<'a> {
    id: Id,
    label: &'a str,
    state: &'static str,
    done: usize,
    total: usize,
    /// The summary of an ended job: what it did, or why it failed.
    note: &'a str,
    stoppable: bool,
}

// ----------------------------------------------------------------------------
/// The `job` event's data for `job` as it is now.
fn event(id: Id, job: &Job, note: &str, ended: bool) -> serde_json::Value {
    let state = if ended { "done" } else { "running" };
    let event = JobEvent {
        id,
        label: &job.label,
        state,
        done: job.done,
        total: job.total,
        note,
        stoppable: !ended && job.kind.stoppable(),
    };
    serde_json::to_value(event).unwrap_or_default()
}

impl App {
    // ------------------------------------------------------------------------
    /// Start a job of `kind`, shown as `label` with `total` steps.
    pub fn start_job(&mut self, kind: JobKind, label: String, total: usize) -> Id {
        let id = Id::generate();
        let job = Job {
            kind,
            label,
            done: 0,
            total,
        };
        self.post_event("job", event(id, &job, "", false));
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
        let data = event(id, job, "", false);
        self.post_event("job", data);
    }

    // ------------------------------------------------------------------------
    /// The job `id` ended, with `note` as its summary: it did all its steps.
    pub fn end_job(&mut self, id: Id, note: &str) {
        let Some(mut job) = self.jobs.running.remove(&id) else {
            return;
        };
        job.done = job.total;
        self.post_event("job", event(id, &job, note, true));
    }
}
