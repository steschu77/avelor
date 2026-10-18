//! Thin typed client for the Google Calendar API v3 — only the calls and
//! fields the sync uses. Calendar's own status-code meanings (409 id taken,
//! 410 sync token expired or event deleted, 412 etag mismatch) are
//! interpreted here and nowhere else.

use std::collections::BTreeMap;

use av_http::Method;
use av_utils::cancel::Cancel;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::url::encode_url_component;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::client::{Client, Page};
use crate::client::{Listed, execute, execute_empty};

// ----------------------------------------------------------------------------
const BASE: &str = "https://www.googleapis.com/calendar/v3";

// ----------------------------------------------------------------------------
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventTime {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attendee {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_status: Option<String>,
    /// A room or other resource rather than a person.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<bool>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReminderOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u64>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Reminders {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_default: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overrides: Option<Vec<ReminderOverride>>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtendedProperties {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private: Option<BTreeMap<String, String>>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<EventTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<EventTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring_event_id: Option<String>,
    /// For an instance of a recurring event: the start the recurring event gives it, which
    /// identifies it even after it was moved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_start_time: Option<EventTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attendees: Option<Vec<Attendee>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminders: Option<Reminders>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extended_properties: Option<ExtendedProperties>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

impl Event {
    // ------------------------------------------------------------------------
    /// The pushable payload: identity and server-owned fields stripped.
    /// This is also what the change-detection snapshot hashes — one code
    /// path for both, so they can never disagree.
    pub fn payload(&self) -> Event {
        let mut p = self.clone();
        p.id = None;
        p.etag = None;
        p.recurring_event_id = None;
        p.created = None;
        p.updated = None;
        p
    }
}

// ----------------------------------------------------------------------------
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CalendarListEntry {
    pub id: String,
    pub summary: String,
    /// The primary calendar's id is the account's email address.
    pub primary: bool,
    pub background_color: String,
    pub time_zone: String,
}

// ----------------------------------------------------------------------------
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarList {
    #[serde(default)]
    items: Vec<CalendarListEntry>,
    next_page_token: Option<String>,
}

impl Page for CalendarList {
    type Item = CalendarListEntry;
    fn into_parts(self) -> (Vec<Self::Item>, Option<String>, Option<String>) {
        (self.items, self.next_page_token, None)
    }
}

// ----------------------------------------------------------------------------
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventList {
    #[serde(default)]
    items: Vec<Event>,
    next_page_token: Option<String>,
    next_sync_token: Option<String>,
}

impl Page for EventList {
    type Item = Event;
    fn into_parts(self) -> (Vec<Self::Item>, Option<String>, Option<String>) {
        (self.items, self.next_page_token, self.next_sync_token)
    }
}

// ----------------------------------------------------------------------------
/// The result of a write that assumed a server state.
pub enum WriteOutcome {
    Written(Box<Event>),
    /// The server's state differs from what the write assumed: the event id
    /// is taken (insert), or the event changed since its etag (patch).
    Conflict,
}

// ----------------------------------------------------------------------------
/// The result of a delete that assumed a server state.
pub enum DeleteOutcome {
    Deleted,
    /// The event isn't there (anymore): deleted before.
    Gone,
    /// The event changed since the etag the delete assumed.
    Conflict,
}

// ----------------------------------------------------------------------------
fn events_url(calendar_id: &str) -> String {
    format!(
        "{BASE}/calendars/{}/events",
        encode_url_component(calendar_id)
    )
}

// ----------------------------------------------------------------------------
fn event_url(calendar_id: &str, event_id: &str) -> String {
    format!(
        "{}/{}",
        events_url(calendar_id),
        encode_url_component(event_id)
    )
}

// ----------------------------------------------------------------------------
/// `Conflict` for the given status, everything else as is.
fn write_outcome(result: Result<Event>, conflict_status: u16) -> Result<WriteOutcome> {
    match result {
        Ok(event) => Ok(WriteOutcome::Written(Box::new(event))),
        Err(Error::Http { status, .. }) if status == conflict_status => Ok(WriteOutcome::Conflict),
        Err(e) => Err(e),
    }
}

// ----------------------------------------------------------------------------
/// All calendars of the account.
pub fn list_calendars(client: &Client, cancel: &Cancel) -> Result<Vec<CalendarListEntry>> {
    let url = format!("{BASE}/users/me/calendarList");
    let listed = client.list_all::<CalendarList>(cancel, &url, &[("maxResults", "250")])?;
    Ok(listed.items)
}

// ----------------------------------------------------------------------------
/// Walk events.list to exhaustion. With a sync token: the deltas since it
/// was issued; if Google rejects the token (410), a full list instead. A
/// full list starts at `time_min` and includes deleted events, so the sync
/// token it issues covers future deletions. Also whether it was a full
/// list.
pub fn list_events(
    client: &Client,
    cancel: &Cancel,
    calendar_id: &str,
    sync_token: Option<&str>,
    time_min: &str,
) -> Result<(Listed<Event>, bool)> {
    let url = events_url(calendar_id);
    let common = [("maxResults", "250"), ("showDeleted", "true")];
    if let Some(token) = sync_token {
        let query = [common[0], common[1], ("syncToken", token)];
        match client.list_all::<EventList>(cancel, &url, &query) {
            Err(Error::Http { status: 410, .. }) => {} // expired: full list below
            listed => return listed.map(|listed| (listed, false)),
        }
    }
    let query = [common[0], common[1], ("timeMin", time_min)];
    let listed = client.list_all::<EventList>(cancel, &url, &query)?;
    Ok((listed, true))
}

// ----------------------------------------------------------------------------
/// Create `event` under the id it carries; `Conflict` if that id is taken.
pub fn insert_event(
    client: &Client,
    cancel: &Cancel,
    calendar_id: &str,
    event: &Event,
) -> Result<WriteOutcome> {
    let req = client.request(cancel, Method::Post, &events_url(calendar_id))?;
    write_outcome(execute(req.json(event)?), 409)
}

// ----------------------------------------------------------------------------
/// Update the event to what `event` holds of the fields the sync keeps: what
/// Google has of them is read first, so the patch can name what the vault
/// dropped (`patch_body`); with an `etag`, only if the event hasn't changed
/// since (`Conflict` otherwise). A patch Google refuses is logged by its
/// shape, its keys without their values.
pub fn patch_event(
    client: &Client,
    cancel: &Cancel,
    calendar_id: &str,
    event_id: &str,
    etag: Option<&str>,
    event: &Event,
) -> Result<WriteOutcome> {
    let remote = get_event(client, cancel, calendar_id, event_id)?;
    let body = patch_body(event, &remote)?;
    let mut req = client
        .request(cancel, Method::Patch, &event_url(calendar_id, event_id))?
        .json(&body)?;
    if let Some(etag) = etag {
        req = req.header("If-Match", etag);
    }
    let outcome = write_outcome(execute(req), 412);
    if let Err(Error::Http { status: 400, .. }) = &outcome {
        log::info!("A patch Google refused named {}", shape(&body));
    }
    outcome
}

// ----------------------------------------------------------------------------
/// A patch keeps what it doesn't name, so beside `event` it names what
/// Google has (`remote`) and the vault dropped: a summary, a description or
/// a location as `null`, a schedule as an empty one, a tentative status as
/// `confirmed`; and where a date takes a time's place, or a time a date's,
/// the other member as `null`, a time's zone with it.
fn patch_body(event: &Event, remote: &Event) -> Result<Value> {
    let mut body = serde_json::to_value(event)?;
    let Value::Object(fields) = &mut body else {
        return Ok(body);
    };
    let dropped = [
        ("summary", remote.summary.is_some()),
        ("description", remote.description.is_some()),
        ("location", remote.location.is_some()),
    ];
    for (key, had) in dropped {
        if had {
            fields.entry(key).or_insert(Value::Null);
        }
    }
    if remote.recurrence.as_ref().is_some_and(|r| !r.is_empty()) {
        fields
            .entry("recurrence")
            .or_insert_with(|| Value::Array(Vec::new()));
    }
    if remote.status.as_deref() == Some("tentative") {
        fields
            .entry("status")
            .or_insert_with(|| Value::from("confirmed"));
    }
    for (key, was) in [("start", &remote.start), ("end", &remote.end)] {
        let (Some(Value::Object(time)), Some(was)) = (fields.get_mut(key), was) else {
            continue;
        };
        if time.contains_key("date") && was.date_time.is_some() {
            time.insert("dateTime".into(), Value::Null);
            time.insert("timeZone".into(), Value::Null);
        }
        if time.contains_key("dateTime") && was.date.is_some() {
            time.insert("date".into(), Value::Null);
        }
    }
    Ok(body)
}

// ----------------------------------------------------------------------------
/// The keys of a patch's `body`, its times' members in braces, `null` and an
/// empty list said, its values left out: `summary, start{date,
/// dateTime=null}`.
fn shape(body: &Value) -> String {
    let Value::Object(fields) = body else {
        return String::new();
    };
    let named = |(key, value): (&String, &Value)| match value {
        Value::Null => format!("{key}=null"),
        Value::Array(items) if items.is_empty() => format!("{key}=[]"),
        Value::Object(members) => {
            let members: Vec<String> = members.iter().map(|m| named_flat(m.0, m.1)).collect();
            format!("{key}{{{}}}", members.join(", "))
        }
        _ => key.clone(),
    };
    fields.iter().map(named).collect::<Vec<_>>().join(", ")
}

fn named_flat(key: &str, value: &Value) -> String {
    match value {
        Value::Null => format!("{key}=null"),
        _ => key.to_string(),
    }
}

// ----------------------------------------------------------------------------
/// Delete the event; with an `etag`, only if it hasn't changed since
/// (`Conflict` otherwise).
pub fn delete_event(
    client: &Client,
    cancel: &Cancel,
    calendar_id: &str,
    event_id: &str,
    etag: Option<&str>,
) -> Result<DeleteOutcome> {
    let mut req = client.request(cancel, Method::Delete, &event_url(calendar_id, event_id))?;
    if let Some(etag) = etag {
        req = req.header("If-Match", etag);
    }
    match execute_empty(req) {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(Error::Http {
            status: 404 | 410, ..
        }) => Ok(DeleteOutcome::Gone),
        Err(Error::Http { status: 412, .. }) => Ok(DeleteOutcome::Conflict),
        Err(e) => Err(e),
    }
}

// ----------------------------------------------------------------------------
pub fn get_event(
    client: &Client,
    cancel: &Cancel,
    calendar_id: &str,
    event_id: &str,
) -> Result<Event> {
    client.get(cancel, &event_url(calendar_id, event_id), &[])
}

// ----------------------------------------------------------------------------
/// Move an event to another calendar (both must belong to the account).
pub fn move_event(
    client: &Client,
    cancel: &Cancel,
    from_calendar_id: &str,
    event_id: &str,
    to_calendar_id: &str,
) -> Result<Event> {
    let url = format!("{}/move", event_url(from_calendar_id, event_id));
    let req = client.request(cancel, Method::Post, &url)?;
    execute(req.query(&[("destination", to_calendar_id)]))
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ------------------------------------------------------------------------
    #[test]
    fn a_patch_drops_only_what_google_has() {
        let remote = Event {
            summary: Some("Dentist".into()),
            location: Some("Main St 1".into()),
            status: Some("tentative".into()),
            recurrence: Some(vec!["RRULE:FREQ=WEEKLY".into()]),
            start: Some(EventTime {
                date_time: Some("2027-01-12T09:00:00Z".into()),
                time_zone: Some("Europe/Berlin".into()),
                ..EventTime::default()
            }),
            ..Event::default()
        };
        let event = Event {
            summary: Some("Dentist".into()),
            start: remote.start.clone(),
            ..Event::default()
        };
        let body = patch_body(&event, &remote).unwrap();
        assert_eq!(body["location"], Value::Null);
        assert_eq!(body["recurrence"], json!([]));
        assert_eq!(body["status"], "confirmed");
        assert!(body.get("description").is_none(), "Google has none");
        assert_eq!(
            body["start"],
            json!({"dateTime": "2027-01-12T09:00:00Z", "timeZone": "Europe/Berlin"})
        );

        let single = Event {
            summary: Some("Dentist".into()),
            ..Event::default()
        };
        let body = patch_body(&single, &single).unwrap();
        assert_eq!(
            body,
            json!({"summary": "Dentist"}),
            "nothing dropped, nothing more"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_patch_turns_a_time_into_a_date_and_back() {
        let timed = Event {
            start: Some(EventTime {
                date_time: Some("2027-01-12T09:00:00Z".into()),
                time_zone: Some("Europe/Berlin".into()),
                ..EventTime::default()
            }),
            ..Event::default()
        };
        let day = Event {
            start: Some(EventTime {
                date: Some("2027-01-12".into()),
                ..EventTime::default()
            }),
            ..Event::default()
        };
        let body = patch_body(&day, &timed).unwrap();
        assert_eq!(
            body["start"],
            json!({"date": "2027-01-12", "dateTime": null, "timeZone": null})
        );
        let body = patch_body(&timed, &day).unwrap();
        assert_eq!(
            body["start"],
            json!({"date": null, "dateTime": "2027-01-12T09:00:00Z", "timeZone": "Europe/Berlin"})
        );
        assert_eq!(shape(&body), "start{date=null, dateTime, timeZone}");
    }
}
