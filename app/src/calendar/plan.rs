//! A sync's job, built from the vault: its calendar entity and its events as
//! last synced.

use std::collections::HashMap;
use std::path::Path;

use av_google::calendar_sync::Changes;
use av_google::calendar_sync::{self, CalendarJob, LocalEvent, SyncJob};
use av_utils::datetime::DateTime;
use av_utils::error::Result;
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::{Calendar, DocumentType, Remote, Roles, Source};
use av_vault::shared::ColorRef;

use super::mapping::{exception, exception_to_google, normalize, payload_of};
use super::{CANCELLED, PlannedEvents};
use crate::app::App;
use crate::google::accounts::CalendarSource;
use av_vault::schema::layout;
use av_vault::schema::remote::{CALENDAR_ID, ETAG, EVENT_ID, SNAPSHOT, SYNC_TOKEN, TIME_MIN};

/// How far back a calendar's first sync reaches, in seconds: a year.
const FIRST_SYNC_SPAN: i64 = 365 * 86_400;

// ----------------------------------------------------------------------------
/// The start of a calendar's first sync at `now`: a year back.
pub(super) fn first_sync_start(now: DateTime) -> DateTime {
    now.plus_seconds(-FIRST_SYNC_SPAN)
}

// ----------------------------------------------------------------------------
/// The vault's calendar entity of the account's Google calendar
/// `calendar_id`, if there is one.
pub(super) fn find_calendar(app: &App, account: Id, calendar_id: &str) -> Option<Id> {
    let entries = app.vault.index().all();
    let entry = entries.iter().find(|e| {
        e.doc.kind() == DocumentType::Calendar
            && e.doc
                .remote_from(Some(&account))
                .and_then(|r| r.get(CALENDAR_ID))
                == Some(calendar_id)
    });
    entry.map(|e| e.doc.id())
}

// ----------------------------------------------------------------------------
/// The vault's calendar entity of `source`, created in `Calendar/` if
/// there is none yet.
pub(super) fn calendar_entity(app: &mut App, account: Id, source: &CalendarSource) -> Result<Id> {
    if let Some(calendar) = find_calendar(app, account, &source.id) {
        return Ok(calendar);
    }

    let roles = Roles {
        calendar: Some(Calendar {
            color: source.color.as_deref().map(ColorRef::parse),
            time_zone: None,
        }),
        ..Default::default()
    };
    let mut remote = Remote {
        account: Some(account),
        remote: Vec::new(),
    };
    remote.set(CALENDAR_ID, &source.id);
    let doc = Document::create(
        roles,
        Source::Remote(remote),
        source.name.clone(),
        Vec::new(),
        FileFormat::YAML,
    );
    app.edit(|index, changes| index.create(doc, Path::new(layout::CALENDAR), changes))
}

// ----------------------------------------------------------------------------
/// The job for the calendar entity `calendar`, two-way if `push`, with the
/// changes an auto-sync `listed` already; and its events' ids and snapshots
/// by path. Two-way, it also holds the calendar's events in the trash that
/// Google still has, to delete there, under their path in `.trash`.
pub(super) fn build_job(
    app: &App,
    account: Id,
    calendar: Id,
    source: &CalendarSource,
    push: bool,
    listed: Option<Changes>,
) -> (SyncJob, PlannedEvents) {
    let mut events = HashMap::new();
    let mut locals: Vec<LocalEvent> = app
        .vault
        .index()
        .all()
        .iter()
        .filter(|e| of_calendar(&e.doc, calendar))
        .map(|entry| {
            let local = local_event(&entry.doc, &entry.local, account);
            events.insert(
                entry.local.clone(),
                (entry.doc.id(), local.snapshot.clone()),
            );
            local
        })
        .collect();
    if push {
        for trashed in app.vault.index().trashed() {
            let Ok(doc) = &trashed.doc else {
                continue;
            };
            if !of_calendar(doc, calendar) {
                continue;
            }
            let path = Path::new(layout::TRASH).join(&trashed.local);
            let local = LocalEvent {
                deleted: true,
                ..local_event(doc, &path, account)
            };
            if local.remote_event_id.is_some() && !local.cancelled {
                events.insert(path, (doc.id(), local.snapshot.clone()));
                locals.push(local);
            }
        }
    }

    let job = SyncJob {
        time_min: first_sync_start(DateTime::now()).as_iso8601(),
        push,
        normalize,
        exception,
        calendars: vec![CalendarJob {
            events: locals,
            listed,
            ..calendar_job(app, account, Some(calendar), source)
        }],
    };
    (job, events)
}

// ----------------------------------------------------------------------------
/// Whether `doc` is an event of the calendar entity `calendar`.
fn of_calendar(doc: &Document, calendar: Id) -> bool {
    let event = doc.roles().event.as_ref();
    event.is_some_and(|ev| ev.calendar == Some(calendar))
}

// ----------------------------------------------------------------------------
/// The job's part for `source` without its events: the calendar entity's
/// path and bookkeeping, if it has one yet.
pub(super) fn calendar_job(
    app: &App,
    account: Id,
    calendar: Option<Id>,
    source: &CalendarSource,
) -> CalendarJob {
    let entry = calendar.and_then(|id| app.vault.index().entry(&id));
    let remote = entry.and_then(|e| e.doc.remote_from(Some(&account)));
    let property = |key| remote.and_then(|r| r.get(key)).map(str::to_string);
    CalendarJob {
        calendar_path: entry.map(|e| e.local.clone()).unwrap_or_default(),
        local_calendar_id: calendar.map(|id| id.to_string()).unwrap_or_default(),
        remote_calendar_id: source.id.clone(),
        sync_token: property(SYNC_TOKEN),
        stored_time_min: property(TIME_MIN),
        events: Vec::new(),
        listed: None,
    }
}

// ----------------------------------------------------------------------------
/// An event entity as the sync sees it. Bookkeeping from another account
/// doesn't count: such an event is new to this one.
pub(super) fn local_event(doc: &Document, path: &Path, account: Id) -> LocalEvent {
    let payload = payload_of(doc);
    let remote = doc.remote_from(Some(&account));
    let property = |key| remote.and_then(|r| r.get(key)).map(str::to_string);
    let exceptions = doc.roles().event.iter().flat_map(|e| &e.exceptions);
    LocalEvent {
        local_id: doc.id().to_string(),
        path: path.to_path_buf(),
        snapshot: calendar_sync::snapshot(&payload),
        cancelled: payload.status.as_deref() == Some(CANCELLED),
        payload,
        stored_snapshot: property(SNAPSHOT),
        remote_event_id: property(EVENT_ID),
        etag: property(ETAG),
        remote_calendar_id: property(CALENDAR_ID),
        exceptions: exceptions.map(exception_to_google).collect(),
        deleted: false,
    }
}
