//! Google's events and the vault's event entities, mapped both ways.

use av_google::calendar::{Event as GoogleEvent, EventTime};
use av_google::calendar_sync;
use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::{Empty, Event, Exception, Roles, Source};
use av_vault::shared::ContactRef;
use av_vault::{html, markdown, md_writer};

use super::CANCELLED;
use super::apply::set_bookkeeping;
use av_vault::schema::remote::{CALENDAR_ID, ETAG, EVENT_ID, SNAPSHOT};

/// What the sync keeps of an event, both ways: exactly what `to_google`
/// sends. Attendees are only pulled (`attendees_of`), and so are the
/// occurrences of a recurring event that differ from it (`exception_of`).
#[derive(Debug, Default, Clone, PartialEq)]
pub(super) struct EventFields {
    pub(super) title: String,
    pub(super) body: String,
    pub(super) start: Option<DateTime>,
    pub(super) end: Option<DateTime>,
    pub(super) time_zone: Option<String>,
    pub(super) location: Option<String>,
    pub(super) status: Option<String>,
    pub(super) schedule: Vec<String>,
}

// ----------------------------------------------------------------------------
/// A Google event as the vault stores it.
pub(super) fn from_google(event: &GoogleEvent) -> EventFields {
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
        schedule: event.recurrence.clone().unwrap_or_default(),
    }
}

// ----------------------------------------------------------------------------
/// What a push sends (and what change detection hashes): the synced fields
/// in Google's shape, the body as HTML. A local event without an end lasts
/// an hour, or a day.
pub(super) fn to_google(fields: &EventFields) -> GoogleEvent {
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
        recurrence: Some(fields.schedule.clone()).filter(|r| !r.is_empty()),
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
pub(super) fn normalize(event: &GoogleEvent) -> GoogleEvent {
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
        schedule: event.schedule,
    }
}

// ----------------------------------------------------------------------------
/// Write `fields` into `doc`; everything else (calendar, links, attendees,
/// projects, labels, …) stays.
pub(super) fn apply_to(fields: &EventFields, doc: &mut Document) {
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
    event.schedule = fields.schedule.clone();
    doc.set_event(Some(event));
}

// ----------------------------------------------------------------------------
/// The pushable payload of an event entity.
pub(super) fn payload_of(doc: &Document) -> GoogleEvent {
    to_google(&of_doc(doc))
}

// ----------------------------------------------------------------------------
/// A Google event's attendees as the vault keeps them: `Name <email>`, or
/// whichever of the two Google has; rooms and other resources left out.
pub(super) fn attendees_of(event: &GoogleEvent) -> Vec<ContactRef> {
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
pub(super) fn take_attendees(doc: &mut Document, remote: &GoogleEvent) {
    if remote.status.as_deref() == Some(CANCELLED) {
        return;
    }
    let mut event = doc.roles().event.clone().unwrap_or_default();
    event.attendees = attendees_of(remote);
    doc.set_event(Some(event));
}

// ----------------------------------------------------------------------------
/// What the vault keeps of Google's `instance` of the recurring event `recurring`: the
/// occurrence it replaces, and what differs from the recurring event -- cancelled;
/// moved or of another length (its start and end); another title or place.
/// Its description and guests aren't kept. None without an original start.
pub(super) fn exception_of(instance: &GoogleEvent, recurring: &EventFields) -> Option<Exception> {
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
    if start != original || (span.is_some() && span != span_of(recurring)) {
        exception.start = Some(start);
        exception.end = end;
    }
    let title = instance.summary.as_deref().unwrap_or_default().trim();
    exception.title = Some(title.to_string()).filter(|t| !t.is_empty() && *t != recurring.title);
    exception.location = instance
        .location
        .clone()
        .filter(|l| !l.is_empty() && recurring.location.as_ref() != Some(l));
    Some(exception)
}

// ----------------------------------------------------------------------------
/// The length of the occurrences of the recurring event `fields`, in seconds: an
/// hour, or a day, without an end.
fn span_of(fields: &EventFields) -> Option<i64> {
    let start = fields.start?;
    let end = fields.end.unwrap_or_else(|| default_end(start));
    Some(end.as_unix() - start.as_unix())
}

// ----------------------------------------------------------------------------
/// Whether `exception` keeps nothing but its original start: the
/// occurrence is as the recurring event has it.
pub(super) fn keeps_nothing(exception: &Exception) -> bool {
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
pub(super) fn exception_to_google(exception: &Exception) -> GoogleEvent {
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
/// of the recurring event whose payload is `recurring`.
pub(super) fn exception(instance: &GoogleEvent, recurring: &GoogleEvent) -> GoogleEvent {
    exception_of(instance, &from_google(recurring))
        .map(|x| exception_to_google(&x))
        .unwrap_or_default()
}

// ----------------------------------------------------------------------------
/// Take Google's `instances` of the recurring event `doc`, each relative to the
/// recurring event as it is now: it replaces what the vault kept of its occurrence,
/// or removes it if it differs from the recurring event in nothing kept.
pub(super) fn take_exceptions(doc: &mut Document, instances: &[GoogleEvent]) {
    let recurring = of_doc(doc);
    let mut event = doc.roles().event.clone().unwrap_or_default();
    for exception in instances.iter().filter_map(|i| exception_of(i, &recurring)) {
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
pub(super) fn looks_like_html(text: &str) -> bool {
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
    let end = start.plus_seconds(seconds);
    DateTime {
        time: start.time.and(end.time),
        ..end
    }
}

// ----------------------------------------------------------------------------
/// The Google event `remote` (of Google's calendar `remote_calendar_id`) as
/// a new event entity of the calendar entity `calendar`: its synced fields,
/// its attendees, and the bookkeeping of `account`, whose snapshot is what
/// the next run compares with.
pub(super) fn event_entity(
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
