//! The calendar sync with Google (calendar.md, api.md#calendar): the mapping
//! between Google's events and the vault's event entities, and the app side of
//! `av_google::calendar_sync` -- building the job from the vault and applying
//! its effects, both on the UI thread.
//!
//! A sync goes plan → run → apply (calendar.md#design): the job is built from
//! the vault, the engine lists and pushes on a worker, and its effects are
//! applied through the vault edit.
//!
//! The bookkeeping lives in the files' `remote:` blocks (vault.md#remote): a
//! calendar entity names its Google calendar, sync token and time-min; an
//! event its Google id and calendar, its etag, and the snapshot of what was
//! last synced.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use av_google::auth::ScopeAccess;
use av_google::calendar_sync::{self, SyncJob};
use av_google::client::Client;
use av_utils::error::Error;
use av_utils::id::Id;
use serde::Deserialize;
use serde_json::json;

use crate::app::App;
use crate::google;
use crate::google::accounts::CalendarSource;
use crate::jobs::JobKind;
use crate::protocol::Req;
use crate::texts::Text;
use apply::apply;
use plan::{build_job, calendar_entity};

mod apply;
mod mapping;
mod plan;

// ----------------------------------------------------------------------------
/// The status of an event deleted in Google.
const CANCELLED: &str = "cancelled";

/// Per event of a sync, by its vault-relative path: its id, and the
/// snapshot it was planned with.
type PlannedEvents = HashMap<PathBuf, (Id, String)>;

// ----------------------------------------------------------------------------
/// What the app keeps for the calendar sync: the sync in progress (one
/// at a time).
#[derive(Default)]
pub struct CalendarState {
    sync: Option<CalendarSync>,
}

// ----------------------------------------------------------------------------
/// A calendar sync in progress (`CalendarState::sync`).
struct CalendarSync {
    job_id: Id,
    account: Id,
    /// The vault's calendar entity of the synced calendar.
    calendar: Id,
    events: PlannedEvents,
}

/// `calendar_sync`'s args: Google's id of one of the account's checked
/// calendars.
#[derive(Deserialize)]
pub(crate) struct SyncRequest {
    account: Id,
    calendar: String,
}

// ----------------------------------------------------------------------------
/// `calendar_sync {account, calendar}`: sync the checked calendar with the
/// vault as a job, answering `{job}` once it started. The job's events
/// follow: an `entity` event (and `content`, for a changed body) per entity
/// written, a `notice` per failure, then the account (`account`, its
/// `last_sync`) and the job's end with its summary. Stopped (`cancel_job`),
/// the sync still records what it did so far. Refused before any job while
/// another sync runs, for an account that isn't connected or has no
/// calendar access, and for a calendar that isn't one of its sources.
pub fn calendar_sync(app: &mut App, req: Req, request: SyncRequest) {
    if app.calendar.sync.is_some() {
        let what = "a second calendar sync".to_string();
        return req.fail(
            Error::Unsupported { what },
            Text::new("calendar.sync_running"),
        );
    }
    let account = request.account;
    let client = match app.google.connected(account) {
        Ok(client) => client,
        Err(e) => return google::refuse_account(req, e),
    };
    let access = client.scope().calendar;
    if access == ScopeAccess::None {
        let what = "a calendar sync without calendar access".to_string();
        return req.fail(Error::Unsupported { what }, Text::new("calendar.no_access"));
    }
    let Some(source) = find_source(app, account, &request.calendar) else {
        return refuse_source(req);
    };
    let calendar = match calendar_entity(app, account, &source) {
        Ok(calendar) => calendar,
        Err(e) => {
            log::warn!("Creating the calendar {} failed: {e}", source.id);
            return req.fail(e, Text::new("calendar.create_failed"));
        }
    };
    let push = access == ScopeAccess::Full;
    let built = build_job(app, account, calendar, &source, push);
    let job_id = start(app, client, account, calendar, &source, built);
    req.ok(json!({ "job": job_id }));
}

// ----------------------------------------------------------------------------
fn find_source(app: &App, account: Id, calendar_id: &str) -> Option<CalendarSource> {
    let account = app.google.get(account).ok()?;
    let calendars = &account.file.google.calendars;
    calendars.iter().find(|c| c.id == calendar_id).cloned()
}

// ----------------------------------------------------------------------------
/// Refuse a request for a calendar that isn't one of the account's sources.
fn refuse_source(req: Req) {
    let arg = "calendar".to_string();
    req.fail(
        Error::InvalidArgument { arg },
        Text::new("calendar.not_a_source"),
    );
}

// ----------------------------------------------------------------------------
/// Sync `source` with its calendar entity `calendar` as a job, run on a
/// worker: the job built from the vault (`build_job`). The job's id.
fn start(
    app: &mut App,
    client: Arc<Client>,
    account: Id,
    calendar: Id,
    source: &CalendarSource,
    (job, events): (SyncJob, PlannedEvents),
) -> Id {
    let label = Text::new("calendar.syncing").arg("name", source.name.as_str());
    let job_id = app.start_job(JobKind::CalendarSync, label, 1);
    log::info!(
        "Calendar sync {job_id}: {} of {account}, {} events, {}",
        source.id,
        events.len(),
        if job.push { "two-way" } else { "pull only" }
    );
    app.calendar.sync = Some(CalendarSync {
        job_id,
        account,
        calendar,
        events,
    });
    app.run_job(
        job_id,
        move |cancel| calendar_sync::run_sync(&client, cancel, job),
        move |app, result| apply(app, job_id, result),
    );
    job_id
}

#[cfg(test)]
mod tests;
