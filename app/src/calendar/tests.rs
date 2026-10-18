use super::*;
use std::path::Path;

use av_google::calendar::{Event as GoogleEvent, EventTime};
use av_google::calendar_sync::{Effect, SyncOutput};
use av_utils::datetime::DateTime;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::Source;
use av_vault::shared::ContactRef;

use super::apply::{Counts, apply, keep_local_version};
use super::mapping::{
    EventFields, event_entity, exception, exception_of, exception_to_google, from_google,
    guests_of, keeps_nothing, looks_like_html, normalize, payload_of, take_exceptions, to_google,
};
use super::plan::{calendar_entity, find_calendar, first_sync_start, local_event};
use crate::testing::{
    CONNECTED, DISCONNECTED, Harness, parse, reply, shape, shown, with_google, words,
};
use av_google::calendar::Attendee;
use av_google::calendar_sync::Report;
use av_vault::schema::remote::{CALENDAR_ID, ETAG, EVENT_ID, SNAPSHOT};
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
/// A weekly recurring event synced into `CAL`, its occurrence of Oct 12 moved to
/// the 13th.
fn recurring_file() -> (&'static str, String) {
    let text = format!(
        "---\nid: {EV}\ntitle: Dentist at 10\ndate: 2026-10-05T08:00:00Z\nremote:\n  account: {CONNECTED}\n  remote:\n  - event-id: abc\n  - calendar-id: cal1\nevent:\n  calendar: {CAL}\n  start: 2026-10-05T08:00:00Z\n  schedule:\n  - RRULE:FREQ=WEEKLY;BYDAY=MO\n  exceptions:\n  - original: 2026-10-12T08:00:00Z\n    start: 2026-10-13T08:00:00Z\n    end: 2026-10-13T09:00:00Z\n---\nLocal body\n"
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
    let job_id = h
        .app
        .start_job(JobKind::CalendarSync, Text::new("email.fetching"), 1);
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
    let sync = |account: &str, calendar: &str| json!({ "account": account, "calendar": calendar });
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
        shown(&sent[0]["err"]["text"]),
        "Another calendar sync is still running."
    );

    let email_only = "  scopes:\n    - email: read\n";
    let mut h = with_google("calendar-no-access", email_only, &[]);
    let sent = h.call("calendar_sync", sync(CONNECTED, "cal1"));
    assert_eq!(shape(&sent), ["reply"], "no job: {sent:?}");
    assert_eq!(
        shown(&sent[0]["err"]["text"]),
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
    assert_eq!(shown(&sent[3]["data"]["summary"]), "1 new");
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
    assert_eq!(copy["data"]["labels"], json!(["conflict"]));
    let copy = Id::parse(copy["data"]["id"].as_str().unwrap()).unwrap();
    let ours = &h.app.vault.index().entry(&copy).unwrap().doc;
    assert_eq!(ours.title(), "Dentist at 10");
    assert!(ours.content().contains("Local body"));
    let job = sent.last().unwrap();
    assert_eq!(job["data"]["state"], "done");
    assert_eq!(shown(&job["data"]["summary"]), "1 conflict");
}

// ------------------------------------------------------------------------
/// Apply `effect` as the sync of `CAL` planned with the event `id` at `path`.
fn sync_effect(h: &mut Harness, path: PathBuf, id: Id, effect: Effect) {
    let job_id = begin(h, HashMap::from([(path, (id, String::new()))]));
    let output = SyncOutput {
        effects: vec![effect],
        report: Report::default(),
    };
    apply(&mut h.app, job_id, Ok(output));
    h.run_queue();
    h.take();
}

// ------------------------------------------------------------------------
#[test]
fn an_edit_pushed_since_is_undone_keeping_the_push() {
    let mut h = with_google(
        "calendar-undo-pushed",
        FULL,
        &[calendar_file(), event_file()],
    );
    let id = Id::parse(EV).unwrap();
    h.call("set_title", json!({ "id": EV, "title": "Dentist at 11" }));
    let path = PathBuf::from(event_file().0);
    let pushed = Effect::SetRemote {
        path: path.clone(),
        remote_event_id: "abc".into(),
        etag: Some("\"2\"".into()),
        snapshot: "pushed".into(),
        remote_calendar_id: "cal1".into(),
        remote: Box::new(GoogleEvent {
            attendees: None,
            ..google_event()
        }),
    };
    sync_effect(&mut h, path.clone(), id, pushed);

    let sent = h.call("undo", json!({}));
    let notice = sent.iter().find(|m| m["ev"] == "notice").unwrap();
    assert_eq!(
        shown(&notice["data"]["text"]),
        "Undone: Renamed Dentist at 10"
    );
    let text = fs::read_to_string(h.vault().join(&path)).unwrap();
    assert!(text.contains("title: Dentist at 10"), "{text}");
    assert!(
        text.contains("etag: '\"2\"'") && text.contains("snapshot: pushed"),
        "{text}"
    );
}

// ------------------------------------------------------------------------
#[test]
fn a_delete_pushed_since_is_not_undone() {
    let mut h = with_google(
        "calendar-undo-deleted",
        FULL,
        &[calendar_file(), event_file()],
    );
    let id = Id::parse(EV).unwrap();
    h.call("delete_entity", json!({ "id": EV }));
    let trashed = h.app.vault.index().trashed();
    let path = Path::new(av_vault::schema::layout::TRASH).join(&trashed[0].local);
    let cancelled = Effect::CancelTrashed {
        path: path.clone(),
        etag: Some("\"2\"".into()),
    };
    sync_effect(&mut h, path, id, cancelled);

    let sent = h.call("undo", json!({}));
    let notice = sent.iter().find(|m| m["ev"] == "notice").unwrap();
    assert_eq!(notice["data"]["level"], "error");
    assert_eq!(
        shown(&notice["data"]["text"]),
        "Not undone, changed since: Deleted Dentist at 10"
    );
    assert!(h.app.vault.index().get(&id).is_none(), "still in the trash");
}

// ------------------------------------------------------------------------
#[test]
fn a_recurring_event_takes_googles_instances_and_its_conflict_copy_doesnt() {
    let mut h = with_google(
        "calendar-exceptions",
        FULL,
        &[calendar_file(), recurring_file()],
    );
    let id = Id::parse(EV).unwrap();
    let entry = h.app.vault.index().entry(&id).unwrap();
    let path = entry.local.clone();
    let planned = calendar_sync::snapshot(&payload_of(&entry.doc));
    let job_id = begin(&mut h, HashMap::from([(path.clone(), (id, planned))]));
    let as_recurring = |instance: GoogleEvent| GoogleEvent {
        summary: Some("Dentist at 10".into()),
        location: None,
        ..instance
    };
    let back = as_recurring(google_instance(
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
    assert_eq!(shown(&sent[2]["data"]["summary"]), "1 updated");

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
    assert!(copy.exceptions.is_empty() && !copy.schedule.is_empty());
}

// ------------------------------------------------------------------------
/// Check or uncheck the calendar `cal2`, Work, in the connected account.
fn check_work(h: &mut Harness, selected: bool) -> Vec<Value> {
    let calendar = json!({ "id": "cal2", "name": "Work", "color": null });
    let args = json!({ "account": CONNECTED, "calendar": calendar, "selected": selected });
    h.call("set_calendar_source", args)
}

// ------------------------------------------------------------------------
fn calendar_files(h: &Harness) -> usize {
    let is_calendar = |path: &&PathBuf| path.to_string_lossy().ends_with(".cal.yaml");
    h.files().keys().filter(is_calendar).count()
}

// ------------------------------------------------------------------------
#[test]
fn checking_a_calendar_makes_its_entity_once() {
    let mut h = with_google("calendar-check", FULL, &[calendar_file()]);
    let sent = check_work(&mut h, true);
    assert_eq!(shape(&sent), ["entity", "account", "reply"], "{sent:?}");
    let work = find_calendar(&h.app, connected(), "cal2").unwrap();
    let made = &sent[0]["data"];
    assert_eq!(made["id"], json!(work));
    assert_eq!(made["title"], "Work");
    assert!(Path::new(made["path"].as_str().unwrap()).starts_with("Calendar"));

    check_work(&mut h, true);
    check_work(&mut h, false);
    assert!(h.app.vault.index().get(&work).is_some(), "kept");
    check_work(&mut h, true);
    assert_eq!(find_calendar(&h.app, connected(), "cal2"), Some(work));
    assert_eq!(calendar_files(&h), 2);

    h.call("undo", json!({}));
    assert_eq!(calendar_files(&h), 2, "no step");
}

// ------------------------------------------------------------------------
#[test]
fn a_calendar_entity_that_cant_be_made_leaves_the_check() {
    let not_a_folder = ("Calendar", "a file".to_string());
    let access = "  scopes:\n    - calendar: full\n";
    let mut h = with_google("calendar-check-failed", access, &[not_a_folder]);
    let sent = check_work(&mut h, true);
    assert_eq!(shape(&sent), ["account", "reply"], "{sent:?}");
    assert_eq!(reply(&sent).get("ok"), Some(&Value::Null));
    assert!(find_source(&h.app, connected(), "cal2").is_some());
    assert!(find_calendar(&h.app, connected(), "cal2").is_none());
}

// ------------------------------------------------------------------------
#[test]
fn a_calendar_checked_while_the_vault_is_indexed_isnt_made_twice() {
    let access = "  scopes:\n    - calendar: full\n";
    let mut h = with_google("calendar-check-indexing", access, &[calendar_file()]);
    let root = h.vault();
    crate::api::switch_to(&mut h.app, root);
    assert!(h.app.indexing());
    let calendar = json!({ "id": "cal1", "name": "Personal", "color": null });
    let args = json!({ "account": CONNECTED, "calendar": calendar, "selected": true });
    h.call("set_calendar_source", args);
    h.wait_indexed();
    assert!(find_source(&h.app, connected(), "cal1").is_some());
    assert_eq!(calendar_files(&h), 1);
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
fn instances_keep_what_differs_from_their_recurring_event() {
    let recurring = from_google(&google_event());
    let at = |text: &str| DateTime::from_iso8601(text).ok();
    let of = |instance: &GoogleEvent| exception_of(instance, &recurring).unwrap();
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
    assert!(exception_of(&no_original, &recurring).is_none());
}

#[test]
fn all_day_instances_keep_dates() {
    let day = |date: &str| {
        Some(EventTime {
            date: Some(date.into()),
            ..EventTime::default()
        })
    };
    let recurring = from_google(&GoogleEvent {
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
    let x = exception_of(&moved, &recurring).unwrap();
    assert_eq!(x.original.as_iso8601(), "2026-12-24");
    assert_eq!(
        x.start.map(|s| s.as_iso8601()).as_deref(),
        Some("2026-12-26")
    );
    let back = exception_to_google(&x);
    assert_eq!(back.start, day("2026-12-26"));
}

#[test]
fn what_a_recurring_event_keeps_compares_equal_to_googles_instance() {
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
    assert_eq!(fields.tz.as_deref(), Some("Europe/Berlin"));
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
fn guests_are_only_pulled() {
    let names: Vec<String> = guests_of(&google_event())
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
    assert_eq!(event.guests.len(), 2, "the room left out");
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
    assert_eq!(words(&Counts::default().summary()), "nothing new");
    let counts = Counts {
        created: 3,
        deleted: 1,
        restored: 2,
        conflicts: 1,
        skipped: 2,
        ..Counts::default()
    };
    assert_eq!(
        words(&counts.summary()),
        "3 new · 1 deleted · 2 restored · 1 conflict · 2 occurrences skipped"
    );
}
