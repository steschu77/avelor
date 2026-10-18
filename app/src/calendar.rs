//! The calendar sync with Google (calendar.md, api.md#calendar): the mapping
//! between Google's events and the vault's event entities, and the app side of
//! `av_google::calendar_sync` -- building the job from the vault and applying
//! its effects, both on the UI thread.
//!
//! A sync goes plan → run → apply (calendar.md#design): the job is built from
//! the vault, the engine lists and pushes on a worker, and its effects are
//! applied through the vault edit.
//!
//! A two-way sync deletes in Google the calendar's events deleted in the vault
//! (their files in the trash), and marks each trashed file cancelled; one that
//! Google changed since comes back from the trash.
//!
//! The bookkeeping lives in the files' `remote:` blocks (vault.md#remote): a
//! calendar entity names its Google calendar, sync token and time-min; an
//! event its Google id and calendar, its etag, and the snapshot of what was
//! last synced.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use av_google::auth::ScopeAccess;
use av_google::calendar::KEPT_TYPES;
use av_google::calendar_sync::{self, SyncJob};
use av_google::client::Client;
use av_utils::error::Error;
use av_utils::id::Id;
use av_vault::document::Document;
use av_vault::envelope::{DocumentType, Source};
use av_vault::schema::remote::{CALENDAR_ID, EVENT_TYPE};
use serde::{Deserialize, Serialize};
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
    let push = two_way(access, &source);
    let built = build_job(app, account, calendar, &source, push);
    let job_id = start(app, client, account, calendar, &source, built);
    req.ok(json!({ "job": job_id }));
}

// ----------------------------------------------------------------------------
/// Whether a sync of `source` writes back to Google: the account's calendar
/// access is Full, and Google lets it change the calendar's events
/// (`CalendarSource::writable`).
fn two_way(access: ScopeAccess, source: &CalendarSource) -> bool {
    access == ScopeAccess::Full && source.writable()
}

// ----------------------------------------------------------------------------
/// What becomes of a change to a synced calendar's events in Google, as its
/// entity tells the page (`EntityMeta::access`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    /// The sync writes it back.
    TwoWay,
    /// Google keeps its own: the account's calendar access is Read, or
    /// Google lets the account only see the calendar.
    ReadOnly,
    /// It waits: the calendar isn't checked, or its account isn't signed in,
    /// or has no calendar access, so nothing syncs now.
    NotSynced,
}

// ----------------------------------------------------------------------------
/// The access of the calendar entity `calendar`, if it syncs with a Google
/// account.
pub fn access(app: &App, calendar: &Document) -> Option<Access> {
    let Some(Source::Remote(remote)) = calendar.source() else {
        return None;
    };
    let (account, calendar_id) = (remote.account?, remote.get(CALENDAR_ID)?);
    let Ok(google) = app.google.get(account) else {
        return Some(Access::NotSynced);
    };
    let scope = match &google.client {
        Some(client) => client.scope().calendar,
        None => google.file.google.scope().calendar,
    };
    let source = find_source(app, account, calendar_id);
    Some(match source {
        Some(source) if scope == ScopeAccess::ReadOnly || !source.writable() => Access::ReadOnly,
        Some(_) if scope == ScopeAccess::Full && app.google.connected(account).is_ok() => {
            Access::TwoWay
        }
        _ => Access::NotSynced,
    })
}

// ----------------------------------------------------------------------------
/// Whether the calendar entity `calendar` is one Google keeps as it is
/// (`Access::ReadOnly`): what its events sync isn't changed in the vault,
/// and they aren't deleted, as the next change in Google would bring them
/// back.
pub fn read_only(app: &App, calendar: &Id) -> bool {
    let entry = app.vault.index().entry(calendar);
    entry.is_some_and(|e| access(app, &e.doc) == Some(Access::ReadOnly))
}

// ----------------------------------------------------------------------------
/// `NotWritable` if the entity `id` is an event Google keeps as it is, of a
/// calendar read-only in Google (`read_only`) or of a kind it keeps
/// (`kept_kind`), for a change of what it syncs: its title, its body, or
/// its delete.
pub fn kept_by_google(app: &App, id: &Id) -> Option<Error> {
    let entry = app.vault.index().entry(id)?;
    let calendar = entry.doc.roles().event.as_ref()?.calendar;
    let kept = kept_kind(&entry.doc) || calendar.is_some_and(|c| read_only(app, &c));
    kept.then(|| Error::NotWritable {
        path: entry.local.clone(),
    })
}

// ----------------------------------------------------------------------------
/// Whether the event `doc` is of a kind Google keeps as it is (a birthday,
/// an out of office, ..., `KEPT_TYPES`), as its sync recorded it
/// (`event-type`).
fn kept_kind(doc: &Document) -> bool {
    let Some(Source::Remote(remote)) = doc.source() else {
        return false;
    };
    remote
        .get(EVENT_TYPE)
        .is_some_and(|kind| KEPT_TYPES.contains(&kind))
}

// ----------------------------------------------------------------------------
/// Make the calendar entities the vault lacks of the account's checked
/// calendars, so an event can be moved into one before its first sync. Not
/// while the vault is indexed: one not read yet would be made a second time.
/// A failure is logged only, the calendar's sync makes the entity too.
pub fn ensure_entities(app: &mut App, account: Id) {
    if app.indexing() {
        return;
    }
    let Ok(found) = app.google.get(account) else {
        return;
    };
    for source in found.file.google.calendars.clone() {
        if let Err(e) = calendar_entity(app, account, &source) {
            log::warn!("Creating the calendar {} failed: {e}", source.id);
        }
    }
}

// ----------------------------------------------------------------------------
/// Send the account `account`'s calendar entities to the page again, their
/// access having changed with it.
pub fn post_calendars(app: &App, account: Id) {
    let index = app.vault.index();
    let of_account = |doc: &Document| {
        doc.kind() == DocumentType::Calendar
            && matches!(doc.source(), Some(Source::Remote(r)) if r.account == Some(account))
    };
    for entry in index.all().iter().filter(|e| of_account(&e.doc)) {
        app.post_entity(entry);
    }
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
