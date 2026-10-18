//! Long operations as jobs (`architecture.md#jobs`): each has a card
//! in the page's jobs panel (the `job` event, `api.md#events`).

use std::collections::HashMap;
use std::path::PathBuf;

use av_utils::id::Id;
use serde::Serialize;

use crate::app::App;
use crate::texts::Text;
use crate::{api, chat};

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum JobKind {
    Indexing,
}

impl JobKind {
    // ------------------------------------------------------------------------
    /// Whether it can be stopped: not the indexing, as a half-scanned
    /// vault helps nobody.
    fn stoppable(self) -> bool {
        self != JobKind::Indexing
    }

    // ------------------------------------------------------------------------
    /// What the log calls it.
    fn name(self) -> &'static str {
        match self {
            JobKind::Indexing => "Indexing",
        }
    }
}

// ----------------------------------------------------------------------------
struct Job {
    kind: JobKind,
    label: Text,
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
    let state = if ended { "done" } else { "running" };
    let event = JobEvent {
        id,
        label: &job.label,
        state,
        done: job.done,
        total: job.total,
        summary,
        stoppable: !ended && job.kind.stoppable(),
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
    /// too: it did all its steps.
    pub fn end_job(&mut self, id: Id, summary: Vec<Text>) {
        let Some(mut job) = self.jobs.running.remove(&id) else {
            return;
        };
        let said = serde_json::to_string(&summary).unwrap_or_default();
        log::info!("{} {id}: {said}", job.kind.name());
        job.done = job.total;
        self.post_event("job", event(id, &job, &summary, true));
    }

    // ------------------------------------------------------------------------
    /// Open `root` once the work bound to the open vault has stopped: the
    /// assistant's replies being written keep what they have so far (as
    /// Stop does).
    pub fn switch_vault_when_stopped(&mut self, root: PathBuf) {
        chat::stop_all(self);
        api::open_vault(self, &root);
    }
}
