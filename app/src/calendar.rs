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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use av_google::auth::ScopeAccess;
use av_google::calendar::{Event as GoogleEvent, EventTime};
use av_google::calendar_sync::{self, CalendarJob, Effect, LocalEvent, SyncJob, SyncOutput};
use av_google::client::Client;
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::{Calendar, DocumentType, Empty, Event, Exception, Remote, Roles, Source};
use av_vault::shared::{ColorRef, ContactRef};
use av_vault::vault::SaveOutcome;
use av_vault::{filename, html, markdown, md_writer};
use serde::Deserialize;
use serde_json::json;

use crate::app::App;
use crate::documents;
use crate::google;
use crate::google_accounts::CalendarSource;
use crate::jobs::{self, JobKind};
use crate::protocol::Req;
use av_vault::schema::layout;
use av_vault::schema::remote::{CALENDAR_ID, ETAG, EVENT_ID, SNAPSHOT, SYNC_TOKEN, TIME_MIN};

// ----------------------------------------------------------------------------
/// The status of an event deleted in Google.
const CANCELLED: &str = "cancelled";
/// How far back a calendar's first sync reaches, in seconds: a year.
const FIRST_SYNC_SPAN: i64 = 365 * 86_400;

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

// ----------------------------------------------------------------------------
// The mapping

/// What the sync keeps of an event, both ways: exactly what `to_google`
/// sends. Attendees are only pulled (`attendees_of`), and so are the
/// occurrences of a series that differ from it (`exception_of`).
#[derive(Debug, Default, Clone, PartialEq)]
struct EventFields {
    title: String,
    body: String,
    start: Option<DateTime>,
    end: Option<DateTime>,
    time_zone: Option<String>,
    location: Option<String>,
    status: Option<String>,
    recurrence: Vec<String>,
}

// ----------------------------------------------------------------------------
/// A Google event as the vault stores it.
fn from_google(event: &GoogleEvent) -> EventFields {
    EventFields {
        title: event
            .summary
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string(),
        body: body_of(event.description.as_deref()),
        start: event.start.as_ref().and_then(time_of),
        end: event.end.as_ref().and_then(time_of),
        time_zone: event.start.as_ref().and_then(|t| t.time_zone.clone()),
        location: event.location.clone(),
        status: event.status.clone(),
        recurrence: event.recurrence.clone().unwrap_or_default(),
    }
}

// ----------------------------------------------------------------------------
/// What a push sends (and what change detection hashes): the synced fields
/// in Google's shape, the body as HTML. A local event without an end lasts
/// an hour, or a day.
fn to_google(fields: &EventFields) -> GoogleEvent {
    let time = |t| event_time(t, fields.time_zone.as_deref());
    let end = fields.end.or_else(|| fields.start.map(default_end));
    GoogleEvent {
        summary: Some(fields.title.clone()).filter(|t| !t.is_empty()),
        description: Some(&fields.body)
            .filter(|b| !b.trim().is_empty())
            .map(|b| markdown::to_html(b)),
        location: fields.location.clone(),
        status: fields.status.clone(),
        start: fields.start.map(time),
        end: end.map(time),
        recurrence: Some(fields.recurrence.clone()).filter(|r| !r.is_empty()),
        ..GoogleEvent::default()
    }
}

// ----------------------------------------------------------------------------
/// `t` in Google's shape: a date, or a date-time meant in `time_zone`.
fn event_time(t: DateTime, time_zone: Option<&str>) -> EventTime {
    match t.time {
        None => EventTime {
            date: Some(t.as_iso8601()),
            ..EventTime::default()
        },
        Some(_) => EventTime {
            date_time: Some(t.as_iso8601()),
            time_zone: time_zone.map(str::to_string),
            ..EventTime::default()
        },
    }
}

// ----------------------------------------------------------------------------
/// `SyncJob::normalize`: a Google event the way the vault would push it
/// back after storing it.
fn normalize(event: &GoogleEvent) -> GoogleEvent {
    to_google(&from_google(event))
}

// ----------------------------------------------------------------------------
/// An event entity's synced fields.
fn of_doc(doc: &Document) -> EventFields {
    let event = doc.roles().event.clone().unwrap_or_default();
    EventFields {
        title: doc.title().to_string(),
        body: doc.content().to_string(),
        start: event.start.or(doc.date()),
        end: event.end,
        time_zone: event.time_zone,
        location: event.location,
        status: event.status,
        recurrence: event.recurrence,
    }
}

// ----------------------------------------------------------------------------
/// Write `fields` into `doc`; everything else (calendar, links, attendees,
/// projects, tags, …) stays.
fn apply_to(fields: &EventFields, doc: &mut Document) {
    doc.set_title(fields.title.clone());
    if let Some(start) = fields.start {
        doc.set_date(start);
    }
    doc.set_content(fields.body.clone());
    let mut event = doc.roles().event.clone().unwrap_or_default();
    event.start = fields.start;
    event.end = fields.end;
    event.time_zone = fields.time_zone.clone();
    event.location = fields.location.clone();
    event.status = fields.status.clone();
    event.recurrence = fields.recurrence.clone();
    doc.set_event(Some(event));
}

// ----------------------------------------------------------------------------
/// The pushable payload of an event entity.
fn payload_of(doc: &Document) -> GoogleEvent {
    to_google(&of_doc(doc))
}

// ----------------------------------------------------------------------------
/// A Google event's attendees as the vault keeps them: `Name <email>`, or
/// whichever of the two Google has; rooms and other resources left out.
fn attendees_of(event: &GoogleEvent) -> Vec<ContactRef> {
    let attendees = event.attendees.iter().flatten();
    attendees
        .filter(|a| a.resource != Some(true))
        .filter_map(|a| {
            let name = a.display_name.as_deref().map(str::trim);
            let email = a.email.as_deref().map(str::trim);
            match (
                name.filter(|n| !n.is_empty()),
                email.filter(|e| !e.is_empty()),
            ) {
                (Some(name), Some(email)) => Some(format!("{name} <{email}>")),
                (Some(one), None) | (None, Some(one)) => Some(one.to_string()),
                (None, None) => None,
            }
        })
        .map(ContactRef::Name)
        .collect()
}

// ----------------------------------------------------------------------------
/// Take Google's attendees of `remote`, unless it is cancelled (a deleted
/// event's tombstone lists none).
fn take_attendees(doc: &mut Document, remote: &GoogleEvent) {
    if remote.status.as_deref() == Some(CANCELLED) {
        return;
    }
    let mut event = doc.roles().event.clone().unwrap_or_default();
    event.attendees = attendees_of(remote);
    doc.set_event(Some(event));
}

// ----------------------------------------------------------------------------
/// What the vault keeps of Google's `instance` of the series `series`: the
/// occurrence it replaces, and what differs from the series -- cancelled;
/// moved or of another length (its start and end); another title or place.
/// Its description and guests aren't kept. None without an original start.
fn exception_of(instance: &GoogleEvent, series: &EventFields) -> Option<Exception> {
    let original = instance.original_start_time.as_ref().and_then(time_of)?;
    let mut exception = Exception {
        original,
        status: None,
        start: None,
        end: None,
        title: None,
        location: None,
    };
    if instance.status.as_deref() == Some(CANCELLED) {
        exception.status = Some(CANCELLED.to_string());
        return Some(exception);
    }
    let start = instance
        .start
        .as_ref()
        .and_then(time_of)
        .unwrap_or(original);
    let end = instance.end.as_ref().and_then(time_of);
    let span = end.map(|end| end.as_unix() - start.as_unix());
    if start != original || (span.is_some() && span != span_of(series)) {
        exception.start = Some(start);
        exception.end = end;
    }
    let title = instance.summary.as_deref().unwrap_or_default().trim();
    exception.title = Some(title.to_string()).filter(|t| !t.is_empty() && *t != series.title);
    exception.location = instance
        .location
        .clone()
        .filter(|l| !l.is_empty() && series.location.as_ref() != Some(l));
    Some(exception)
}

// ----------------------------------------------------------------------------
/// The length of the occurrences of the series `fields`, in seconds: an
/// hour, or a day, without an end.
fn span_of(fields: &EventFields) -> Option<i64> {
    let start = fields.start?;
    let end = fields.end.unwrap_or_else(|| default_end(start));
    Some(end.as_unix() - start.as_unix())
}

// ----------------------------------------------------------------------------
/// Whether `exception` keeps nothing but its original start: the
/// occurrence is as the series has it.
fn keeps_nothing(exception: &Exception) -> bool {
    let Exception {
        original: _,
        status,
        start,
        end,
        title,
        location,
    } = exception;
    status.is_none() && start.is_none() && end.is_none() && title.is_none() && location.is_none()
}

// ----------------------------------------------------------------------------
/// What the vault keeps of an occurrence, in Google's shape: what
/// `SyncJob::exception` compares.
fn exception_to_google(exception: &Exception) -> GoogleEvent {
    let time = |t| event_time(t, None);
    GoogleEvent {
        original_start_time: Some(time(exception.original)),
        status: exception.status.clone(),
        start: exception.start.map(time),
        end: exception.end.map(time),
        summary: exception.title.clone(),
        location: exception.location.clone(),
        ..GoogleEvent::default()
    }
}

// ----------------------------------------------------------------------------
/// `SyncJob::exception`: what the vault would keep of Google's `instance`
/// of the series whose payload is `series`.
fn exception(instance: &GoogleEvent, series: &GoogleEvent) -> GoogleEvent {
    exception_of(instance, &from_google(series))
        .map(|x| exception_to_google(&x))
        .unwrap_or_default()
}

// ----------------------------------------------------------------------------
/// Take Google's `instances` of the series `doc`, each relative to the
/// series as it is now: it replaces what the vault kept of its occurrence,
/// or removes it if it differs from the series in nothing kept.
fn take_exceptions(doc: &mut Document, instances: &[GoogleEvent]) {
    let series = of_doc(doc);
    let mut event = doc.roles().event.clone().unwrap_or_default();
    for exception in instances.iter().filter_map(|i| exception_of(i, &series)) {
        event
            .exceptions
            .retain(|x| x.original != exception.original);
        if !keeps_nothing(&exception) {
            event.exceptions.push(exception);
        }
    }
    event.exceptions.sort_by_key(|x| x.original.as_unix());
    doc.set_event(Some(event));
}

// ----------------------------------------------------------------------------
/// A description as markdown: HTML (as Google Calendar writes it) converted,
/// plain text kept line by line.
fn body_of(description: Option<&str>) -> String {
    match description {
        None => String::new(),
        Some(d) if looks_like_html(d) => html::to_markdown(d),
        Some(d) => md_writer::plain_to_markdown(d),
    }
}

// ----------------------------------------------------------------------------
/// Whether `text` contains a tag: `<` before a letter, `/` or `!`, with a
/// `>` later on.
fn looks_like_html(text: &str) -> bool {
    text.match_indices('<').any(|(i, _)| {
        let rest = &text[i + 1..];
        rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '/' || c == '!')
            && rest.contains('>')
    })
}

// ----------------------------------------------------------------------------
fn time_of(time: &EventTime) -> Option<DateTime> {
    let text = time.date_time.as_deref().or(time.date.as_deref())?;
    DateTime::from_iso8601(text).ok()
}

// ----------------------------------------------------------------------------
/// The end of an event starting at `start` that has none: an hour later,
/// or the next day for an all-day event.
fn default_end(start: DateTime) -> DateTime {
    let seconds = if start.time.is_some() { 3600 } else { 86_400 };
    let end = DateTime::from_unix_millis((start.as_unix() + seconds) * 1000);
    DateTime {
        time: start.time.and(end.time),
        ..end
    }
}

// ----------------------------------------------------------------------------
// The job

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
            "Another calendar sync is still running.",
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
        return req.fail(
            Error::Unsupported { what },
            "The account has no calendar access.",
        );
    }
    let Some(source) = find_source(app, account, &request.calendar) else {
        return refuse_source(req);
    };
    let calendar = match calendar_entity(app, account, &source) {
        Ok(calendar) => calendar,
        Err(e) => {
            log::warn!("Creating the calendar {} failed: {e}", source.id);
            return req.fail(e, "The calendar couldn't be created in the vault.");
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
        "The calendar isn't a source of the account.",
    );
}

// ----------------------------------------------------------------------------
/// The start of a calendar's first sync at `now`: a year back.
fn first_sync_start(now: DateTime) -> DateTime {
    DateTime::from_unix_millis((now.as_unix() - FIRST_SYNC_SPAN) * 1000)
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
    let label = format!("Syncing calendar · {}", source.name);
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

// ----------------------------------------------------------------------------
/// The vault's calendar entity of the account's Google calendar
/// `calendar_id`, if there is one.
fn find_calendar(app: &App, account: Id, calendar_id: &str) -> Option<Id> {
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
fn calendar_entity(app: &mut App, account: Id, source: &CalendarSource) -> Result<Id> {
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
/// The job for the calendar entity `calendar`, two-way if `push`; and its
/// events' ids and snapshots by path. Two-way, it also holds the calendar's
/// events in the trash that Google still has, to delete there, under their
/// path in `.trash`.
fn build_job(
    app: &App,
    account: Id,
    calendar: Id,
    source: &CalendarSource,
    push: bool,
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
fn calendar_job(
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
fn local_event(doc: &Document, path: &Path, account: Id) -> LocalEvent {
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

// ----------------------------------------------------------------------------
// Applying the result

/// What a sync came to, for the job's summary.
#[derive(Default)]
struct Counts {
    created: usize,
    updated: usize,
    pushed: usize,
    deleted: usize,
    restored: usize,
    conflicts: usize,
    skipped: usize,
    failed: usize,
}

impl Counts {
    // ------------------------------------------------------------------------
    /// "3 new · 1 updated · 1 conflict": the counts that aren't zero.
    fn summary(&self) -> String {
        let counts = [
            (self.created, "new", "new"),
            (self.updated, "updated", "updated"),
            (self.pushed, "pushed", "pushed"),
            (self.deleted, "deleted", "deleted"),
            (self.restored, "restored", "restored"),
            (self.conflicts, "conflict", "conflicts"),
            (self.skipped, "occurrence skipped", "occurrences skipped"),
            (self.failed, "failed", "failed"),
        ];
        jobs::count_summary(&counts, "nothing new")
    }
}

// ----------------------------------------------------------------------------
/// How applying one effect went.
enum Applied {
    Done,
    /// The event changed in the vault while the sync ran: left for the
    /// next run, which plans it again.
    Skipped,
}

// ----------------------------------------------------------------------------
/// The sync ran, or was stopped and returned what it did so far: apply its
/// effects in order, report, and end the job. Ignored if the job is gone.
fn apply(app: &mut App, job_id: Id, result: Result<SyncOutput>) {
    let Some(sync) = app.calendar.sync.take_if(|s| s.job_id == job_id) else {
        return;
    };
    let output = match result {
        Ok(output) => output,
        Err(Error::Cancelled) => return end_sync(app, job_id, "stopped"),
        Err(Error::Expired) => return end_sync(app, job_id, "reconnect the account"),
        Err(e) => return end_sync(app, job_id, &format!("failed: {e}")),
    };

    // What the stop cancelled isn't a failure; the next sync does it.
    let report = output.report;
    let failures: Vec<_> = report
        .failures
        .iter()
        .filter(|f| !matches!(f.error, Error::Cancelled))
        .collect();
    let mut counts = Counts {
        pushed: report.pushed,
        deleted: report.deleted,
        conflicts: report.conflicts.len(),
        skipped: report.skipped_exceptions,
        failed: failures.len(),
        ..Counts::default()
    };
    // The host's words in the status bar, the error in the log
    // (`App::notice`).
    let fail = |app: &App, message: &str, error: &Error| {
        log::warn!("{message}: {error}");
        app.notice(&format!("{message}."));
    };
    for failure in failures {
        let message = format!("Could not sync {}", failure.subject);
        fail(app, &message, &failure.error);
    }

    // A sync token only moves on once everything before it is applied.
    let mut clean = true;
    for effect in output.effects {
        if matches!(effect, Effect::SetSyncToken { .. }) && !clean {
            log::info!("Calendar sync {job_id}: sync token kept for the next run");
            continue;
        }
        match apply_effect(app, &sync, effect, &mut counts) {
            Ok(Applied::Done) => {}
            Ok(Applied::Skipped) => clean = false,
            Err(e) => {
                clean = false;
                counts.failed += 1;
                fail(app, "Could not write a synced event", &e);
            }
        }
    }
    finish(app, sync, &counts);
}

// ----------------------------------------------------------------------------
fn apply_effect(
    app: &mut App,
    sync: &CalendarSync,
    effect: Effect,
    counts: &mut Counts,
) -> Result<Applied> {
    let account = sync.account;
    match effect {
        Effect::CreateLocal {
            remote,
            remote_calendar_id,
            exceptions,
            ..
        } => {
            create_event(app, sync, &remote, &exceptions, &remote_calendar_id)?;
            counts.created += 1;
        }
        Effect::OverwriteLocal {
            path,
            remote,
            remote_calendar_id,
        } => {
            let Some(id) = unchanged_event(app, sync, &path) else {
                return Ok(Applied::Skipped);
            };
            if take_remote(app, account, id, &remote, &remote_calendar_id)? {
                counts.updated += 1;
            }
        }
        Effect::CancelLocal { path, etag } => {
            let Some(id) = unchanged_event(app, sync, &path) else {
                return Ok(Applied::Skipped);
            };
            if cancel_event(app, account, id, etag.as_deref())? {
                counts.updated += 1;
            }
        }
        Effect::Conflict {
            path,
            remote,
            remote_calendar_id,
        } => {
            let Some(id) = unchanged_event(app, sync, &path) else {
                return Ok(Applied::Skipped);
            };
            keep_local_version(app, id)?;
            match remote {
                Some(remote) => take_remote(app, account, id, &remote, &remote_calendar_id)?,
                None => cancel_event(app, account, id, None)?,
            };
        }
        Effect::SetRemote {
            path,
            remote_event_id,
            etag,
            snapshot,
            remote_calendar_id,
            remote,
        } => {
            // Bookkeeping (and pulled attendees) only: applied even if the
            // event changed meanwhile, which then still counts as changed.
            let Some(&(id, _)) = sync.events.get(&path) else {
                return Ok(Applied::Skipped);
            };
            let before = app
                .vault
                .index()
                .entry(&id)
                .and_then(|e| e.doc.roles().event.clone());
            let result = app.edit(|index, changes| {
                index.edit(&id, changes, |doc| {
                    take_attendees(doc, &remote);
                    set_bookkeeping(doc, account, |r| {
                        r.set(EVENT_ID, &remote_event_id);
                        r.set(CALENDAR_ID, &remote_calendar_id);
                        if let Some(etag) = &etag {
                            r.set(ETAG, etag);
                        }
                        r.set(SNAPSHOT, &snapshot);
                    });
                })
            });
            app.vault.require_written(result?)?;
            let after = app
                .vault
                .index()
                .entry(&id)
                .and_then(|e| e.doc.roles().event.clone());
            let attendees = |e: Option<Event>| e.map(|e| e.attendees).unwrap_or_default();
            if attendees(before) != attendees(after) {
                counts.updated += 1;
            }
        }
        Effect::SetExceptions { path, exceptions } => {
            // Pulled only: applied even if the event changed meanwhile.
            let Some(&(id, _)) = sync.events.get(&path) else {
                return Ok(Applied::Skipped);
            };
            if set_exceptions(app, id, &exceptions)? {
                counts.updated += 1;
            }
        }
        Effect::CancelTrashed { path, etag } => {
            let Some(&(id, _)) = sync.events.get(&path) else {
                return Ok(Applied::Skipped);
            };
            cancel_trashed(app, account, id, &path, etag.as_deref())?;
        }
        Effect::RestoreLocal {
            path,
            remote,
            remote_calendar_id,
            exceptions,
        } => {
            let Some(&(id, _)) = sync.events.get(&path) else {
                return Ok(Applied::Skipped);
            };
            let back = Back {
                remote: &remote,
                instances: &exceptions,
                remote_calendar_id: &remote_calendar_id,
            };
            match restore_event(app, sync, id, &path, back)? {
                true => counts.restored += 1,
                false => counts.created += 1,
            }
        }
        Effect::SetSyncToken {
            token, time_min, ..
        } => {
            let result = app.edit(|index, changes| {
                index.edit(&sync.calendar, changes, |doc| {
                    set_bookkeeping(doc, account, |r| {
                        r.set(SYNC_TOKEN, &token);
                        r.set(TIME_MIN, &time_min);
                    });
                })
            });
            app.vault.require_written(result?)?;
        }
    }
    Ok(Applied::Done)
}

// ----------------------------------------------------------------------------
/// The id of the event at `path` -- if it is still what the job planned
/// with (the user didn't edit it while the sync ran).
fn unchanged_event(app: &App, sync: &CalendarSync, path: &Path) -> Option<Id> {
    let (id, planned) = sync.events.get(path)?;
    let doc = &app.vault.index().entry(id)?.doc;
    (calendar_sync::snapshot(&payload_of(doc)) == *planned).then_some(*id)
}

// ----------------------------------------------------------------------------
/// Change `doc`'s `remote:` block from `account` (a fresh one if it has
/// none from it, e.g. an event made in the vault and pushed).
fn set_bookkeeping(doc: &mut Document, account: Id, change: impl FnOnce(&mut Remote)) {
    let mut remote = doc.remote_from(Some(&account)).cloned().unwrap_or(Remote {
        account: Some(account),
        remote: Vec::new(),
    });
    change(&mut remote);
    doc.set_source(Some(Source::Remote(remote)));
}

// ----------------------------------------------------------------------------
/// A new event entity for `remote` (with Google's `instances` of it, if it
/// is a series) in the calendar's events folder.
fn create_event(
    app: &mut App,
    sync: &CalendarSync,
    remote: &GoogleEvent,
    instances: &[GoogleEvent],
    remote_calendar_id: &str,
) -> Result<()> {
    let mut doc = event_entity(remote, sync.calendar, sync.account, remote_calendar_id);
    take_exceptions(&mut doc, instances);
    let folder = events_folder(app, sync.calendar);
    app.edit(|index, changes| index.create(doc, &folder, changes))?;
    Ok(())
}

// ----------------------------------------------------------------------------
/// The Google event `remote` (of Google's calendar `remote_calendar_id`) as
/// a new event entity of the calendar entity `calendar`: its synced fields,
/// its attendees, and the bookkeeping of `account`, whose snapshot is what
/// the next run compares with.
fn event_entity(
    remote: &GoogleEvent,
    calendar: Id,
    account: Id,
    remote_calendar_id: &str,
) -> Document {
    let fields = from_google(remote);
    let snapshot = calendar_sync::snapshot(&to_google(&fields));
    let roles = Roles {
        event: Some(Event {
            calendar: Some(calendar),
            ..Event::default()
        }),
        ..Default::default()
    };
    let mut doc = Document::create(
        roles,
        Source::Created(Empty {}),
        String::new(),
        Vec::new(),
        FileFormat::Markdown,
    );
    apply_to(&fields, &mut doc);
    take_attendees(&mut doc, remote);
    set_bookkeeping(&mut doc, account, |r| {
        r.set(EVENT_ID, remote.id.as_deref().unwrap_or_default());
        r.set(CALENDAR_ID, remote_calendar_id);
        if let Some(etag) = &remote.etag {
            r.set(ETAG, etag);
        }
        r.set(SNAPSHOT, &snapshot);
    });
    doc
}

// ----------------------------------------------------------------------------
/// Where the events of the calendar entity `calendar` go: a folder named
/// after it, beside it.
fn events_folder(app: &App, calendar: Id) -> PathBuf {
    let entry = app.vault.index().entry(&calendar);
    let beside = entry
        .and_then(|e| e.local.parent())
        .unwrap_or(Path::new(layout::CALENDAR));
    let title = entry.map_or("", |e| e.doc.title());
    beside.join(filename::folder_name(title))
}

// ----------------------------------------------------------------------------
/// Let the remote version win the event `id`: synced fields, attendees and
/// bookkeeping. Whether the file changed beyond the bookkeeping.
fn take_remote(
    app: &mut App,
    account: Id,
    id: Id,
    remote: &GoogleEvent,
    remote_calendar_id: &str,
) -> Result<bool> {
    let fields = from_google(remote);
    let snapshot = calendar_sync::snapshot(&to_google(&fields));
    let Some(entry) = app.vault.index().entry(&id) else {
        return Err(Error::NotFound);
    };
    let (body, event) = (
        entry.doc.content().to_string(),
        entry.doc.roles().event.clone(),
    );
    let title = entry.doc.title().to_string();

    let result = app.edit(|index, changes| {
        index.edit(&id, changes, |doc| {
            apply_to(&fields, doc);
            take_attendees(doc, remote);
            set_bookkeeping(doc, account, |r| {
                if let Some(event_id) = &remote.id {
                    r.set(EVENT_ID, event_id);
                }
                r.set(CALENDAR_ID, remote_calendar_id);
                if let Some(etag) = &remote.etag {
                    r.set(ETAG, etag);
                }
                r.set(SNAPSHOT, &snapshot);
            });
        })
    });
    app.vault.require_written(result?)?;

    // A changed body has gone to the page with the entity (`App::edit`).
    let doc = app.vault.index().entry(&id).map(|e| &e.doc);
    let changed =
        doc.is_some_and(|d| d.content() != body || d.roles().event != event || d.title() != title);
    Ok(changed)
}

// ----------------------------------------------------------------------------
/// Mark the event `id` cancelled (deleted in Google), recording `etag`.
/// Whether it wasn't already.
fn cancel_event(app: &mut App, account: Id, id: Id, etag: Option<&str>) -> Result<bool> {
    let was_cancelled = app
        .vault
        .index()
        .entry(&id)
        .and_then(|e| e.doc.roles().event.as_ref()?.status.clone())
        .is_some_and(|s| s == CANCELLED);
    let result = app
        .edit(|index, changes| index.edit(&id, changes, |doc| mark_cancelled(doc, account, etag)));
    app.vault.require_written(result?)?;
    Ok(!was_cancelled)
}

// ----------------------------------------------------------------------------
/// Mark the event `doc` cancelled, recording `etag` and the snapshot that
/// makes it count as synced.
fn mark_cancelled(doc: &mut Document, account: Id, etag: Option<&str>) {
    let mut event = doc.roles().event.clone().unwrap_or_default();
    event.status = Some(CANCELLED.to_string());
    doc.set_event(Some(event));
    let snapshot = calendar_sync::snapshot(&payload_of(doc));
    set_bookkeeping(doc, account, |r| {
        if let Some(etag) = etag {
            r.set(ETAG, etag);
        }
        r.set(SNAPSHOT, &snapshot);
    });
}

// ----------------------------------------------------------------------------
/// Take Google's `instances` of the series `id` (`take_exceptions`).
/// Whether its file changed.
fn set_exceptions(app: &mut App, id: Id, instances: &[GoogleEvent]) -> Result<bool> {
    let outcome =
        app.edit(|index, changes| index.edit(&id, changes, |doc| take_exceptions(doc, instances)))?;
    let saved = matches!(outcome, SaveOutcome::Saved);
    app.vault.require_written(outcome)?;
    Ok(saved)
}

// ----------------------------------------------------------------------------
/// The trashed file of the event `id`, at `path` under `.trash`: mark it
/// cancelled -- it is gone in Google too -- recording `etag`. A file that
/// left the trash meanwhile (restored, or deleted for good) or holds
/// another entity is left alone.
fn cancel_trashed(app: &App, account: Id, id: Id, path: &Path, etag: Option<&str>) -> Result<()> {
    let edited = app.vault.index().edit_trashed(in_trash(path), |doc| {
        if doc.id() == id {
            mark_cancelled(doc, account, etag);
        }
    });
    match edited {
        Ok(_) => Ok(()),
        Err(Error::FileNotFound { .. }) => {
            log::info!("{}: no longer in the trash", path.display());
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// ----------------------------------------------------------------------------
/// `path` (under `.trash`) relative to the trash.
fn in_trash(path: &Path) -> &Path {
    path.strip_prefix(layout::TRASH).unwrap_or(path)
}

// ----------------------------------------------------------------------------
/// What comes back of an event deleted in the vault (`restore_event`).
struct Back<'a> {
    remote: &'a GoogleEvent,
    instances: &'a [GoogleEvent],
    remote_calendar_id: &'a str,
}

// ----------------------------------------------------------------------------
/// The event `id`, deleted in the vault, changed in Google since: its file
/// at `path` (under `.trash`) is restored and takes Google's version, so it
/// keeps its id, labels, projects and links. If the restore is refused (a
/// file in its place, its id in the vault, the file gone), it comes back as
/// a new event, and its trashed copy is marked cancelled. Whether it was
/// restored.
fn restore_event(
    app: &mut App,
    sync: &CalendarSync,
    id: Id,
    path: &Path,
    back: Back,
) -> Result<bool> {
    let local = in_trash(path).to_path_buf();
    match app.edit(|index, changes| index.restore(&local, changes)) {
        Ok(restored) => {
            log::info!("{}: restored, changed in Google", path.display());
            take_remote(
                app,
                sync.account,
                restored,
                back.remote,
                back.remote_calendar_id,
            )?;
            if !back.instances.is_empty() {
                set_exceptions(app, restored, back.instances)?;
            }
            Ok(true)
        }
        Err(e) => {
            log::info!(
                "{}: not restored ({e}), back as a new event",
                path.display()
            );
            create_event(
                app,
                sync,
                back.remote,
                back.instances,
                back.remote_calendar_id,
            )?;
            cancel_trashed(app, sync.account, id, path, None)?;
            Ok(false)
        }
    }
}

// ----------------------------------------------------------------------------
/// Before a conflict lets the remote win: keep the local version of `id`
/// as a new entity beside it, made here and tagged `conflict`, without the
/// series' exceptions. It belongs to the same calendar, so a two-way sync
/// pushes it as a new event.
fn keep_local_version(app: &mut App, id: Id) -> Result<()> {
    // Exceptions belong to Google's series, not to the new event.
    let roles = |roles: &mut Roles| {
        if let Some(event) = &mut roles.event {
            event.exceptions.clear();
        }
    };
    documents::keep_conflict_copy(app, id, None, roles)?;
    Ok(())
}

// ----------------------------------------------------------------------------
/// The sync is done: finish the account's sync (`google::finish_sync`), and
/// end the job with its summary.
fn finish(app: &mut App, sync: CalendarSync, counts: &Counts) {
    google::finish_sync(app, sync.account, |_| {});
    let note = match app.job_stopping(sync.job_id) {
        true => format!("stopped · {}", counts.summary()),
        false => counts.summary(),
    };
    end_sync(app, sync.job_id, &note);
}

// ----------------------------------------------------------------------------
/// End the job with `note`, its summary or why it failed.
fn end_sync(app: &mut App, job_id: Id, note: &str) {
    log::info!("Calendar sync {job_id}: {note}");
    app.end_job(job_id, note);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::trashed;
    use crate::testing::{CONNECTED, DISCONNECTED, Harness, parse, shape, with_google};
    use av_google::calendar::Attendee;
    use av_google::calendar_sync::Report;
    use serde_json::Value;
    use std::fs;

    /// The accounts' access and checked calendar (`with_google`).
    const FULL: &str =
        "  scopes:\n    - calendar: full\n  calendars:\n  - id: cal1\n    name: Personal\n";
    /// The calendar entity of `cal1`, and an event synced into it.
    const CAL: &str = "01a0e1cc-a24c-7490-82fe-e75669985b50";
    const EV: &str = "01a0e1cc-a24c-7490-82fe-e75669985b51";

    // ------------------------------------------------------------------------
    fn calendar_file() -> (&'static str, String) {
        let text = format!(
            "id: {CAL}\ntitle: Personal\nremote:\n  account: {CONNECTED}\n  remote:\n  - calendar-id: cal1\ncalendar: {{}}\n"
        );
        ("Calendar/personal.cal.yaml", text)
    }

    // ------------------------------------------------------------------------
    fn event_file() -> (&'static str, String) {
        let text = format!(
            "---\nid: {EV}\ntitle: Dentist at 10\ndate: 2026-10-05T08:00:00Z\nremote:\n  account: {CONNECTED}\n  remote:\n  - event-id: abc\n  - calendar-id: cal1\n  - etag: '\"1\"'\nevent:\n  calendar: {CAL}\n  start: 2026-10-05T08:00:00Z\n---\nLocal body\n"
        );
        ("Calendar/Personal/dentist.evt.md", text)
    }

    // ------------------------------------------------------------------------
    /// A weekly series synced into `CAL`, its occurrence of Oct 12 moved to
    /// the 13th.
    fn series_file() -> (&'static str, String) {
        let text = format!(
            "---\nid: {EV}\ntitle: Dentist at 10\ndate: 2026-10-05T08:00:00Z\nremote:\n  account: {CONNECTED}\n  remote:\n  - event-id: abc\n  - calendar-id: cal1\nevent:\n  calendar: {CAL}\n  start: 2026-10-05T08:00:00Z\n  recurrence:\n  - RRULE:FREQ=WEEKLY;BYDAY=MO\n  exceptions:\n  - original: 2026-10-12T08:00:00Z\n    start: 2026-10-13T08:00:00Z\n    end: 2026-10-13T09:00:00Z\n---\nLocal body\n"
        );
        ("Calendar/Personal/dentist.evt.md", text)
    }

    // ------------------------------------------------------------------------
    fn connected() -> Id {
        Id::parse(CONNECTED).unwrap()
    }

    // ------------------------------------------------------------------------
    /// A sync of `CAL` that has started, planned with `events`; its job.
    fn begin(h: &mut Harness, events: HashMap<PathBuf, (Id, String)>) -> Id {
        let job_id = h.app.start_job(JobKind::CalendarSync, "Syncing".into(), 1);
        h.app.calendar.sync = Some(CalendarSync {
            job_id,
            account: connected(),
            calendar: Id::parse(CAL).unwrap(),
            events,
        });
        h.take();
        job_id
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_sync_is_refused_before_any_job() {
        let mut h = with_google("calendar-refused", FULL, &[]);
        let sync =
            |account: &str, calendar: &str| json!({ "account": account, "calendar": calendar });
        let unknown = Id::generate().to_string();
        let cases = [
            (sync(&unknown, "cal1"), "not_found"),
            (sync(DISCONNECTED, "cal1"), "not_connected"),
            (sync(CONNECTED, "cal2"), "invalid_argument"),
        ];
        for (args, code) in cases {
            let sent = h.call("calendar_sync", args.clone());
            assert_eq!(shape(&sent), ["reply"], "no job: {sent:?}");
            assert_eq!(sent[0]["err"]["code"], code, "{args}");
        }

        begin(&mut h, HashMap::new());
        let sent = h.call("calendar_sync", sync(CONNECTED, "cal1"));
        assert_eq!(shape(&sent), ["reply"], "no job: {sent:?}");
        assert_eq!(sent[0]["err"]["code"], "unsupported");
        assert_eq!(
            sent[0]["err"]["message"],
            "Another calendar sync is still running."
        );

        let email_only = "  scopes:\n    - email: read\n";
        let mut h = with_google("calendar-no-access", email_only, &[]);
        let sent = h.call("calendar_sync", sync(CONNECTED, "cal1"));
        assert_eq!(shape(&sent), ["reply"], "no job: {sent:?}");
        assert_eq!(
            sent[0]["err"]["message"],
            "The account has no calendar access."
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_sync_writes_its_events_and_finishes_the_accounts_sync() {
        let mut h = with_google("calendar-apply", FULL, &[calendar_file()]);
        let source = find_source(&h.app, connected(), "cal1").unwrap();
        let calendar = calendar_entity(&mut h.app, connected(), &source).unwrap();
        assert_eq!(calendar.to_string(), CAL, "found, not made again");

        let job_id = begin(&mut h, HashMap::new());
        let output = SyncOutput {
            effects: vec![
                Effect::CreateLocal {
                    calendar_path: PathBuf::new(),
                    local_calendar_id: CAL.into(),
                    remote: google_event(),
                    remote_calendar_id: "cal1".into(),
                    exceptions: vec![google_instance(
                        "2026-10-12T10:00:00+02:00",
                        "2026-10-13T10:00:00+02:00",
                        "2026-10-13T11:00:00+02:00",
                    )],
                },
                Effect::SetSyncToken {
                    calendar_path: PathBuf::new(),
                    token: "token-2".into(),
                    time_min: "2025-10-07T00:00:00Z".into(),
                },
            ],
            report: Report::default(),
        };
        apply(&mut h.app, job_id, Ok(output));
        h.run_queue();
        let sent = parse(&h.take());
        assert_eq!(
            shape(&sent),
            ["entity", "entity", "account", "job"],
            "{sent:?}"
        );
        let event = &sent[0]["data"];
        let path = Path::new(event["path"].as_str().unwrap());
        assert!(path.starts_with("Calendar/Personal"), "{path:?}");
        assert_eq!(event["title"], "Dentist");
        assert_eq!(event["event"]["calendar"], CAL);
        let moved = json!([{
            "original": "2026-10-12T08:00:00Z",
            "start": "2026-10-13T08:00:00Z",
            "end": "2026-10-13T09:00:00Z",
        }]);
        assert_eq!(event["event"]["exceptions"], moved);
        assert_eq!(sent[1]["data"]["id"], CAL);
        let text = fs::read_to_string(h.vault().join("Calendar/personal.cal.yaml")).unwrap();
        assert!(text.contains("sync-token: token-2"), "{text}");
        assert!(sent[2]["data"]["last_sync"].is_string());
        assert_eq!(sent[3]["data"]["state"], "done");
        assert_eq!(sent[3]["data"]["note"], "1 new");
        assert!(h.app.calendar.sync.is_none());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_conflict_keeps_the_vault_version_beside_googles() {
        let mut h = with_google("calendar-conflict", FULL, &[calendar_file(), event_file()]);
        let id = Id::parse(EV).unwrap();
        let entry = h.app.vault.index().entry(&id).unwrap();
        let path = entry.local.clone();
        let planned = calendar_sync::snapshot(&payload_of(&entry.doc));
        let job_id = begin(&mut h, HashMap::from([(path.clone(), (id, planned))]));
        let output = SyncOutput {
            effects: vec![Effect::Conflict {
                path,
                remote: Some(google_event()),
                remote_calendar_id: "cal1".into(),
            }],
            report: Report {
                conflicts: vec!["Dentist at 10".into()],
                ..Report::default()
            },
        };
        apply(&mut h.app, job_id, Ok(output));
        h.run_queue();
        let sent = parse(&h.take());

        let theirs = &h.app.vault.index().entry(&id).unwrap().doc;
        assert_eq!(theirs.title(), "Dentist");
        assert!(theirs.content().starts_with("Bring the **card**."));
        let copy = sent
            .iter()
            .filter(|m| m["ev"] == "entity")
            .find(|m| m["data"]["id"] != EV)
            .expect("the kept copy");
        assert_eq!(copy["data"]["tags"], json!(["conflict"]));
        let copy = Id::parse(copy["data"]["id"].as_str().unwrap()).unwrap();
        let ours = &h.app.vault.index().entry(&copy).unwrap().doc;
        assert_eq!(ours.title(), "Dentist at 10");
        assert!(ours.content().contains("Local body"));
        let job = sent.last().unwrap();
        assert_eq!(
            (&job["data"]["state"], &job["data"]["note"]),
            (&Value::from("done"), &Value::from("1 conflict"))
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_series_takes_googles_instances_and_its_conflict_copy_doesnt() {
        let mut h = with_google(
            "calendar-exceptions",
            FULL,
            &[calendar_file(), series_file()],
        );
        let id = Id::parse(EV).unwrap();
        let entry = h.app.vault.index().entry(&id).unwrap();
        let path = entry.local.clone();
        let planned = calendar_sync::snapshot(&payload_of(&entry.doc));
        let job_id = begin(&mut h, HashMap::from([(path.clone(), (id, planned))]));
        let as_series = |instance: GoogleEvent| GoogleEvent {
            summary: Some("Dentist at 10".into()),
            location: None,
            ..instance
        };
        let back = as_series(google_instance(
            "2026-10-12T10:00:00+02:00",
            "2026-10-12T10:00:00+02:00",
            "2026-10-12T11:00:00+02:00",
        ));
        let cancelled = GoogleEvent {
            status: Some(CANCELLED.into()),
            ..google_instance(
                "2026-10-19T10:00:00+02:00",
                "2026-10-19T10:00:00+02:00",
                "2026-10-19T11:00:00+02:00",
            )
        };
        let output = SyncOutput {
            effects: vec![Effect::SetExceptions {
                path,
                exceptions: vec![back, cancelled],
            }],
            report: Report::default(),
        };
        apply(&mut h.app, job_id, Ok(output));
        h.run_queue();
        let sent = parse(&h.take());
        assert_eq!(shape(&sent), ["entity", "account", "job"], "{sent:?}");
        let exceptions = json!([{ "original": "2026-10-19T08:00:00Z", "status": "cancelled" }]);
        assert_eq!(sent[0]["data"]["event"]["exceptions"], exceptions);
        assert_eq!(sent[2]["data"]["note"], "1 updated");

        keep_local_version(&mut h.app, id).unwrap();
        let sent = parse(&h.take());
        let copy = Id::parse(sent[0]["data"]["id"].as_str().unwrap()).unwrap();
        let copy = h
            .app
            .vault
            .index()
            .entry(&copy)
            .unwrap()
            .doc
            .roles()
            .event
            .clone();
        let copy = copy.unwrap();
        assert!(copy.exceptions.is_empty() && !copy.recurrence.is_empty());
    }

    // ------------------------------------------------------------------------
    /// An event file in the trash at `path`: synced from `account` as Google's
    /// `gone`, of the calendar entity `calendar`, with `status`.
    fn trashed_event(
        path: &'static str,
        account: &str,
        calendar: &str,
        status: &str,
    ) -> (&'static str, String) {
        let id = Id::generate();
        let text = format!(
            "---\nid: {id}\ntitle: Gone\ndate: 2026-10-05T08:00:00Z\nremote:\n  account: {account}\n  remote:\n  - event-id: gone\n  - calendar-id: cal1\nevent:\n  calendar: {calendar}\n  status: {status}\n  start: 2026-10-05T08:00:00Z\n---\n"
        );
        (path, text)
    }

    // ------------------------------------------------------------------------
    /// The job for `cal1`, two-way if `push`, and its planned events.
    fn planned(h: &Harness, push: bool) -> (SyncJob, PlannedEvents) {
        let source = find_source(&h.app, connected(), "cal1").unwrap();
        let calendar = Id::parse(CAL).unwrap();
        build_job(&h.app, connected(), calendar, &source, push)
    }

    // ------------------------------------------------------------------------
    #[test]
    fn two_way_jobs_take_the_calendars_deleted_events() {
        let other = Id::generate().to_string();
        let files = [
            calendar_file(),
            trashed_event(
                ".trash/Calendar/Personal/a.evt.md",
                CONNECTED,
                CAL,
                "confirmed",
            ),
            trashed_event(
                ".trash/Calendar/Personal/b.evt.md",
                CONNECTED,
                CAL,
                "cancelled",
            ),
            trashed_event(
                ".trash/Calendar/Personal/c.evt.md",
                DISCONNECTED,
                CAL,
                "confirmed",
            ),
            trashed_event(
                ".trash/Calendar/Other/d.evt.md",
                CONNECTED,
                &other,
                "confirmed",
            ),
        ];
        let h = with_google("calendar-trashed", FULL, &files);
        let (job, events) = planned(&h, true);
        let [l] = &job.calendars[0].events[..] else {
            panic!("one deleted event");
        };
        assert!(l.deleted && l.wants_push());
        assert_eq!(l.path, Path::new(".trash/Calendar/Personal/a.evt.md"));
        assert!(events.contains_key(&l.path));
        let (job, _) = planned(&h, false);
        assert!(job.calendars[0].events.is_empty(), "pull only: no deletes");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_deleted_event_changed_in_google_comes_back_from_the_trash() {
        let files = [calendar_file(), synced_event_file("calendar-back-0")];
        let mut h = with_google("calendar-back", FULL, &files);
        let id = Id::parse(EV).unwrap();
        h.call("set_tags", json!({ "id": EV, "tags": ["dentist"] }));
        h.call("delete_entity", json!({ "id": EV }));
        let come_back = |h: &mut Harness| {
            let (job, events) = planned(h, true);
            let path = job.calendars[0].events[0].path.clone();
            let job_id = begin(h, events);
            let output = SyncOutput {
                effects: vec![Effect::RestoreLocal {
                    path,
                    remote: google_event(),
                    remote_calendar_id: "cal1".into(),
                    exceptions: Vec::new(),
                }],
                report: Report::default(),
            };
            apply(&mut h.app, job_id, Ok(output));
            h.run_queue();
            parse(&h.take())
        };
        let sent = come_back(&mut h);
        assert_eq!(sent.last().unwrap()["data"]["note"], "1 restored");
        let doc = &h.app.vault.index().entry(&id).unwrap().doc;
        assert_eq!(doc.title(), "Dentist", "Google's version");
        assert_eq!(doc.tags(), ["dentist"], "its labels kept");
        let Some(Source::Remote(bookkeeping)) = doc.source() else {
            panic!("{:?}", doc.source());
        };
        assert_eq!(bookkeeping.get(ETAG), Some("\"1\""));
        assert!(!trashed(&h.vault(), event_file().0));

        // Its place taken: back as a new event, the trashed copy cancelled.
        h.call("delete_entity", json!({ "id": EV }));
        fs::write(h.vault().join(event_file().0), "taken\n").unwrap();
        let sent = come_back(&mut h);
        assert_eq!(sent.last().unwrap()["data"]["note"], "1 new");
        let file = h.vault().join(".trash").join(event_file().0);
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("status: cancelled"), "{text}");
        assert!(h.app.vault.index().entry(&id).is_none());
    }

    // ------------------------------------------------------------------------
    /// `event_file` as last synced: its snapshot recorded (worked out in a
    /// vault of its own, `name`).
    fn synced_event_file(name: &str) -> (&'static str, String) {
        let h = with_google(name, FULL, &[calendar_file(), event_file()]);
        let entry = h.app.vault.index().entry(&Id::parse(EV).unwrap()).unwrap();
        let snapshot = calendar_sync::snapshot(&payload_of(&entry.doc));
        let (path, text) = event_file();
        let recorded = format!("  - snapshot: {snapshot}\n  - etag:");
        (path, text.replace("  - etag:", &recorded))
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_first_sync_reaches_back_a_year() {
        let now = DateTime::from_unix_millis(1_790_000_000_000);
        assert_eq!(
            first_sync_start(now).as_unix(),
            now.as_unix() - 365 * 86_400
        );
    }

    fn timed(date_time: &str, time_zone: &str) -> Option<EventTime> {
        Some(EventTime {
            date_time: Some(date_time.into()),
            time_zone: Some(time_zone.into()),
            ..EventTime::default()
        })
    }

    fn google_event() -> GoogleEvent {
        GoogleEvent {
            id: Some("abc".into()),
            etag: Some("\"1\"".into()),
            status: Some("confirmed".into()),
            summary: Some(" Dentist ".into()),
            description: Some("Bring the <b>card</b>.<br>Arrive early.".into()),
            location: Some("Main Street 1".into()),
            start: timed("2026-10-05T10:00:00+02:00", "Europe/Berlin"),
            end: timed("2026-10-05T11:00:00+02:00", "Europe/Berlin"),
            recurrence: Some(vec!["RRULE:FREQ=WEEKLY;BYDAY=MO".into()]),
            attendees: Some(vec![
                Attendee {
                    email: Some("jan@example.com".into()),
                    display_name: Some("Jan Schmidt".into()),
                    response_status: Some("accepted".into()),
                    ..Attendee::default()
                },
                Attendee {
                    email: Some("kim@example.com".into()),
                    ..Attendee::default()
                },
                Attendee {
                    email: Some("room@resource.calendar.google.com".into()),
                    display_name: Some("Room 1".into()),
                    resource: Some(true),
                    ..Attendee::default()
                },
            ]),
            ..GoogleEvent::default()
        }
    }

    /// Google's instance of `google_event()` that it starts at `original`.
    fn google_instance(original: &str, start: &str, end: &str) -> GoogleEvent {
        GoogleEvent {
            id: Some(format!("abc_{original}")),
            recurring_event_id: Some("abc".into()),
            original_start_time: timed(original, "Europe/Berlin"),
            start: timed(start, "Europe/Berlin"),
            end: timed(end, "Europe/Berlin"),
            recurrence: None,
            ..google_event()
        }
    }

    #[test]
    fn instances_keep_what_differs_from_their_series() {
        let series = from_google(&google_event());
        let at = |text: &str| DateTime::from_iso8601(text).ok();
        let of = |instance: &GoogleEvent| exception_of(instance, &series).unwrap();
        let instance = |start, end| google_instance("2026-10-12T10:00:00+02:00", start, end);

        let same = instance("2026-10-12T10:00:00+02:00", "2026-10-12T11:00:00+02:00");
        let x = of(&same);
        assert_eq!(x.original, at("2026-10-12T08:00:00Z").unwrap());
        assert!(keeps_nothing(&x), "{x:?}");

        let moved = of(&instance(
            "2026-10-13T10:00:00+02:00",
            "2026-10-13T11:00:00+02:00",
        ));
        assert_eq!(
            (moved.start, moved.end),
            (at("2026-10-13T08:00:00Z"), at("2026-10-13T09:00:00Z"))
        );
        let longer = of(&instance(
            "2026-10-12T10:00:00+02:00",
            "2026-10-12T12:00:00+02:00",
        ));
        assert_eq!(longer.end, at("2026-10-12T10:00:00Z"));
        assert_eq!(longer.start, at("2026-10-12T08:00:00Z"));

        let retitled = of(&GoogleEvent {
            summary: Some(" Retro ".into()),
            location: Some("Room 2".into()),
            ..same.clone()
        });
        assert_eq!(retitled.title.as_deref(), Some("Retro"));
        assert_eq!(retitled.location.as_deref(), Some("Room 2"));
        assert_eq!((retitled.start, retitled.status.as_deref()), (None, None));

        let cancelled = of(&GoogleEvent {
            status: Some(CANCELLED.into()),
            summary: Some("Other".into()),
            ..same.clone()
        });
        assert_eq!(cancelled.status.as_deref(), Some(CANCELLED));
        assert_eq!((cancelled.start, cancelled.title), (None, None));

        let no_original = GoogleEvent {
            original_start_time: None,
            ..same
        };
        assert!(exception_of(&no_original, &series).is_none());
    }

    #[test]
    fn all_day_instances_keep_dates() {
        let day = |date: &str| {
            Some(EventTime {
                date: Some(date.into()),
                ..EventTime::default()
            })
        };
        let series = from_google(&GoogleEvent {
            start: day("2026-12-24"),
            end: day("2026-12-25"),
            ..GoogleEvent::default()
        });
        let moved = GoogleEvent {
            original_start_time: day("2026-12-24"),
            start: day("2026-12-26"),
            end: day("2026-12-27"),
            ..GoogleEvent::default()
        };
        let x = exception_of(&moved, &series).unwrap();
        assert_eq!(x.original.as_iso8601(), "2026-12-24");
        assert_eq!(
            x.start.map(|s| s.as_iso8601()).as_deref(),
            Some("2026-12-26")
        );
        let back = exception_to_google(&x);
        assert_eq!(back.start, day("2026-12-26"));
    }

    #[test]
    fn what_a_series_keeps_compares_equal_to_googles_instance() {
        let remote = google_event();
        let moved = google_instance(
            "2026-10-12T10:00:00+02:00",
            "2026-10-13T10:00:00+02:00",
            "2026-10-13T11:00:00+02:00",
        );
        let mut doc = event_entity(&remote, Id::generate(), connected(), "cal1");
        take_exceptions(&mut doc, std::slice::from_ref(&moved));
        let reloaded = Document::load(doc.save(), FileFormat::Markdown);
        let local = local_event(&reloaded, Path::new("x.evt.md"), connected());
        assert_eq!(local.exceptions, [exception(&moved, &local.payload)]);
        assert_eq!(local.snapshot, calendar_sync::snapshot(&normalize(&remote)));
    }

    #[test]
    fn google_events_map_to_utc_and_markdown() {
        let fields = from_google(&google_event());
        assert_eq!(fields.title, "Dentist");
        assert_eq!(fields.start.unwrap().as_iso8601(), "2026-10-05T08:00:00Z");
        assert_eq!(fields.end.unwrap().as_iso8601(), "2026-10-05T09:00:00Z");
        assert_eq!(fields.time_zone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(fields.body, "Bring the **card**.  \nArrive early.\n");

        let back = to_google(&fields);
        let start = back.start.unwrap();
        assert_eq!(start.date_time.as_deref(), Some("2026-10-05T08:00:00Z"));
        assert_eq!(start.time_zone.as_deref(), Some("Europe/Berlin"));
        assert!(back.description.unwrap().contains("Arrive early."));
        assert_eq!(back.recurrence.unwrap(), ["RRULE:FREQ=WEEKLY;BYDAY=MO"]);
        assert_eq!(back.id, None, "a pushable payload");
    }

    #[test]
    fn normalize_is_stable() {
        let once = normalize(&google_event());
        assert_eq!(normalize(&once), once);
        assert_eq!(
            calendar_sync::snapshot(&normalize(&once)),
            calendar_sync::snapshot(&once)
        );
    }

    #[test]
    fn markdown_bodies_survive_the_html_round_trip() {
        // What `html::to_markdown` keeps. Headings, code and tables come
        // back as plain text; emphasis across a hard break comes back
        // closed and opened again on each line.
        let bodies = [
            "One line.\n",
            "First paragraph.\n\nSecond paragraph.\n",
            "Line one\\\nline two\n",
            "A [link](https://example.com).\n",
            "A [**bold** link](https://example.com).\n",
            "**Bold** and *italic*, ***both***, ~~struck~~.\n",
            "- one\n- two\n",
            "1. one\n2. two\n",
            "3. three\n4. four\n",
            "- a\n  - b\n- c\n",
            "1. a\n   1. b\n2. c\n",
            "- loose\n\n- list\n",
            "> quoted\n",
        ];
        let mut unstable = Vec::new();
        for body in bodies {
            let fields = EventFields {
                body: body.to_string(),
                ..EventFields::default()
            };
            let pushed = to_google(&fields);
            if normalize(&pushed) != pushed {
                unstable.push((body, normalize(&pushed).description));
            }
        }
        assert!(unstable.is_empty(), "{unstable:#?}");
    }

    #[test]
    fn plain_descriptions_keep_their_lines() {
        let event = GoogleEvent {
            description: Some("Line one\nLine two\n\nNew paragraph with a < b".into()),
            ..GoogleEvent::default()
        };
        let body = from_google(&event).body;
        assert!(
            body.contains("Line one") && body.contains("New paragraph"),
            "{body}"
        );
        assert!(!looks_like_html("a < b and c > d"));
        assert!(looks_like_html("a<br>b"));
    }

    #[test]
    fn all_day_events_keep_dates() {
        let event = GoogleEvent {
            start: Some(EventTime {
                date: Some("2026-12-24".into()),
                ..EventTime::default()
            }),
            end: Some(EventTime {
                date: Some("2026-12-27".into()),
                ..EventTime::default()
            }),
            ..GoogleEvent::default()
        };
        let fields = from_google(&event);
        assert_eq!(fields.start.unwrap().time, None);
        assert_eq!(
            to_google(&fields).end.unwrap().date.as_deref(),
            Some("2026-12-27")
        );

        let no_end = EventFields {
            end: None,
            ..fields
        };
        let end = to_google(&no_end).end.unwrap();
        assert_eq!(end.date.as_deref(), Some("2026-12-25"), "a day long");
    }

    #[test]
    fn attendees_are_only_pulled() {
        let names: Vec<String> = attendees_of(&google_event())
            .into_iter()
            .map(|a| match a {
                ContactRef::Name(name) => name,
                ContactRef::Id(id) => id.to_string(),
            })
            .collect();
        assert_eq!(names, ["Jan Schmidt <jan@example.com>", "kim@example.com"]);
        assert_eq!(normalize(&google_event()).attendees, None, "never pushed");
    }

    #[test]
    fn a_google_event_becomes_an_event_entity_that_snapshots_like_it() {
        let remote = google_event();
        let calendar = Id::generate();
        let doc = event_entity(&remote, calendar, connected(), "cal1");
        assert_eq!(doc.title(), "Dentist");
        let event = doc.roles().event.clone().unwrap();
        assert_eq!(event.calendar, Some(calendar));
        assert_eq!(event.location.as_deref(), Some("Main Street 1"));
        assert_eq!(event.attendees.len(), 2, "the room left out");
        let Some(Source::Remote(bookkeeping)) = doc.source() else {
            panic!("{:?}", doc.source());
        };
        assert_eq!(bookkeeping.account, Some(connected()));
        assert_eq!(
            (
                bookkeeping.get(EVENT_ID),
                bookkeeping.get(CALENDAR_ID),
                bookkeeping.get(ETAG)
            ),
            (Some("abc"), Some("cal1"), Some("\"1\""))
        );

        // What it stores hashes like the normalized remote, so the next run
        // sees it unchanged on both sides.
        let reloaded = Document::load(doc.save(), FileFormat::Markdown);
        let snapshot = calendar_sync::snapshot(&payload_of(&reloaded));
        assert_eq!(snapshot, calendar_sync::snapshot(&normalize(&remote)));
        assert_eq!(bookkeeping.get(SNAPSHOT), Some(snapshot.as_str()));
        assert_eq!(reloaded.date(), from_google(&remote).start);
    }

    #[test]
    fn summaries_name_what_happened() {
        assert_eq!(Counts::default().summary(), "nothing new");
        let counts = Counts {
            created: 3,
            deleted: 1,
            restored: 2,
            conflicts: 1,
            skipped: 2,
            ..Counts::default()
        };
        assert_eq!(
            counts.summary(),
            "3 new · 1 deleted · 2 restored · 1 conflict · 2 occurrences skipped"
        );
    }
}
