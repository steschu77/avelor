use super::*;
use std::path::Path;

use av_google::calendar::{Event as GoogleEvent, EventTime};
use av_google::calendar_sync::Changes;
use av_google::calendar_sync::{Effect, SyncOutput};
use av_utils::error::Result;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::Source;
use av_vault::shared::ContactRef;

use super::apply::{Counts, apply, keep_local_version};
use super::auto_sync::{auto_sync_sources, on_auto_listed};
use super::mapping::{
    EventFields, attendees_of, event_entity, exception, exception_of, exception_to_google,
    from_google, keeps_nothing, looks_like_html, normalize, payload_of, take_exceptions, to_google,
};
use super::plan::{build_job, calendar_entity, first_sync_start, local_event};
use crate::testing::{CONNECTED, DISCONNECTED, Harness, parse, reply, shape, shown, with_google};
use crate::testing::{trashed, words};
use crate::texts::Text;
use av_google::calendar::Attendee;
use av_google::calendar_sync::Failure;
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
        source: "cal1".into(),
        calendar: Id::parse(CAL).unwrap(),
        events,
        auto_sync: false,
    });
    h.take();
    job_id
}

// ------------------------------------------------------------------------
/// The auto-sync of `cal1`, switched on.
fn switch_on(h: &mut Harness) -> SourceKey {
    let args = json!({ "account": CONNECTED, "calendar": "cal1", "enabled": true });
    let sent = h.call("set_calendar_sync", args);
    assert_eq!(reply(&sent)["ok"], Value::Null, "{sent:?}");
    (connected(), "cal1".to_string())
}

// ------------------------------------------------------------------------
fn listed(full: bool) -> Result<Changes> {
    Ok(Changes {
        deltas: Vec::new(),
        next_sync_token: Some("token-2".into()),
        time_min: "2025-10-07T00:00:00Z".into(),
        full,
    })
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
    build_job(&h.app, connected(), calendar, &source, push, None)
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
    h.call("set_labels", json!({ "id": EV, "labels": ["dentist"] }));
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
    assert_eq!(
        shown(&sent.last().unwrap()["data"]["summary"]),
        "1 restored"
    );
    let doc = &h.app.vault.index().entry(&id).unwrap().doc;
    assert_eq!(doc.title(), "Dentist", "Google's version");
    assert_eq!(doc.labels(), ["dentist"], "its labels kept");
    let Some(Source::Remote(bookkeeping)) = doc.source() else {
        panic!("{:?}", doc.source());
    };
    assert_eq!(bookkeeping.get(ETAG), Some("\"1\""));
    assert!(!trashed(&h.vault(), event_file().0));

    // Its place taken: back as a new event, the trashed copy cancelled.
    h.call("delete_entity", json!({ "id": EV }));
    fs::write(h.vault().join(event_file().0), "taken\n").unwrap();
    let sent = come_back(&mut h);
    assert_eq!(shown(&sent.last().unwrap()["data"]["summary"]), "1 new");
    let file = h.vault().join(".trash").join(event_file().0);
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("status: cancelled"), "{text}");
    assert!(h.app.vault.index().entry(&id).is_none());
}

// ------------------------------------------------------------------------
#[test]
fn a_calendars_auto_sync_is_switched_on_and_off() {
    let mut h = with_google("calendar-auto-set", FULL, &[]);
    let key = (connected(), "cal1".to_string());
    let soon = DateTime::now().plus_seconds(60);
    h.app.calendar.auto.next.insert(key.clone(), soon);
    let set =
        |enabled: bool| json!({ "account": CONNECTED, "calendar": "cal1", "enabled": enabled });
    let sent = h.call("set_calendar_sync", set(true));
    assert_eq!(shape(&sent), ["account", "reply"]);
    assert_eq!(sent[0]["data"]["calendars"][0]["auto_sync"], true);
    let next = &h.app.calendar.auto.next;
    assert!(!next.contains_key(&key), "due at the next tick");
    let file = h.vault().join(".avelor/accounts/google_a.yaml");
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("auto-sync: true"), "{text}");

    let sent = h.call("set_calendar_sync", set(false));
    assert_eq!(sent[0]["data"]["calendars"][0]["auto_sync"], false);
    assert!(!fs::read_to_string(&file).unwrap().contains("auto-sync"));

    let unknown = json!({ "account": Id::generate(), "calendar": "cal1", "enabled": true });
    let sent = h.call("set_calendar_sync", unknown);
    assert_eq!(sent[0]["err"]["code"], "not_found");
    let other = json!({ "account": CONNECTED, "calendar": "cal2", "enabled": true });
    let sent = h.call("set_calendar_sync", other);
    assert_eq!(sent[0]["err"]["code"], "invalid_argument");
}

// ------------------------------------------------------------------------
#[test]
fn auto_sync_waits_for_its_time_a_sync_and_the_vault_scan() {
    let mut h = with_google("calendar-auto-wait", FULL, &[calendar_file()]);
    let now = DateTime::now();
    auto_sync(&mut h.app, now);
    assert!(h.app.calendar.auto.listing.is_none(), "not switched on");

    // The disconnected account's calendar doesn't count.
    let key = switch_on(&mut h);
    let args = json!({ "account": DISCONNECTED, "calendar": "cal1", "enabled": true });
    h.call("set_calendar_sync", args);
    assert_eq!(auto_sync_sources(&h.app), std::slice::from_ref(&key));

    let soon = now.plus_seconds(60);
    h.app.calendar.auto.next.insert(key.clone(), soon);
    auto_sync(&mut h.app, now);
    assert!(h.app.calendar.auto.listing.is_none(), "not due yet");

    h.app.calendar.auto.next.clear();
    let job_id = begin(&mut h, HashMap::new());
    auto_sync(&mut h.app, now);
    assert!(h.app.calendar.auto.listing.is_none(), "a sync runs");
    apply(&mut h.app, job_id, Err(Error::Cancelled));

    // Opening the vault again: the indexing runs first.
    let root = h.vault();
    crate::api::switch_to(&mut h.app, root.clone());
    assert!(h.app.indexing());
    auto_sync(&mut h.app, now);
    assert!(h.app.calendar.auto.listing.is_none(), "the indexing runs");
    // Not due when the indexing ends, so no list goes out in this test.
    h.app.calendar.auto.next.insert(key, soon);
    h.wait_indexed();
    assert!(!h.app.indexing() && h.app.calendar.auto.listing.is_none());
}

// ------------------------------------------------------------------------
#[test]
fn a_listing_with_nothing_new_leaves_no_trace_and_a_failure_shows_once() {
    let synced = format!(
        "id: {CAL}\ntitle: Personal\nremote:\n  account: {CONNECTED}\n  remote:\n  - calendar-id: cal1\n  - sync-token: token-1\ncalendar: {{}}\n"
    );
    let file = "Calendar/personal.cal.yaml";
    let mut h = with_google("calendar-auto-listed", FULL, &[(file, synced)]);
    let key = switch_on(&mut h);
    h.take();
    let root = Some(h.vault());

    on_auto_listed(&mut h.app, key.clone(), root.clone(), listed(false));
    h.run_queue();
    assert!(h.take().is_empty(), "nothing new: no job, no write");
    assert!(h.app.calendar.sync.is_none());
    let text = fs::read_to_string(h.vault().join(file)).unwrap();
    assert!(text.contains("token-1"), "the token stays: {text}");

    let other = Some(h.vault().with_file_name("other"));
    on_auto_listed(&mut h.app, key.clone(), other, listed(true));
    assert!(h.app.calendar.sync.is_none(), "another vault: dropped");

    let error = || {
        Err(Error::Network {
            msg: "offline".into(),
        })
    };
    on_auto_listed(&mut h.app, key.clone(), root.clone(), error());
    on_auto_listed(&mut h.app, key.clone(), root.clone(), error());
    let sent = parse(&h.take());
    assert_eq!(shape(&sent), ["notice"], "shown once: {sent:?}");
    on_auto_listed(&mut h.app, key.clone(), root.clone(), listed(false));
    assert!(h.app.calendar.auto.failing.is_empty(), "working again");

    let manual = begin(&mut h, HashMap::new());
    on_auto_listed(&mut h.app, key.clone(), root, listed(true));
    let sync = h.app.calendar.sync.as_ref().unwrap();
    assert_eq!(sync.job_id, manual, "the running sync goes on");
    let next = h.app.calendar.auto.next[&key];
    assert!(next <= DateTime::now(), "again at the next tick");
}

// ------------------------------------------------------------------------
#[test]
fn an_auto_syncs_failures_are_shown_until_a_run_succeeds() {
    let mut h = with_google("calendar-auto-failing", FULL, &[calendar_file()]);
    let failed = || SyncOutput {
        effects: Vec::new(),
        report: Report {
            failures: vec![Failure {
                subject: "Dentist".into(),
                error: Error::Network {
                    msg: "offline".into(),
                },
            }],
            ..Report::default()
        },
    };
    let run = |h: &mut Harness, output: SyncOutput| {
        let job_id = begin(h, HashMap::new());
        h.app.calendar.sync.as_mut().unwrap().auto_sync = true;
        apply(&mut h.app, job_id, Ok(output));
        h.run_queue();
        let sent = parse(&h.take());
        sent.iter().filter(|m| m["ev"] == "notice").count()
    };
    assert_eq!(run(&mut h, failed()), 1);
    assert_eq!(run(&mut h, failed()), 0, "failing already");
    assert_eq!(run(&mut h, SyncOutput::default()), 0);
    assert!(h.app.calendar.auto.failing.is_empty());
    assert_eq!(run(&mut h, failed()), 1, "failing again");
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
