use std::collections::HashSet;

use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_vault::envelope::Source;
use av_vault::shared::OneOrMany;
use serde_json::json;

use super::apply::{Action, Stored, apply_email, fetch_action};
use super::entities::{Downloaded, email_entity};
use super::fetch::plan_fetch;
use crate::testing::{CONNECTED, DISCONNECTED, parse, shape, shown, with_google};
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
