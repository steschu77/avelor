use super::*;
use std::collections::{HashSet, VecDeque};

use av_google::gmail::EmailFilter;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_vault::envelope::Source;
use av_vault::shared::OneOrMany;
use serde_json::json;

use super::apply::{Action, Stored, apply_email, fetch_action};
use super::auto_sync::{on_sync_listed, sync_query};
use super::entities::{Downloaded, email_entity};
use super::fetch::{FetchPlan, finish, new_email_notice, plan_fetch, start_fetch};
use crate::google::accounts::Analysis;
use crate::testing::Harness;
use crate::testing::{CONNECTED, DISCONNECTED, parse, reply, shape, shown, with_google};
use crate::texts::Text;
use av_vault::schema::remote::{GMAIL_ID, MESSAGE_ID};
use serde_json::Value;
use std::fs;

/// The accounts' access (`with_google`).
const EMAIL: &str = "  scopes:\n    - email: read\n";
const OTHER: &str = "01a0e1cc-a24c-7490-82fe-e75669985b26";

const RAW: &str = "From: \"Hausverwaltung Krause\" <info@krause.example>\r\n\
To: Jan <jan@example.com>, maya@example.org\r\n\
Cc: lisa@example.org\r\n\
Subject: Nebenkosten 2026\r\n\
Message-ID: <abc.123@krause.example>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/alternative; boundary=\"b1\"\r\n\
\r\n\
--b1\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hello Jan, the bill is here.\r\n\
--b1\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Hello Jan, the bill is <a href=\"https://krause.example/bill\">here</a>.</p>\r\n\
--b1--\r\n";

// ------------------------------------------------------------------------
fn downloaded(message_id: &str, body: &str) -> Downloaded {
    let date = DateTime::from_unix_millis(1_790_000_000_000);
    let mut email = Downloaded::parse(RAW.as_bytes(), date);
    email.message_id = message_id.to_string();
    email.body = body.to_string();
    email
}

// ------------------------------------------------------------------------
#[test]
fn a_raw_email_parses_into_its_parts() {
    let date = DateTime::from_unix_millis(1_790_000_000_000);
    let email = Downloaded::parse(RAW.as_bytes(), date);
    assert_eq!(email.subject, "Nebenkosten 2026");
    assert_eq!(
        email.from,
        "\"Hausverwaltung Krause\" <info@krause.example>"
    );
    assert_eq!(email.to, ["Jan <jan@example.com>", "maya@example.org"]);
    assert_eq!(email.cc, ["lisa@example.org"]);
    assert_eq!(email.message_id, "<abc.123@krause.example>");
    assert_eq!(email.date, date);
    assert!(
        email.body.contains("](https://krause.example/bill)"),
        "the HTML part, its link kept: {}",
        email.body
    );
}

// ------------------------------------------------------------------------
#[test]
fn a_email_becomes_an_email_entity() {
    let account = Id::parse(CONNECTED).unwrap();
    let doc = email_entity(account, "g1", downloaded("<m1@x>", "Body"));
    assert_eq!(doc.title(), "Nebenkosten 2026");
    assert_eq!(
        doc.date(),
        Some(DateTime::from_unix_millis(1_790_000_000_000))
    );
    assert_eq!(doc.content(), "Body");
    let email = doc.roles().email.as_ref().unwrap();
    assert_eq!(email.to.as_slice().len(), 2);
    assert_eq!(email.cc.as_slice(), ["lisa@example.org"]);
    assert_eq!(email.bcc, OneOrMany::None);
    let Some(Source::Remote(remote)) = doc.source() else {
        panic!("{:?}", doc.source());
    };
    assert_eq!(remote.account, Some(account));
    assert_eq!(remote.get(MESSAGE_ID), Some("<m1@x>"));
    assert_eq!(remote.get(GMAIL_ID), Some("g1"));

    let mut untitled = downloaded("", "Body");
    untitled.subject = "  ".to_string();
    untitled.cc.clear();
    let doc = email_entity(account, "g2", untitled);
    assert_eq!(doc.title(), "No subject");
    assert_eq!(doc.roles().email.as_ref().unwrap().cc, OneOrMany::None);
    let Some(Source::Remote(remote)) = doc.source() else {
        panic!("{:?}", doc.source());
    };
    assert_eq!(remote.get(MESSAGE_ID), None, "no Message-ID: none kept");
}

// ------------------------------------------------------------------------
#[test]
fn the_plan_skips_known_mails_unless_replaced_and_actions_follow() {
    let set = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<HashSet<_>>();
    let ids = ["a", "b", "c", "d"].map(String::from).to_vec();
    let (queue, skipped) = plan_fetch(ids, &set(&["c"]), &set(&["b", "c"]));
    assert_eq!(queue, ["a", "c", "d"]);
    assert_eq!(skipped, 1, "b is in the vault and not replaced");

    let id = Id::generate();
    assert_eq!(fetch_action(None, false), Action::Create);
    assert_eq!(fetch_action(None, true), Action::Create);
    assert_eq!(fetch_action(Some(id), true), Action::Replace(id));
    assert_eq!(fetch_action(Some(id), false), Action::Skip);
}

// ------------------------------------------------------------------------
#[test]
fn applying_creates_skips_replaces_and_matches_by_message_id() {
    let known = format!(
        "---\nid: 01a0e1cc-a24c-7490-82fe-e75669985b30\ntitle: Known\nremote:\n  account: {OTHER}\n  remote:\n  - message-id: <known@x>\nemail: {{}}\n---\nOld body\n"
    );
    let mut h = with_google("email-apply", EMAIL, &[("Email/known.md", known)]);
    let account = Id::parse(CONNECTED).unwrap();

    let stored = apply_email(
        &mut h.app,
        account,
        "g1",
        downloaded("<m1@x>", "First"),
        false,
    );
    assert!(matches!(stored.unwrap(), Stored::Created(_)));
    let sent = parse(&h.take());
    assert_eq!(shape(&sent), ["entity"]);
    let path = h.vault().join(sent[0]["data"]["path"].as_str().unwrap());
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("gmail-id: g1") && text.contains("message-id: <m1@x>"),
        "{text}"
    );

    let again = apply_email(
        &mut h.app,
        account,
        "g1",
        downloaded("<m1@x>", "Second"),
        false,
    );
    assert_eq!(again.unwrap(), Stored::Skipped);
    assert!(h.take().is_empty());

    let replaced = apply_email(
        &mut h.app,
        account,
        "g1",
        downloaded("<m1@x>", "Second"),
        true,
    );
    assert_eq!(replaced.unwrap(), Stored::Replaced);
    let sent = parse(&h.take());
    assert!(shape(&sent).contains(&"content"), "{sent:?}");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.trim_end().ends_with("Second"), "{text}");

    let by_message_id = apply_email(
        &mut h.app,
        account,
        "g9",
        downloaded("<known@x>", "x"),
        false,
    );
    assert_eq!(
        by_message_id.unwrap(),
        Stored::Skipped,
        "known from another account"
    );
}

// ------------------------------------------------------------------------
#[test]
fn requests_need_a_connected_account() {
    let mut h = with_google("email-accounts", EMAIL, &[]);
    let unknown = Id::generate();
    for account in [unknown.to_string(), DISCONNECTED.to_string()] {
        let requests = [
            ("email_labels", json!({ "account": account })),
            ("email_search", json!({ "account": account, "filter": {} })),
            (
                "email_headers",
                json!({ "account": account, "ids": ["g1"] }),
            ),
            ("email_fetch", json!({ "account": account, "ids": [] })),
        ];
        for (cmd, args) in requests {
            let sent = h.call(cmd, args);
            let code = sent[0]["err"]["code"].as_str().unwrap();
            let expected = match account == DISCONNECTED {
                true => "not_connected",
                false => "not_found",
            };
            assert_eq!(code, expected, "{cmd} {account}");
        }
    }
}

// ------------------------------------------------------------------------
#[test]
fn a_fetch_answers_its_job_and_finishes_the_accounts_sync() {
    let mut h = with_google("email-fetch", EMAIL, &[]);
    let fetch = |id: u64| {
        let filter = json!({ "starred": true, "since": "week" });
        let args =
            json!({ "account": CONNECTED, "ids": [], "filter": filter, "auto_update": false });
        json!({ "id": id, "cmd": "email_fetch", "args": args }).to_string()
    };
    h.app.on_web_message(fetch(1));
    h.app.on_web_message(fetch(2));
    h.run_queue();
    let sent = parse(&h.take());
    // The second request is answered after the first fetch's end.
    let shapes = shape(&sent);
    assert_eq!(
        shapes,
        ["job", "reply", "account", "job", "reply"],
        "{sent:?}"
    );
    let job = &sent[0]["data"]["id"];
    assert_eq!(sent[1]["ok"]["job"], *job);
    assert_eq!(sent[3]["data"]["state"], "done");
    assert_eq!(shown(&sent[3]["data"]["summary"]), "nothing to fetch");
    assert_eq!(sent[4]["err"]["code"], "unsupported", "one fetch at a time");
    assert_eq!(
        shown(&sent[4]["err"]["text"]),
        "Another fetch is still running."
    );

    let account = &sent[2]["data"];
    assert!(account["last_sync"].is_string(), "{account}");
    assert!(
        account["fetch_email"]["last_fetch"].is_string(),
        "{account}"
    );
    let file = h.vault().join(".avelor/accounts/google_a.yaml");
    let text = fs::read_to_string(file).unwrap();
    assert!(
        text.contains("last-sync:")
            && text.contains("last-fetch:")
            && text.contains("starred: true"),
        "{text}"
    );
    assert_eq!(account["fetch_email"]["auto_update"], Value::Bool(false));
}

// ------------------------------------------------------------------------
fn connected_id() -> Id {
    Id::parse(CONNECTED).unwrap()
}

// ------------------------------------------------------------------------
/// Switch the connected account's auto-sync on, with `filter`.
fn switch_on(h: &mut Harness, filter: Value) {
    let args = json!({ "account": CONNECTED, "enabled": true, "filter": filter });
    let sent = h.call("set_email_sync", args);
    assert_eq!(reply(&sent)["ok"], Value::Null, "{sent:?}");
}

// ------------------------------------------------------------------------
#[test]
fn an_auto_sync_asks_for_the_email_since() {
    let since = DateTime::from_unix_millis(1_790_000_000_000);
    let filter = EmailFilter {
        labels: vec!["Umzug".into()],
        starred: true,
        since: Some("week".into()),
        after: Some(DateTime::from_unix_millis(1_700_000_000_000)),
        before: Some(DateTime::from_unix_millis(1_800_000_000_000)),
        ..EmailFilter::default()
    };
    let expected = EmailFilter {
        labels: vec!["Umzug".into()],
        starred: true,
        after: Some(since),
        ..EmailFilter::default()
    };
    assert_eq!(sync_query(&filter, since), expected.to_query());
}

// ------------------------------------------------------------------------
#[test]
fn auto_sync_starts_from_now_and_switched_off_keeps_its_filter() {
    let mut h = with_google("email-sync-set", EMAIL, &[]);
    let before = DateTime::from_unix_millis(DateTime::now().as_unix() * 1000);
    let filter = json!({ "labels": ["Umzug"], "since": "week", "after": "2026-01-01T00:00:00Z" });
    let args = json!({ "account": CONNECTED, "enabled": true, "filter": filter });
    let sent = h.call("set_email_sync", args);
    assert_eq!(shape(&sent), ["account", "reply"]);
    let on = sent[0]["data"]["fetch_email"]["auto_sync"].clone();
    assert_eq!(on["enabled"], true);
    assert_eq!(on["filter"], json!({ "labels": ["Umzug"] }), "no dates");
    let since = DateTime::from_iso8601(on["since"].as_str().unwrap()).unwrap();
    assert!(since >= before, "from now: {since:?}");
    let file = h.vault().join(".avelor/accounts/google_a.yaml");
    let text = fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("auto-sync:") && text.contains("enabled: true"),
        "{text}"
    );

    let args = json!({ "account": CONNECTED, "enabled": false });
    let sent = h.call("set_email_sync", args);
    let off = &sent[0]["data"]["fetch_email"]["auto_sync"];
    assert_eq!(off["enabled"], false);
    assert_eq!(
        (&off["filter"], &off["since"]),
        (&on["filter"], &on["since"])
    );

    let old = DateTime::from_unix_millis(1_700_000_000_000);
    h.app
        .google
        .update(connected_id(), |account| {
            let email = account.file.google.fetch_email.as_mut().unwrap();
            email.auto_sync.as_mut().unwrap().since = old;
            true
        })
        .unwrap();
    let sent = h.call(
        "set_email_sync",
        json!({ "account": CONNECTED, "enabled": true }),
    );
    let again = &sent[0]["data"]["fetch_email"]["auto_sync"]["since"];
    let again = DateTime::from_iso8601(again.as_str().unwrap()).unwrap();
    assert!(again >= before, "switched on again: from now");

    let unknown = json!({ "account": Id::generate(), "enabled": true });
    assert_eq!(
        h.call("set_email_sync", unknown)[0]["err"]["code"],
        "not_found"
    );
}

// ------------------------------------------------------------------------
#[test]
fn auto_sync_waits_for_its_time_a_fetch_and_the_vault_scan() {
    let mut h = with_google("email-sync-wait", EMAIL, &[]);
    let now = DateTime::now();
    auto_sync(&mut h.app, now);
    assert!(h.app.email.auto.listing.is_none(), "not switched on");

    switch_on(&mut h, json!({}));
    let account = connected_id();
    h.app.email.auto.next.insert(account, now.plus_seconds(60));
    auto_sync(&mut h.app, now);
    assert!(h.app.email.auto.listing.is_none(), "not due yet");

    h.app.email.auto.next.clear();
    let client = h.app.google.connected(account).unwrap();
    let plan = FetchPlan {
        account,
        client,
        queue: VecDeque::new(),
        skipped: 0,
        replace: HashSet::new(),
        origin: Origin::Manual,
        started: now,
        analysis: None,
    };
    start_fetch(&mut h.app, Text::new("email.fetching"), plan);
    auto_sync(&mut h.app, now);
    assert!(h.app.email.auto.listing.is_none(), "a fetch runs");
    finish(&mut h.app);

    // Opening the vault again: the indexing runs first.
    let root = h.vault();
    crate::api::switch_to(&mut h.app, root.clone());
    assert!(h.app.indexing());
    auto_sync(&mut h.app, now);
    assert!(h.app.email.auto.listing.is_none(), "the indexing runs");
    // Not due when the indexing ends, so no search goes out in this test.
    h.app.email.auto.next.insert(account, now.plus_seconds(60));
    h.wait_indexed();
    assert!(!h.app.indexing() && h.app.email.auto.listing.is_none());
}

// ------------------------------------------------------------------------
#[test]
fn a_listing_with_nothing_new_leaves_no_trace_and_a_failure_shows_once() {
    let known = format!(
        "---\nid: 01a0e1cc-a24c-7490-82fe-e75669985b31\ntitle: Known\nremote:\n  account: {CONNECTED}\n  remote:\n  - gmail-id: g1\nemail: {{}}\n---\nBody\n"
    );
    let mut h = with_google("email-sync-listed", EMAIL, &[("Email/known.md", known)]);
    switch_on(&mut h, json!({}));
    h.take();
    let account = connected_id();
    let root = Some(h.vault());
    let now = DateTime::now();

    on_sync_listed(
        &mut h.app,
        account,
        root.clone(),
        now,
        Ok(vec!["g1".into()]),
    );
    h.run_queue();
    assert!(h.take().is_empty(), "nothing new: no job, no write");
    assert!(h.app.email.fetch.is_none());

    let other = Some(h.vault().with_file_name("other"));
    on_sync_listed(&mut h.app, account, other, now, Ok(vec!["g2".into()]));
    assert!(h.app.email.fetch.is_none(), "another vault: dropped");

    let error = || {
        Err(Error::Network {
            msg: "offline".into(),
        })
    };
    on_sync_listed(&mut h.app, account, root.clone(), now, error());
    on_sync_listed(&mut h.app, account, root.clone(), now, error());
    let sent = parse(&h.take());
    assert_eq!(shape(&sent), ["notice"], "shown once: {sent:?}");

    let client = h.app.google.connected(account).unwrap();
    let plan = FetchPlan {
        account,
        client,
        queue: VecDeque::new(),
        skipped: 0,
        replace: HashSet::new(),
        origin: Origin::Manual,
        started: now,
        analysis: None,
    };
    let manual = start_fetch(&mut h.app, Text::new("email.fetching"), plan);
    on_sync_listed(&mut h.app, account, root, now, Ok(vec!["g2".into()]));
    let fetch = h.app.email.fetch.as_ref().unwrap();
    assert_eq!(fetch.job_id, manual, "the manual fetch goes on");
    let next = h.app.email.auto.next[&account];
    assert!(next <= DateTime::now(), "again at the next tick");
}

// ------------------------------------------------------------------------
#[test]
fn a_complete_auto_sync_moves_its_start_on() {
    let mut h = with_google("email-sync-finish", EMAIL, &[]);
    switch_on(&mut h, json!({ "starred": true }));
    h.take();
    let account = connected_id();
    let started = DateTime::from_unix_millis(1_790_000_000_000);
    let client = h.app.google.connected(account).unwrap();
    let plan = FetchPlan {
        account,
        client,
        queue: VecDeque::new(),
        skipped: 0,
        replace: HashSet::new(),
        origin: Origin::AutoSync,
        started,
        analysis: None,
    };
    start_fetch(&mut h.app, Text::new("email.fetching"), plan);
    finish(&mut h.app);
    h.run_queue();
    let sent = parse(&h.take());
    assert_eq!(shape(&sent), ["job", "account", "job"], "{sent:?}");
    let account = &sent[1]["data"];
    assert_eq!(
        account["fetch_email"]["auto_sync"]["since"],
        started.as_iso8601()
    );
    assert_eq!(
        account["fetch_email"]["last_fetch"],
        Value::Null,
        "the page's stays"
    );
    assert!(account["last_sync"].is_string());
}

// ------------------------------------------------------------------------
#[test]
fn new_email_is_told_by_its_title_or_its_count() {
    let (a, b) = (Id::generate(), Id::generate());
    let title_of = |id: Id| (id == a).then(|| "Nebenkosten 2026".to_string());
    let told = |created: &[Id]| {
        let told = new_email_notice(created, title_of, "jan@example.com");
        told.map(|(text, entity)| (text.render(), entity))
    };
    assert_eq!(told(&[]), None);
    assert_eq!(
        told(&[a]),
        Some(("New email: Nebenkosten 2026".to_string(), Some(a)))
    );
    assert_eq!(
        told(&[b]),
        Some(("New email: No subject".to_string(), Some(b))),
        "gone meanwhile"
    );
    assert_eq!(
        told(&[a, b]),
        Some(("2 new emails · jan@example.com".to_string(), None))
    );
}

// ------------------------------------------------------------------------
#[test]
fn a_fetch_that_wrote_email_tells_the_page_after_its_end() {
    let known = format!(
        "---\nid: 01a0e1cc-a24c-7490-82fe-e75669985b32\ntitle: Nebenkosten 2026\nremote:\n  account: {CONNECTED}\n  remote:\n  - gmail-id: g1\nemail: {{}}\n---\nBody\n"
    );
    let mut h = with_google("email-new-notice", EMAIL, &[("Email/n.md", known)]);
    let account = connected_id();
    let email = Id::parse("01a0e1cc-a24c-7490-82fe-e75669985b32").unwrap();
    let client = h.app.google.connected(account).unwrap();
    let plan = FetchPlan {
        account,
        client,
        queue: VecDeque::new(),
        skipped: 0,
        replace: HashSet::new(),
        origin: Origin::Manual,
        started: DateTime::now(),
        analysis: None,
    };
    start_fetch(&mut h.app, Text::new("email.fetching"), plan);
    h.app
        .email
        .fetch
        .as_mut()
        .unwrap()
        .counts
        .created
        .push(email);
    finish(&mut h.app);
    let sent = parse(&h.take());
    assert_eq!(
        shape(&sent),
        ["job", "account", "job", "notice"],
        "{sent:?}"
    );
    assert_eq!(shown(&sent[2]["data"]["summary"]), "1 new");
    assert_eq!(
        (&sent[3]["data"]["level"], &sent[3]["data"]["entity"]),
        (&json!("info"), &json!(email))
    );
    assert_eq!(
        shown(&sent[3]["data"]["text"]),
        "New email: Nebenkosten 2026"
    );
}

// ------------------------------------------------------------------------
#[test]
fn a_fetch_with_the_analysis_on_needs_a_model_and_remembers_it() {
    let mut h = with_google("email-fetch-analysis", EMAIL, &[]);
    let model = json!({ "account": Id::generate(), "model": "m" });
    let fetch = |level: &str| {
        let analysis = json!({ "level": level, "model": model });
        json!({ "account": CONNECTED, "ids": [], "analysis": analysis })
    };
    let sent = h.call("email_fetch", fetch("lite"));
    assert_eq!(shape(&sent), ["reply"], "nothing fetched: {sent:?}");
    assert_eq!(sent[0]["err"]["code"], "invalid_argument");
    assert_eq!(
        shown(&sent[0]["err"]["text"]),
        "For the email analysis, the picked model\u{2019}s account is gone; pick another model."
    );
    let file = h.vault().join(".avelor/accounts/google_a.yaml");
    assert!(!fs::read_to_string(&file).unwrap().contains("analysis"));

    let sent = h.call("email_fetch", fetch("off"));
    assert!(sent.iter().any(|m| m["ok"]["job"].is_string()), "{sent:?}");
    h.run_queue();
    let text = fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("analysis:") && text.contains("level: off") && text.contains("model: m"),
        "off, the model kept: {text}"
    );
}

// ------------------------------------------------------------------------
#[test]
fn the_auto_syncs_analysis_is_stored_as_given() {
    let mut h = with_google("email-sync-analysis", EMAIL, &[]);
    let analysis = json!({ "level": "lite" });
    let args = json!({ "account": CONNECTED, "enabled": true, "analysis": analysis });
    let sent = h.call("set_email_sync", args);
    assert_eq!(shape(&sent), ["account", "reply"]);
    let sync = &sent[0]["data"]["fetch_email"]["auto_sync"];
    assert_eq!(sync["analysis"], json!({ "level": "lite" }), "no model yet");
    let file = h.vault().join(".avelor/accounts/google_a.yaml");
    let text = fs::read_to_string(file).unwrap();
    assert!(text.contains("level: lite"), "{text}");
}

// ------------------------------------------------------------------------
#[test]
fn a_finished_fetch_hands_its_new_mails_to_the_analysis() {
    let mut h = with_google("email-analyse-after", EMAIL, &[]);
    let account = connected_id();
    let lite = Analysis {
        level: crate::google::accounts::AnalysisLevel::Lite,
        model: None,
    };
    for stopped in [false, true] {
        let client = h.app.google.connected(account).unwrap();
        let plan = FetchPlan {
            account,
            client,
            queue: VecDeque::new(),
            skipped: 0,
            replace: HashSet::new(),
            origin: Origin::Manual,
            started: DateTime::now(),
            analysis: Some(lite.clone()),
        };
        let job = start_fetch(&mut h.app, Text::new("email.fetching"), plan);
        let fetch = h.app.email.fetch.as_mut().unwrap();
        fetch.counts.created.push(Id::generate());
        if stopped {
            crate::jobs::stop(&mut h.app, job);
        } else {
            finish(&mut h.app);
        }
        let sent = parse(&h.take());
        // No AI account in this vault: the analysis says so.
        let refused = sent.iter().any(|m| {
            let message = shown(&m["data"]["text"]);
            message.contains("For the email analysis, add an AI provider")
        });
        assert_eq!(refused, !stopped, "stopped: {stopped}, {sent:?}");
    }
}
