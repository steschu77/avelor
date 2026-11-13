//! One change to entities (`api.md#structure`'s `edit_entities`): a
//! document's genre, one of a task's fields, an event's fields, or a
//! calendar's. [`Index::edit_entities`] writes it to every entity named
//! that can take it.
//!
//! [`Index::edit_entities`]: crate::vault::Index::edit_entities

use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::patch::present;
use serde::Deserialize;

use crate::document::Document;
use crate::envelope::{DocumentType, Event};
use crate::shared::{ContactRef, Genre};

// ----------------------------------------------------------------------------
/// What part of a trip an event is (vault.md#event).
const KINDS: [&str; 4] = ["transport", "stay", "activity", "layover"];
/// An event's status as the app sets it: booked, or held.
const STATUSES: [&str; 2] = ["confirmed", "tentative"];

// ----------------------------------------------------------------------------
/// The change, as the page sends it: `{"genre": …}`, `{"task":
/// {"status" | "due" | "assignee": …}}`, `{"event": {…}}` or `{"calendar":
/// {"work-of": …}}`. A null or empty value removes the key; what can't be
/// written at all is refused by [`Change::check`].
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    /// A document's genre (vault.md#document).
    Genre(Option<String>),
    /// One of a task's fields (vault.md#task).
    Task(TaskChange),
    /// An event's fields (vault.md#event).
    Event(Box<EventChange>),
    /// A calendar's fields (vault.md#calendar).
    Calendar(CalendarChange),
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskChange {
    /// `done` also writes when it was marked done.
    Status(String),
    Due(Option<DateTime>),
    /// A contact of the vault (`@<uuid>`) or a name.
    Assignee(Option<String>),
}

// ----------------------------------------------------------------------------
/// An event's fields to change, by the block's own keys: each one given is
/// set, or removed by null or empty text; the others stay. A time comes
/// with its zone (a flight's Leaves, `start` and `timeZone`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct EventChange {
    #[serde(default, deserialize_with = "present")]
    pub kind: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub way: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub from: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub to: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub place: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub booking: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub work: Option<Option<String>>,
    /// Also the entity's `date`; it can't be removed.
    #[serde(default, deserialize_with = "present")]
    pub start: Option<Option<DateTime>>,
    #[serde(default, deserialize_with = "present")]
    pub end: Option<Option<DateTime>>,
    #[serde(rename = "timeZone", default, deserialize_with = "present")]
    pub time_zone: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub end_time_zone: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub location: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub status: Option<Option<String>>,
}

// ----------------------------------------------------------------------------
/// A calendar's fields to change: whose work calendar it is, a contact of
/// the vault (`@<uuid>`) or a name.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct CalendarChange {
    #[serde(default, deserialize_with = "present")]
    pub work_of: Option<Option<String>>,
}

impl EventChange {
    // ------------------------------------------------------------------------
    /// Whether it changes the event's times, which a recurring event keeps.
    fn moves(&self) -> bool {
        self.start.is_some()
            || self.end.is_some()
            || self.time_zone.is_some()
            || self.end_time_zone.is_some()
    }

    // ------------------------------------------------------------------------
    /// A status the app doesn't set, a kind that isn't one, or no start.
    fn check(&self) -> Result<()> {
        let invalid = |arg: &str| Err(Error::InvalidArgument { arg: arg.into() });
        let outside = |field: &Option<Option<String>>, values: &[&str]| {
            given(field).is_some_and(|value| !values.contains(&value))
        };
        if outside(&self.status, &STATUSES) {
            return invalid("status");
        }
        if outside(&self.kind, &KINDS) {
            return invalid("kind");
        }
        if self.start == Some(None) {
            return invalid("start");
        }
        Ok(())
    }

    // ------------------------------------------------------------------------
    fn apply(&self, event: &mut Event) {
        let texts = [
            (&mut event.kind, &self.kind),
            (&mut event.way, &self.way),
            (&mut event.from, &self.from),
            (&mut event.to, &self.to),
            (&mut event.place, &self.place),
            (&mut event.booking, &self.booking),
            (&mut event.work, &self.work),
            (&mut event.time_zone, &self.time_zone),
            (&mut event.end_time_zone, &self.end_time_zone),
            (&mut event.location, &self.location),
            (&mut event.status, &self.status),
        ];
        for (field, change) in texts {
            if let Some(value) = change {
                *field = text(value).map(str::to_string);
            }
        }
        if let Some(start) = self.start {
            event.start = start;
        }
        if let Some(end) = self.end {
            event.end = end;
        }
    }
}

impl Change {
    // ------------------------------------------------------------------------
    /// Whether the change can be written at all: a task's status can't be
    /// empty; an event's status is `confirmed` or `tentative`, its kind
    /// one of a trip's, and its start stays.
    pub fn check(&self) -> Result<()> {
        match self {
            Change::Task(TaskChange::Status(status)) if status.trim().is_empty() => {
                Err(Error::InvalidArgument {
                    arg: "status".into(),
                })
            }
            Change::Event(event) => event.check(),
            _ => Ok(()),
        }
    }

    // ------------------------------------------------------------------------
    /// Whether `doc` can take the change: a genre a document, a task's
    /// field a task, an event's an event (its times only if it isn't a
    /// recurring event), a calendar's a calendar.
    pub fn fits(&self, doc: &Document) -> bool {
        let kind = doc.kind();
        match self {
            Change::Genre(_) => kind == DocumentType::Document,
            Change::Task(_) => kind == DocumentType::Task,
            Change::Event(event) => {
                let roles = doc.roles();
                let recurring = roles.event.as_ref().is_some_and(|e| !e.schedule.is_empty());
                kind == DocumentType::Event && !(recurring && event.moves())
            }
            Change::Calendar(_) => kind == DocumentType::Calendar,
        }
    }

    // ------------------------------------------------------------------------
    /// Apply the change to `doc`, one that [`Change::fits`] it; `now` is
    /// when a task is marked done.
    pub fn apply(&self, doc: &mut Document, now: DateTime) {
        match self {
            Change::Genre(genre) => {
                doc.set_genre(text(genre).map(Genre::parse));
            }
            Change::Task(TaskChange::Status(status)) => {
                doc.set_task_status(status.trim().to_string(), now)
            }
            Change::Task(TaskChange::Due(due)) => doc.set_task_due(*due),
            Change::Task(TaskChange::Assignee(assignee)) => {
                doc.set_task_assignee(text(assignee).map(ContactRef::parse))
            }
            Change::Event(change) => {
                let mut event = doc.roles().event.clone().unwrap_or_default();
                change.apply(&mut event);
                if let Some(Some(start)) = change.start {
                    doc.set_date(start);
                }
                doc.set_event(Some(event));
            }
            Change::Calendar(change) => {
                let mut calendar = doc.roles().calendar.clone().unwrap_or_default();
                if let Some(work_of) = &change.work_of {
                    calendar.work_of = text(work_of).map(ContactRef::parse);
                }
                doc.set_calendar(Some(calendar));
            }
        }
    }
}

// ----------------------------------------------------------------------------
/// `value` trimmed; none if it is empty.
fn text(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

// ----------------------------------------------------------------------------
/// The text a patch's `field` sets, if it sets one.
fn given(field: &Option<Option<String>>) -> Option<&str> {
    field.as_ref().and_then(text)
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::FileFormat;
    use av_utils::id::Id;

    fn task(text: &str) -> Document {
        Document::load(format!("---\ntask:\n{text}---\n"), FileFormat::Markdown)
    }

    fn entity(yaml: &str) -> Document {
        Document::load(format!("---\n{yaml}---\n"), FileFormat::Markdown)
    }

    /// An event's change, its fields as the page would send them.
    fn event_change(yaml: &str) -> Change {
        Change::Event(serde_yaml_ng::from_str(yaml).unwrap())
    }

    fn calendar_change(yaml: &str) -> Change {
        Change::Calendar(serde_yaml_ng::from_str(yaml).unwrap())
    }

    fn date(iso: &str) -> DateTime {
        DateTime::from_iso8601(iso).unwrap()
    }

    const NOW: &str = "2026-10-12T08:30:00Z";

    // ------------------------------------------------------------------------
    #[test]
    fn an_empty_status_is_refused_and_the_rest_fits_its_kind() {
        let empty = Change::Task(TaskChange::Status(" ".into()));
        assert!(matches!(empty.check(), Err(Error::InvalidArgument { .. })));
        let genre = Change::Genre(Some(String::new()));
        assert!(genre.check().is_ok());
        let document = entity("document: {}\n");
        let todo = task("  status: todo\n");
        assert!(genre.fits(&document) && !genre.fits(&todo));
        let due = Change::Task(TaskChange::Due(None));
        assert!(due.fits(&todo) && !due.fits(&document));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_event_patch_sets_and_removes_its_fields_and_moves_its_date() {
        let mut doc = entity("event:\n  start: 2027-07-08\n  location: O'Hare\n");
        let held = event_change(
            "{kind: transport, way: flight, from: ' ORD ', to: HAM,
              status: tentative, location: null, booking: '',
              start: 2027-07-08T23:05:00Z, timeZone: America/Chicago,
              end: 2027-07-09T12:20:00Z, end-time-zone: Europe/Berlin}",
        );
        assert!(held.check().is_ok() && held.fits(&doc));
        held.apply(&mut doc, date(NOW));
        let event = doc.roles().event.clone().unwrap();
        assert_eq!(
            (
                event.kind.as_deref(),
                event.way.as_deref(),
                event.from.as_deref()
            ),
            (Some("transport"), Some("flight"), Some("ORD"))
        );
        assert_eq!((event.location, event.booking), (None, None));
        assert_eq!(event.status.as_deref(), Some("tentative"));
        assert_eq!(event.end_time_zone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(doc.date(), Some(date("2027-07-08T23:05:00Z")));

        let booked = event_change("{status: confirmed, booking: X7K2PL}");
        booked.apply(&mut doc, date(NOW));
        let event = doc.roles().event.clone().unwrap();
        assert_eq!(event.booking.as_deref(), Some("X7K2PL"));
        assert_eq!(event.way.as_deref(), Some("flight"), "the others stay");
        assert_eq!(doc.date(), Some(date("2027-07-08T23:05:00Z")));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_event_patch_refuses_what_the_app_doesnt_write() {
        let refused = [
            ("{status: cancelled}", "status"),
            ("{kind: day}", "kind"),
            ("{start: null}", "start"),
        ];
        for (yaml, arg) in refused {
            match event_change(yaml).check() {
                Err(Error::InvalidArgument { arg: got }) => assert_eq!(got, arg),
                other => panic!("{arg}: {other:?}"),
            }
        }
        let cleared = event_change("{status: null, kind: ''}");
        assert!(cleared.check().is_ok());
        assert!(event_change("{kind: layover}").check().is_ok());
        let unknown = serde_yaml_ng::from_str::<EventChange>("{title: x}");
        assert!(unknown.is_err());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_recurring_event_takes_a_trip_field_but_not_new_times() {
        let recurring =
            entity("event:\n  start: 2026-10-05T08:00:00Z\n  schedule: ['RRULE:FREQ=WEEKLY']\n");
        let kind = event_change("{kind: activity}");
        let moved = event_change("{start: 2026-10-06T08:00:00Z}");
        let zone = event_change("{timeZone: Europe/Berlin}");
        assert!(kind.fits(&recurring));
        assert!(!moved.fits(&recurring) && !zone.fits(&recurring));
        assert!(!kind.fits(&task("  status: todo\n")));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_calendar_is_someones_work_calendar_until_removed() {
        let contact = Id::generate();
        let mut doc = entity("calendar:\n  color: '#7986cb'\n");
        let work_of = calendar_change(&format!("{{work-of: '@{contact}'}}"));
        assert!(work_of.fits(&doc) && !work_of.fits(&task("  status: todo\n")));
        work_of.apply(&mut doc, date(NOW));
        let calendar = doc.roles().calendar.clone().unwrap();
        assert_eq!(calendar.work_of, Some(ContactRef::Id(contact)));
        assert!(calendar.color.is_some(), "the rest stays");
        calendar_change("{work-of: null}").apply(&mut doc, date(NOW));
        assert_eq!(doc.roles().calendar.clone().unwrap().work_of, None);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_due_date_is_set_and_removed_keeping_its_time() {
        let mut doc = task("  status: todo\n");
        let due = Some(date("2026-10-20T15:00:00Z"));
        Change::Task(TaskChange::Due(due)).apply(&mut doc, date(NOW));
        assert!(
            doc.save().contains("due: 2026-10-20T15:00:00Z"),
            "{}",
            doc.save()
        );
        Change::Task(TaskChange::Due(None)).apply(&mut doc, date(NOW));
        assert!(!doc.save().contains("due"), "{}", doc.save());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_assignee_is_a_contact_or_a_name_and_empty_removes_it() {
        let assign = |doc: &mut Document, to: &str| {
            Change::Task(TaskChange::Assignee(Some(to.into()))).apply(doc, date(NOW));
            doc.roles().task.as_ref().unwrap().assignee.clone()
        };
        let contact = Id::generate();
        let mut doc = task("  status: todo\n");
        assert_eq!(
            assign(&mut doc, &format!("@{contact}")),
            Some(ContactRef::Id(contact))
        );
        assert_eq!(
            assign(&mut doc, " Jane Doe "),
            Some(ContactRef::Name("Jane Doe".into()))
        );
        assert_eq!(assign(&mut doc, ""), None);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_genre_is_parsed_and_a_status_trimmed() {
        let mut doc = Document::load("---\ndocument: {}\n---\n".into(), FileFormat::Markdown);
        Change::Genre(Some(" Contract ".into())).apply(&mut doc, date(NOW));
        let genre = doc.roles().document.as_ref().unwrap().genre.clone();
        assert_eq!(genre, Some(Genre::Contract));

        let mut doc = task("  status: todo\n");
        Change::Task(TaskChange::Status(" done ".into())).apply(&mut doc, date(NOW));
        let task = doc.roles().task.clone().unwrap();
        assert_eq!(
            (task.status.as_deref(), task.done),
            (Some("done"), Some(date(NOW)))
        );
    }
}
