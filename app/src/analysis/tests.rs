use super::*;
use av_vault::document::Document;
use av_vault::shared::DocType;
use serde_json::Value;

use super::answer::{Lite, TITLE_CHARS, parse};
use super::apply::apply_lite;
use super::context::{BODY_CHARS, Context, EmailText, context};
use super::request::{LITE_SHAPE, again, request_for};
use crate::ai;
use crate::ai::accounts::ModelChoice;
use crate::google::accounts::{Analysis, AnalysisLevel};
use crate::testing::{FakeOllama, Harness, parse as parse_sent, shown};
use crate::texts::Text;
use av_vault::document::FileFormat;
use av_vault::envelope::{Email, Roles, Source};
use av_vault::shared::OneOrMany;
use std::fs;
use std::sync::{Arc, Mutex};

const OLLAMA_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b60";
const EMAIL_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b61";
const EMAIL_2: &str = "01a0e1cc-a24c-7490-82fe-e75669985b62";
const FLAT: &str = "01a0e1cc-a24c-7490-82fe-e75669985b63";
const OLD: &str = "01a0e1cc-a24c-7490-82fe-e75669985b64";

// ------------------------------------------------------------------------
fn ctx() -> Context {
    Context {
        labels: vec![
            "Rechnungen".into(),
            "Umzug".into(),
            "Steuer".into(),
            "Bank".into(),
        ],
        projects: vec![
            (Id::parse(FLAT).unwrap(), "Wohnung Köln".into()),
            (Id::parse(OLD).unwrap(), "Auto".into()),
        ],
    }
}

// ------------------------------------------------------------------------
fn email_file(id: &str, subject: &str, labels: &str) -> String {
    format!(
        "---\nid: {id}\ntitle: '{subject}'\n{labels}email:\n  from: Stadtwerke <info@stadtwerke.example>\n---\nIhre Abschlagszahlung ist fällig.\n"
    )
}

// ------------------------------------------------------------------------
#[test]
fn the_context_holds_the_labels_in_use_and_the_active_projects() {
    let files = [
        (
            "Email/a.md",
            email_file(EMAIL_ID, "A", "tags: [Umzug, Rechnungen]\n"),
        ),
        (
            "Email/b.md",
            email_file(EMAIL_2, "B", "tags: [rechnungen]\n"),
        ),
        (
            "Projects/f.md",
            format!("---\nid: {FLAT}\ntitle: Wohnung\nproject: {{}}\n---\n"),
        ),
        (
            "Projects/o.md",
            format!("---\nid: {OLD}\ntitle: Altes Auto\nproject: {{}}\narchived: true\n---\n"),
        ),
    ];
    let h = Harness::with_vault("analysis-context", &files);
    let ctx = context(h.app.vault.index());
    assert_eq!(
        ctx.labels,
        ["Rechnungen", "Umzug"],
        "most used first, spelled as first seen"
    );
    assert_eq!(
        ctx.projects,
        [(Id::parse(FLAT).unwrap(), "Wohnung".to_string())]
    );
}

// ------------------------------------------------------------------------
#[test]
fn the_request_lists_the_choices_and_cuts_the_body() {
    let email = EmailText {
        id: Id::generate(),
        title: "Abschlag".into(),
        subject: "AW: [#4711] Abschlag".into(),
        from: "info@stadtwerke.example".into(),
        to: "jan@example.com".into(),
        date: None,
        body: "Body".into(),
    };
    let request = request_for(AnalysisLevel::Lite, "m", &ctx(), &email);
    assert_eq!(request.model, "m");
    let asked = again(request.clone(), "Sorry.", AnalysisLevel::Lite);
    assert_eq!(asked.messages.len(), 3);
    assert_eq!(asked.messages[1].role, av_llm::Role::Assistant);
    let Some(av_llm::Part::Text(reminder)) = asked.messages[2].parts.first() else {
        panic!("{:?}", asked.messages);
    };
    assert!(reminder.starts_with("That wasn't the JSON object"));
    assert!(reminder.contains(LITE_SHAPE), "{reminder}");
    assert_eq!(
        (request.think, request.context),
        (Some(false), Some(ai::CONTEXT_TOKENS))
    );
    let system = request.system.as_deref().unwrap();
    assert!(
        system.contains("- type:") && !system.contains("- check:"),
        "{system}"
    );
    let empty = again(request.clone(), "  ", AnalysisLevel::Lite);
    assert_eq!(
        empty.messages.len(),
        1,
        "an empty answer is just asked again"
    );
    assert!(
        request
            .system
            .as_deref()
            .unwrap()
            .contains("one JSON object")
    );
    let Some(av_llm::Part::Text(text)) = request.messages[0].parts.first() else {
        panic!("{:?}", request.messages);
    };
    assert!(text.contains("Labels:\n- Rechnungen\n- Umzug\n"), "{text}");
    assert!(
        text.contains("Projects:\n- Wohnung Köln\n- Auto\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "Subject: AW: [#4711] Abschlag\nFrom: info@stadtwerke.example\nTo: jan@example.com\n"
        ),
        "{text}"
    );
    let empty = request_for(AnalysisLevel::Lite, "m", &Context::default(), &email);
    let Some(av_llm::Part::Text(text)) = empty.messages[0].parts.first() else {
        panic!();
    };
    assert!(
        text.starts_with("Labels:\n(none)\n\nProjects:\n(none)\n"),
        "{text}"
    );

    let long = "ä".repeat(BODY_CHARS + 5);
    let mut doc = Document::create(
        Roles::default(),
        Source::Created(Default::default()),
        "T".into(),
        Vec::new(),
        FileFormat::Markdown,
    );
    doc.set_content(long);
    let text = EmailText::of(&doc);
    assert_eq!(text.body.chars().count(), BODY_CHARS + 4, "cut, marked");
    assert!(text.body.ends_with("\n[\u{2026}]"));
    assert_eq!((text.subject.as_str(), text.to.as_str()), ("T", ""));

    doc.set_email(Some(Email {
        subject: Some("AW: T".into()),
        to: OneOrMany::from_vec(vec!["a@example.com".into(), "b@example.com".into()]),
        ..Default::default()
    }));
    let text = EmailText::of(&doc);
    assert_eq!(
        (text.title.as_str(), text.subject.as_str(), text.to.as_str()),
        ("T", "AW: T", "a@example.com, b@example.com"),
        "the subject as the email came"
    );
}

// ------------------------------------------------------------------------
#[test]
fn the_answer_is_read_against_the_lists() {
    let answer = "Sure! ```json\n{\"title\": \"  Abschlag Oktober \", \"labels\": [\"rechnungen\", \"Unbekannt\", \"Rechnungen\", \"Umzug\", \"Steuer\", \"Bank\"], \"projects\": [\"wohnung köln\", \"Garten\", \"Auto\", \"Wohnung Köln\"]}\n``` Done.";
    let lite = parse(answer, AnalysisLevel::Lite, &ctx(), "AW: Abschlag").unwrap();
    assert_eq!(lite.title.as_deref(), Some("Abschlag Oktober"));
    assert_eq!(lite.doc_type, None);
    assert_eq!(
        lite.labels,
        ["Rechnungen", "Umzug", "Steuer"],
        "known, once, three"
    );
    let projects = [Id::parse(FLAT).unwrap(), Id::parse(OLD).unwrap()];
    assert_eq!(lite.projects, projects);

    let lite = |answer: &str, title: &str| parse(answer, AnalysisLevel::Lite, &ctx(), title);
    let same = lite(
        r#"{"type": " Notification", "title": " Abschlag ", "labels": "Bank"}"#,
        "Abschlag",
    );
    assert_eq!(
        same.unwrap(),
        Lite {
            title: None,
            doc_type: Some("notification".into()),
            labels: vec!["Bank".into()],
            projects: vec![],
        },
        "the email's own title is no change; one label as a string; the type in any case"
    );
    let unknown = lite(r#"{"type": "bill"}"#, "t").unwrap();
    assert_eq!(unknown.doc_type, None, "only an email's own types");
    let long = format!(r#"{{"title": "{}"}}"#, "x".repeat(200));
    let cut = lite(&long, "t").unwrap().title.unwrap();
    assert_eq!(cut.chars().count(), TITLE_CHARS);
    assert_eq!(lite(r#"{"title": "  "}"#, "t").unwrap().title, None);

    for bad in ["Sorry, I can't.", "{not json}", "[1, 2]", "} {"] {
        let refused = lite(bad, "t");
        assert_eq!(
            refused.err().map(|e| e.render()).as_deref(),
            Some("The answer wasn't the JSON object asked for."),
            "{bad}"
        );
    }
}

// ------------------------------------------------------------------------
#[test]
fn applying_keeps_the_subject_and_adds_to_what_the_email_has() {
    let roles = Roles {
        email: Some(Email::default()),
        ..Default::default()
    };
    let source = Source::Created(Default::default());
    let mut doc = Document::create(
        roles,
        source,
        "AW: Abschlag".into(),
        Vec::new(),
        FileFormat::Markdown,
    );
    doc.set_labels(vec!["umzug".into()]);
    let flat = Id::parse(FLAT).unwrap();
    doc.join_project(flat);
    let lite = Lite {
        title: Some("Abschlag Oktober".into()),
        doc_type: Some("notification".into()),
        labels: vec!["Umzug".into(), "Rechnungen".into()],
        projects: vec![flat, Id::parse(OLD).unwrap()],
    };
    apply_lite(&mut doc, &lite);
    let doc_type = |doc: &Document| doc.roles().document.clone().and_then(|d| d.doc_type);
    assert_eq!(doc.title(), "Abschlag Oktober");
    let email = doc.roles().email.clone().unwrap();
    assert_eq!(email.subject.as_deref(), Some("AW: Abschlag"));
    assert_eq!(doc_type(&doc), Some(DocType::parse("notification")));
    assert_eq!(doc.labels(), ["umzug", "Rechnungen"]);
    assert_eq!(doc.projects(), [flat, Id::parse(OLD).unwrap()]);

    let again = Lite {
        title: Some("Abschlag".into()),
        ..Lite::default()
    };
    apply_lite(&mut doc, &again);
    let email = doc.roles().email.clone().unwrap();
    assert_eq!(
        email.subject.as_deref(),
        Some("AW: Abschlag"),
        "the first subject stays"
    );
    assert_eq!(
        doc_type(&doc),
        Some(DocType::parse("notification")),
        "no type keeps it"
    );
}

// ------------------------------------------------------------------------
/// A vault with an Ollama account at `address` offering `m` (with
/// `extra` lines in its block), two emails, an email with labels and a
/// project.
fn with_emails(name: &str, address: &str, extra: &str) -> Harness {
    let account =
        format!("id: {OLLAMA_ID}\nollama:\n  address: {address}\n  offered: [m]\n{extra}");
    let files = [
        (".avelor/accounts/ollama.yaml", account),
        (
            "Email/a.md",
            email_file(EMAIL_ID, "AW: [#4711] Abschlag", ""),
        ),
        ("Email/b.md", email_file(EMAIL_2, "Abschlag", "")),
        (
            "Email/c.md",
            email_file(&Id::generate().to_string(), "C", "tags: [Rechnungen]\n"),
        ),
        (
            "Projects/f.md",
            format!("---\nid: {FLAT}\ntitle: Wohnung\nproject: {{}}\n---\n"),
        ),
    ];
    Harness::with_vault(name, &files)
}

// ------------------------------------------------------------------------
/// The analysis at `level` with `m` of the Ollama account.
fn at(level: AnalysisLevel) -> Analysis {
    let model = ModelChoice {
        account: Id::parse(OLLAMA_ID).unwrap(),
        model: "m".into(),
    };
    Analysis {
        level,
        model: Some(model),
    }
}

// ------------------------------------------------------------------------
/// What the page got until the job ended, parsed.
fn until_done(h: &mut Harness) -> Vec<Value> {
    let ended = |sent: &[String]| sent.iter().any(|m| m.contains(r#""state":"done""#));
    parse_sent(&h.wait_for(ended))
}

// ------------------------------------------------------------------------
fn of<'a>(sent: &'a [Value], ev: &str) -> Vec<&'a Value> {
    sent.iter()
        .filter(|m| m["ev"] == ev)
        .map(|m| &m["data"])
        .collect()
}

// ------------------------------------------------------------------------
#[test]
fn an_analysis_writes_the_title_labels_and_projects() {
    let answer =
        r#"{"title": "Abschlag Oktober", "labels": ["rechnungen"], "projects": ["Wohnung"]}"#;
    let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
    let mut h = with_emails("analysis-run", &address, "");
    let ids = vec![Id::parse(EMAIL_ID).unwrap()];
    analyse(&mut h.app, "jan@example.com", ids, &at(AnalysisLevel::Lite));
    let sent = until_done(&mut h);

    let jobs = of(&sent, "job");
    assert_eq!(
        shown(&jobs[0]["label"]),
        "Analysing email · jan@example.com"
    );
    assert_eq!(shown(&jobs.last().unwrap()["summary"]), "1 analysed");
    assert!(of(&sent, "notice").is_empty(), "{sent:?}");
    assert_eq!(of(&sent, "entity")[0]["title"], "Abschlag Oktober");
    let text = fs::read_to_string(h.vault().join("Email/a.md")).unwrap();
    assert!(text.contains("title: Abschlag Oktober"), "{text}");
    assert!(text.contains("subject: 'AW: [#4711] Abschlag'"), "{text}");
    assert!(
        text.contains("- Rechnungen") && text.contains(FLAT),
        "{text}"
    );
    assert!(
        !h.vault().join("Conversations").exists(),
        "Lite writes no conversation"
    );
}

// ------------------------------------------------------------------------
#[test]
fn an_unreadable_answer_is_asked_again() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let answers = ["Sorry.", r#"{"title": "Abschlag Oktober"}"#];
    let address = FakeOllama::new(&["m"])
        .answering_in_turn(&answers)
        .recording(&requests)
        .start();
    let mut h = with_emails("analysis-again", &address, "");
    analyse(
        &mut h.app,
        "jan@example.com",
        vec![Id::parse(EMAIL_ID).unwrap()],
        &at(AnalysisLevel::Lite),
    );
    let sent = until_done(&mut h);

    assert_eq!(
        shown(&of(&sent, "job").last().unwrap()["summary"]),
        "1 analysed"
    );
    assert!(of(&sent, "notice").is_empty(), "{sent:?}");
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let messages = requests[1]["messages"].as_array().unwrap();
    let roles: Vec<_> = messages
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["system", "user", "assistant", "user"]);
    assert_eq!(messages[2]["content"], "Sorry.");
    let reminder = messages[3]["content"].as_str().unwrap();
    assert!(
        reminder.starts_with("That wasn't the JSON object"),
        "{reminder}"
    );
}

// ------------------------------------------------------------------------
#[test]
fn a_cut_off_answer_is_not_asked_again() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let address = FakeOllama::new(&["m"])
        .replying(&[""])
        .cut_off()
        .recording(&requests)
        .start();
    let mut h = with_emails("analysis-cut", &address, "");
    let ids = vec![Id::parse(EMAIL_ID).unwrap()];
    analyse(&mut h.app, "jan@example.com", ids, &at(AnalysisLevel::Lite));
    let sent = until_done(&mut h);

    assert_eq!(
        shown(&of(&sent, "job").last().unwrap()["summary"]),
        "1 failed"
    );
    let message = shown(&of(&sent, "notice")[0]["text"]);
    let cut_off = Text::new("analysis.cut_off").render();
    assert!(message.ends_with(&cut_off), "{message}");
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "not asked again");
    assert_eq!(requests[0]["think"], false);
    assert_eq!(requests[0]["options"]["num_ctx"], ai::CONTEXT_TOKENS);
}

// ------------------------------------------------------------------------
#[test]
fn unreadable_answers_leave_the_mails_and_are_told_once() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let address = FakeOllama::new(&["m"])
        .replying(&["Sorry, I can't."])
        .recording(&requests)
        .start();
    let mut h = with_emails("analysis-bad", &address, "");
    let before = fs::read_to_string(h.vault().join("Email/a.md")).unwrap();
    let ids = vec![Id::parse(EMAIL_ID).unwrap(), Id::parse(EMAIL_2).unwrap()];
    analyse(&mut h.app, "jan@example.com", ids, &at(AnalysisLevel::Lite));
    let sent = until_done(&mut h);

    assert_eq!(
        shown(&of(&sent, "job").last().unwrap()["summary"]),
        "2 failed"
    );
    let notices = of(&sent, "notice");
    assert_eq!(notices.len(), 1, "{sent:?}");
    let message = shown(&notices[0]["text"]);
    assert!(
        message
            .starts_with("Email analysis · jan@example.com: 2 of 2 emails couldn't be analysed;"),
        "{message}"
    );
    assert!(
        message.contains("wasn't the JSON object asked for. (asked 3 times)"),
        "{message}"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        6,
        "each email asked three times"
    );
    assert_eq!(
        fs::read_to_string(h.vault().join("Email/a.md")).unwrap(),
        before
    );
}

// ------------------------------------------------------------------------
#[test]
fn the_spend_limit_stops_it_between_emails() {
    let answer = r#"{"title": "Abschlag Oktober"}"#;
    let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
    let mut h = with_emails("analysis-limit", &address, "");
    let ids = vec![Id::parse(EMAIL_ID).unwrap(), Id::parse(EMAIL_2).unwrap()];
    analyse(&mut h.app, "jan@example.com", ids, &at(AnalysisLevel::Lite));
    // The first email is on its way; then the limit is reached.
    let account = Id::parse(OLLAMA_ID).unwrap();
    h.app
        .ai
        .update(account, |s| s.spend_limit = Some(0.0))
        .unwrap();
    let sent = until_done(&mut h);

    let note = shown(&of(&sent, "job").last().unwrap()["summary"]);
    assert_eq!(note, "1 analysed · 1 left · spend limit reached");
    let notices = of(&sent, "notice");
    let message = shown(&notices[0]["text"]);
    assert!(
        message.starts_with("Email analysis · jan@example.com: Ollama has reached its spend limit"),
        "{message}"
    );
    let text = fs::read_to_string(h.vault().join("Email/b.md")).unwrap();
    assert!(
        text.contains("title: 'Abschlag'"),
        "the second email stays: {text}"
    );
}

// ------------------------------------------------------------------------
#[test]
fn a_refused_pick_is_told_and_starts_nothing() {
    let mut h = with_emails("analysis-no-pick", "http://127.0.0.1:9", "");
    let unpicked = Analysis {
        level: AnalysisLevel::Lite,
        model: None,
    };
    analyse(
        &mut h.app,
        "jan@example.com",
        vec![Id::parse(EMAIL_ID).unwrap()],
        &unpicked,
    );
    let sent = parse_sent(&h.take());
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0]["data"]["level"], "error");
    assert_eq!(
        shown(&sent[0]["data"]["text"]),
        "Email analysis · jan@example.com: For the email analysis, pick a model."
    );
    assert!(h.app.analysis.runs.is_empty());
}

// ------------------------------------------------------------------------
#[test]
fn stop_ends_it_and_a_late_answer_is_dropped() {
    let answer = r#"{"title": "Abschlag Oktober"}"#;
    let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
    let mut h = with_emails("analysis-stop", &address, "");
    let ids = vec![Id::parse(EMAIL_ID).unwrap(), Id::parse(EMAIL_2).unwrap()];
    analyse(&mut h.app, "jan@example.com", ids, &at(AnalysisLevel::Lite));
    let job = *h.app.analysis.runs.keys().next().unwrap();
    crate::jobs::stop(&mut h.app, job);
    let sent = parse_sent(&h.take());
    let last = shown(&of(&sent, "job").last().unwrap()["summary"]);
    assert_eq!(last, "stopped · 2 left");
    assert!(h.app.analysis.runs.is_empty());
    // Whatever the cancelled request brings back finds no run.
    h.run_queue();
    let text = fs::read_to_string(h.vault().join("Email/a.md")).unwrap();
    assert!(text.contains("title: 'AW: [#4711] Abschlag'"), "{text}");
}
