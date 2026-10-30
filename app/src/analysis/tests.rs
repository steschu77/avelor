use super::*;
use av_vault::document::Document;
use av_vault::envelope::DocumentType;
use av_vault::shared::DocType;
use serde_json::Value;

use super::answer::{
    CHECKS_PER_EMAIL, Lite, Medium, POINT_CHARS, TITLE_CHARS, parse, parse_document,
};
use super::apply::{apply_document, apply_lite, medium_messages};
use super::context::{BODY_CHARS, Context, DocText, EmailText, context};
use super::request::{
    DOCUMENT_MEDIUM_SHAPE, DOCUMENT_SHAPE, LITE_SHAPE, MEDIUM_SHAPE, again, document_request,
    request_for,
};
use crate::ai;
use crate::ai::accounts::ModelChoice;
use crate::testing::{FakeOllama, Harness, parse as parse_sent, shown};
use crate::texts::Text;
use av_vault::document::FileFormat;
use av_vault::envelope::{Email, Roles, Source};
use av_vault::shared::OneOrMany;
use serde_json::json;
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
        types: vec!["bill".into(), "letter".into(), "Steuerbescheid".into()],
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
        (
            "Documents/s.md",
            "---\ntitle: S\ndocument:\n  type: Steuerbescheid\n---\n".to_string(),
        ),
        (
            "Documents/b.md",
            "---\ntitle: B\ndocument:\n  type: BILL\n---\n".to_string(),
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
    assert!(ctx.types.starts_with(&["bill".to_string()]));
    assert!(!ctx.types.iter().any(|g| g == "note"), "never a note");
    assert_eq!(ctx.types.last().map(String::as_str), Some("Steuerbescheid"));
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
        attachments: Vec::new(),
    };
    let request = request_for(AnalysisLevel::Lite, "m", &ctx(), &email);
    assert_eq!(request.model, "m");
    let asked = again(request.clone(), "Sorry.", LITE_SHAPE);
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
    let medium = request_for(AnalysisLevel::Medium, "m", &ctx(), &email);
    let system = medium.system.as_deref().unwrap();
    assert!(system.contains(MEDIUM_SHAPE), "{system}");
    assert!(
        system.contains("- check:")
            && system.contains("- reply:")
            && system.contains("- questions:")
    );
    let asked = again(medium.clone(), "Sorry.", MEDIUM_SHAPE);
    let Some(av_llm::Part::Text(reminder)) = asked.messages[2].parts.first() else {
        panic!("{:?}", asked.messages);
    };
    assert!(reminder.contains(MEDIUM_SHAPE), "{reminder}");
    let empty = again(request.clone(), "  ", LITE_SHAPE);
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
/// The attachment `Rechnung.pdf` of the email "Abschlag", read to `body`,
/// dated `date`.
fn doc_text(body: &str, date: Option<DateTime>) -> DocText {
    DocText {
        id: Id::generate(),
        title: "Rechnung".into(),
        name: "Rechnung.pdf".into(),
        date,
        email: Some(("Abschlag".into(), "info@stadtwerke.example".into())),
        body: body.into(),
        keep_title: false,
    }
}

// ------------------------------------------------------------------------
#[test]
fn an_attachment_is_asked_for_as_a_document() {
    let request = document_request(
        AnalysisLevel::Medium,
        "m",
        &ctx(),
        &doc_text("Betrag 42 EUR", None),
    );
    let system = request.system.as_deref().unwrap();
    assert!(system.starts_with("You sort documents.") && system.contains(DOCUMENT_SHAPE));
    assert!(system.contains("- type:"), "{system}");
    assert_eq!(request.think, Some(false));
    let Some(av_llm::Part::Text(text)) = request.messages[0].parts.first() else {
        panic!("{:?}", request.messages);
    };
    assert!(
        text.starts_with("Types:\n- bill\n- letter\n- Steuerbescheid\n\nLabels:\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "The document:\nFile: Rechnung.pdf\nAttached to the email: Abschlag (from info@stadtwerke.example)\n\nBetrag 42 EUR"
        ),
        "{text}"
    );
    let asked = again(request, "Sorry.", DOCUMENT_SHAPE);
    let Some(av_llm::Part::Text(reminder)) = asked.messages[2].parts.first() else {
        panic!("{:?}", asked.messages);
    };
    assert!(reminder.contains(DOCUMENT_SHAPE), "{reminder}");
}

// ------------------------------------------------------------------------
#[test]
fn an_attachments_answer_takes_a_listed_type_and_its_own_date() {
    let email_date = DateTime::from_ymd_hms(2026, 10, 12, 9, 30, 0).unwrap();
    let doc = doc_text("", Some(email_date));
    let read = |answer: &str| {
        parse_document(answer, AnalysisLevel::Lite, &ctx(), &doc)
            .unwrap()
            .0
    };
    let lite = read(
        r#"{"title": "Stadtwerke Abschlag Oktober", "date": "2026-10-01", "type": "BILL", "labels": ["rechnungen"], "projects": ["Auto"]}"#,
    );
    assert_eq!(lite.title.as_deref(), Some("Stadtwerke Abschlag Oktober"));
    assert_eq!(
        lite.doc_type.as_deref(),
        Some("bill"),
        "as the list spells it"
    );
    assert_eq!(
        lite.date,
        Some(DateTime::from_ymd_hms(2026, 10, 1, 12, 0, 0).unwrap()),
        "that day at noon UTC"
    );
    assert_eq!(lite.labels, ["Rechnungen"]);
    assert_eq!(lite.projects, [Id::parse(OLD).unwrap()]);

    assert_eq!(
        read(r#"{"type": "Mahnung"}"#).doc_type,
        None,
        "only from the list"
    );
    assert_eq!(read(r#"{"type": null, "date": null}"#), Lite::default());
    let date = |date: &str| read(&format!(r#"{{"date": "{date}"}}"#)).date;
    assert!(
        date("2026-10-13T08:00:00Z").is_some(),
        "a day after the email"
    );
    assert_eq!(date("2026-10-14"), None, "later than the email");
    assert_eq!(date("1899-12-31"), None);
    assert_eq!(date("2026-13-01"), None);
    assert_eq!(date("1.10.2026"), None);
    assert_eq!(
        read(r#"{"title": "Rechnung"}"#).title,
        None,
        "its own title"
    );
    assert!(parse_document("Sorry.", AnalysisLevel::Lite, &ctx(), &doc).is_err());
}

// ------------------------------------------------------------------------
#[test]
fn applying_to_an_attachment_keeps_a_type_it_has() {
    let roles = Roles {
        document: Some(Default::default()),
        ..Default::default()
    };
    let mut doc = Document::create(
        roles,
        Source::Created(Default::default()),
        "Rechnung".into(),
        Vec::new(),
        FileFormat::Markdown,
    );
    let date = DateTime::from_ymd_hms(2026, 10, 1, 12, 0, 0).unwrap();
    let lite = Lite {
        title: Some("Stadtwerke Abschlag".into()),
        doc_type: Some("bill".into()),
        date: Some(date),
        labels: vec!["Rechnungen".into()],
        ..Lite::default()
    };
    apply_document(&mut doc, &lite);
    assert_eq!(
        (doc.title(), doc.date()),
        ("Stadtwerke Abschlag", Some(date))
    );
    let doc_type = |doc: &Document| doc.roles().document.clone().unwrap().doc_type;
    assert_eq!(doc_type(&doc), Some(DocType::Bill));
    assert_eq!(doc.labels(), ["Rechnungen"]);
    let letter = Lite {
        doc_type: Some("letter".into()),
        ..Lite::default()
    };
    apply_document(&mut doc, &letter);
    assert_eq!(doc_type(&doc), Some(DocType::Bill), "a type it has stays");
}

// ------------------------------------------------------------------------
#[test]
fn the_answer_is_read_against_the_lists() {
    let answer = "Sure! ```json\n{\"title\": \"  Abschlag Oktober \", \"labels\": [\"rechnungen\", \"Unbekannt\", \"Rechnungen\", \"Umzug\", \"Steuer\", \"Bank\"], \"projects\": [\"wohnung köln\", \"Garten\", \"Auto\", \"Wohnung Köln\"]}\n``` Done.";
    let (lite, medium) = parse(answer, AnalysisLevel::Lite, &ctx(), "AW: Abschlag").unwrap();
    assert_eq!(lite.title.as_deref(), Some("Abschlag Oktober"));
    assert_eq!((lite.doc_type, medium), (None, None));
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
        same.unwrap().0,
        Lite {
            title: None,
            doc_type: Some("notification".into()),
            labels: vec!["Bank".into()],
            projects: vec![],
            ..Lite::default()
        },
        "the email's own title is no change; one label as a string; the type in any case"
    );
    let unknown = lite(r#"{"type": "bill"}"#, "t").unwrap().0;
    assert_eq!(unknown.doc_type, None, "only an email's own types");
    let long = format!(r#"{{"title": "{}"}}"#, "x".repeat(200));
    let cut = lite(&long, "t").unwrap().0.title.unwrap();
    assert_eq!(cut.chars().count(), TITLE_CHARS);
    assert_eq!(lite(r#"{"title": "  "}"#, "t").unwrap().0.title, None);

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
fn medium_reads_the_points_the_reply_and_the_questions() {
    let medium = |answer: &str| parse(answer, AnalysisLevel::Medium, &ctx(), "t").unwrap().1;
    let long = "x".repeat(POINT_CHARS + 10);
    let answer = format!(
        r#"{{"check": [" Der Betrag ", "", "{long}", "Die Frist", "Zu viel"], "reply": " Danke! ", "questions": "Warum?"}}"#
    );
    let found = medium(&answer).unwrap();
    assert_eq!(found.check.len(), CHECKS_PER_EMAIL, "{found:?}");
    assert_eq!(found.check[0], "Der Betrag");
    assert_eq!(found.check[1].chars().count(), POINT_CHARS, "cut");
    assert_eq!(found.reply.as_deref(), Some("Danke!"));
    assert_eq!(found.questions, ["Warum?"]);

    let notification =
        medium(r#"{"type": "notification", "check": ["Der Betrag"], "reply": "Danke!"}"#);
    assert_eq!(
        notification,
        Some(Medium {
            check: vec!["Der Betrag".into()],
            reply: None,
            questions: vec![],
        }),
        "no reply to a notification"
    );
    let newsletter =
        r#"{"type": "newsletter", "check": ["Rabatt"], "reply": "Nein", "questions": ["Was?"]}"#;
    assert_eq!(medium(newsletter), None, "nothing for a newsletter");
    let nothing = r#"{"reply": null, "check": [], "questions": ["Was?"]}"#;
    assert_eq!(
        medium(nothing),
        None,
        "questions alone make no conversation"
    );
}

// ------------------------------------------------------------------------
#[test]
fn medium_messages_follow_each_other() {
    let account = Id::parse(OLLAMA_ID).unwrap();
    let now = DateTime::now();
    let medium = Medium {
        check: vec!["Der Betrag".into(), "Die Frist".into()],
        reply: Some("Danke!".into()),
        questions: vec!["Warum?".into()],
    };
    let messages = medium_messages(&medium, "m", account, now);
    let [points, reply] = &messages[..] else {
        panic!("{messages:?}");
    };
    assert_eq!(points.content.as_deref(), Some("- Der Betrag\n- Die Frist"));
    assert_eq!((points.parent, reply.parent), (None, points.id));
    assert_eq!(reply.content.as_deref(), Some("Danke!"));
    assert!(points.suggestions.is_empty());
    assert_eq!(reply.suggestions, ["Warum?"]);
    assert_eq!(
        (reply.model.as_deref(), reply.account),
        (Some("m"), Some(account))
    );
    assert!(reply.is_reply());

    let only_reply = Medium {
        check: vec![],
        ..medium
    };
    let messages = medium_messages(&only_reply, "m", account, now);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].parent, None);
    assert_eq!(messages[0].suggestions, ["Warum?"]);
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
        ..Lite::default()
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
    let answer = r#"{"title": "Abschlag Oktober", "labels": ["rechnungen"], "projects": ["Wohnung"], "reply": "Danke!"}"#;
    let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
    let mut h = with_emails("analysis-run", &address, "");
    let ids = vec![Id::parse(EMAIL_ID).unwrap()];
    analyse(
        &mut h.app,
        Batch::email("jan@example.com", ids, Vec::new()),
        &at(AnalysisLevel::Lite),
    );
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
fn medium_writes_a_chat_linked_to_the_email() {
    let answer = r#"{"title": "Abschlag Oktober", "check": ["Der Betrag ist neu."], "reply": "Danke, ist bezahlt.", "questions": ["Warum ist er hoeher?"]}"#;
    let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
    let mut h = with_emails("analysis-medium", &address, "");
    let email = Id::parse(EMAIL_ID).unwrap();
    analyse(
        &mut h.app,
        Batch::email("jan@example.com", vec![email], Vec::new()),
        &at(AnalysisLevel::Medium),
    );
    let sent = until_done(&mut h);

    assert_eq!(
        shown(&of(&sent, "job").last().unwrap()["summary"]),
        "1 analysed · 1 conversation"
    );
    assert!(of(&sent, "notice").is_empty(), "{sent:?}");
    let conversations: Vec<_> = fs::read_dir(h.vault().join("Conversations"))
        .unwrap()
        .collect();
    assert_eq!(conversations.len(), 1);

    let chat = h
        .app
        .vault
        .index()
        .all()
        .iter()
        .find(|e| e.doc.linked_to() == Some(email))
        .unwrap();
    let id = chat.doc.id();
    assert_eq!(chat.doc.title(), "Email analysis · Abschlag Oktober");
    assert_eq!(chat.doc.kind(), DocumentType::Conversation);
    let conversation = chat.doc.roles().conversation.clone().unwrap();
    let [points, reply] = &conversation.messages[..] else {
        panic!("{conversation:?}");
    };
    assert_eq!(points.content.as_deref(), Some("- Der Betrag ist neu."));
    assert_eq!(reply.parent, points.id);
    assert_eq!(reply.content.as_deref(), Some("Danke, ist bezahlt."));
    assert_eq!(conversation.active, reply.id);
    assert_eq!(reply.account, Some(Id::parse(OLLAMA_ID).unwrap()));

    // As the page opens it: the questions under the reply.
    let opened = h.call("chat_open", json!({ "id": id }));
    let messages = &opened[0]["ok"]["conversation"]["messages"];
    assert_eq!(messages[0]["role"], "assistant");
    assert!(messages[0].get("suggestions").is_none());
    assert_eq!(messages[1]["suggestions"], json!(["Warum ist er hoeher?"]));
}

// ------------------------------------------------------------------------
#[test]
fn a_email_and_its_attachment_are_analysed_in_one_job() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let answers = [
        r#"{"type": "notification", "title": "Abschlag Oktober"}"#,
        r#"{"title": "Stadtwerke Rechnung", "date": "2026-10-01", "type": "bill"}"#,
    ];
    let address = FakeOllama::new(&["m"])
        .answering_in_turn(&answers)
        .recording(&requests)
        .start();
    let account = format!("id: {OLLAMA_ID}\nollama:\n  address: {address}\n  offered: [m]\n");
    let bill = Id::generate();
    let attachment = format!(
        "---\nid: {bill}\ntitle: Rechnung\ndate: 2026-10-12T09:30:00Z\nlinked-to: {EMAIL_ID}\ndocument:\n  original:\n    file: r.pdf\n    name: Rechnung.pdf\n---\nBetrag: 42,00 EUR\n"
    );
    let files = [
        (".avelor/accounts/ollama.yaml", account),
        ("Email/a.md", email_file(EMAIL_ID, "Abschlag", "")),
        ("Attachments/r.md", attachment),
    ];
    let mut h = Harness::with_vault("analysis-attachment", &files);
    let email = vec![Id::parse(EMAIL_ID).unwrap()];
    analyse(
        &mut h.app,
        Batch::email("jan@example.com", email, vec![bill]),
        &at(AnalysisLevel::Medium),
    );
    let sent = until_done(&mut h);

    let jobs = of(&sent, "job");
    assert_eq!(jobs[0]["total"], 2);
    assert_eq!(shown(&jobs.last().unwrap()["summary"]), "2 analysed");
    assert!(of(&sent, "notice").is_empty(), "{sent:?}");
    let requests = requests.lock().unwrap();
    let user = |i: usize| {
        requests[i]["messages"][1]["content"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(
        user(0)
            .ends_with("Its attached files, as read:\n\n--- Rechnung.pdf ---\nBetrag: 42,00 EUR"),
        "{}",
        user(0)
    );
    assert!(user(1).contains("File: Rechnung.pdf\nAttached to the email: Abschlag"));
    let text = fs::read_to_string(h.vault().join("Attachments/r.md")).unwrap();
    assert!(text.contains("title: Stadtwerke Rechnung"), "{text}");
    assert!(text.contains("date: 2026-10-01T12:00:00Z"), "{text}");
    assert!(text.contains("type: bill"), "{text}");
    assert!(
        !h.vault().join("Conversations").exists(),
        "Medium writes no conversation for an attachment, nor for this email"
    );
}

// ------------------------------------------------------------------------
#[test]
fn a_scan_at_medium_gets_points_to_check_in_a_chat_of_its_own() {
    let doc = DocText {
        email: None,
        ..doc_text("Betrag 42 EUR", None)
    };
    let request = document_request(AnalysisLevel::Medium, "m", &ctx(), &doc);
    let system = request.system.as_deref().unwrap();
    assert!(
        system.contains(DOCUMENT_MEDIUM_SHAPE) && system.contains("- check:"),
        "{system}"
    );
    assert!(!system.contains("- reply:"), "{system}");
    let answer = r#"{"title": "Rechnung", "check": ["Der Betrag"], "reply": "Danke", "questions": ["Wann?"]}"#;
    let (_, medium) = parse_document(answer, AnalysisLevel::Medium, &ctx(), &doc).unwrap();
    let medium = medium.unwrap();
    assert_eq!(
        (medium.check, medium.reply, medium.questions),
        (
            vec!["Der Betrag".to_string()],
            None,
            vec!["Wann?".to_string()]
        )
    );
    let questions = r#"{"questions": ["Wann?"]}"#;
    let (_, none) = parse_document(questions, AnalysisLevel::Medium, &ctx(), &doc).unwrap();
    assert_eq!(none, None, "questions alone make no conversation");
    let (_, lite) = parse_document(answer, AnalysisLevel::Lite, &ctx(), &doc).unwrap();
    assert_eq!(lite, None);

    let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
    let account = format!("id: {OLLAMA_ID}\nollama:\n  address: {address}\n  offered: [m]\n");
    let scan = Id::generate();
    let files = [
        (".avelor/accounts/ollama.yaml", account),
        (
            "Scans/s.md",
            format!("---\nid: {scan}\ntitle: Seite 1\ndocument: {{}}\n---\nBetrag 42 EUR\n"),
        ),
    ];
    let mut h = Harness::with_vault("analysis-scan", &files);
    let batch = Batch::scans(WatchKind::Images, "Scanner", vec![scan], vec![scan]);
    analyse(&mut h.app, batch, &at(AnalysisLevel::Medium));
    let sent = until_done(&mut h);

    let jobs = of(&sent, "job");
    assert_eq!(shown(&jobs[0]["label"]), "Analysing scans \u{b7} Scanner");
    assert_eq!(
        shown(&jobs.last().unwrap()["summary"]),
        "1 analysed \u{b7} 1 conversation"
    );
    let text = fs::read_to_string(h.vault().join("Scans/s.md")).unwrap();
    assert!(
        text.contains("title: Seite 1"),
        "a title given stays: {text}"
    );
    let chat = h
        .app
        .vault
        .index()
        .all()
        .iter()
        .find(|e| e.doc.linked_to() == Some(scan))
        .unwrap();
    assert_eq!(chat.doc.title(), "Document analysis \u{b7} Seite 1");
    let conversation = chat.doc.roles().conversation.clone().unwrap();
    let [points] = &conversation.messages[..] else {
        panic!("{conversation:?}");
    };
    assert_eq!(points.content.as_deref(), Some("- Der Betrag"));
    assert_eq!(points.suggestions, ["Wann?"]);
}

// ------------------------------------------------------------------------
#[test]
fn a_refused_scan_pick_is_told_in_the_watchs_words() {
    let mut h = with_emails("analysis-scan-no-pick", "http://127.0.0.1:9", "");
    let unpicked = Analysis {
        level: AnalysisLevel::Lite,
        model: None,
    };
    let documents = || vec![Id::parse(EMAIL_ID).unwrap()];
    let batch = Batch::scans(WatchKind::Images, "Scanner", documents(), Vec::new());
    analyse(&mut h.app, batch, &unpicked);
    let sent = parse_sent(&h.take());
    assert_eq!(
        shown(&sent[0]["data"]["text"]),
        "Scan analysis \u{b7} Scanner: For the scan analysis, pick a model."
    );
    let batch = Batch::scans(WatchKind::Pdf, "Bills", documents(), Vec::new());
    assert_eq!(batch.label.render(), "Analysing PDFs \u{b7} Bills");
    analyse(&mut h.app, batch, &unpicked);
    let sent = parse_sent(&h.take());
    assert_eq!(
        shown(&sent[0]["data"]["text"]),
        "PDF analysis \u{b7} Bills: For the PDF analysis, pick a model."
    );
}

// ------------------------------------------------------------------------
#[test]
fn nothing_to_analyse_starts_nothing() {
    let mut h = with_emails("analysis-nothing", "http://127.0.0.1:9", "");
    let unpicked = Analysis {
        level: AnalysisLevel::Lite,
        model: None,
    };
    analyse(
        &mut h.app,
        Batch::email("jan@example.com", Vec::new(), Vec::new()),
        &unpicked,
    );
    assert!(h.take().is_empty(), "not even the refused pick");
}

// ------------------------------------------------------------------------
#[test]
fn a_newsletter_gets_no_chat() {
    let answer = r#"{"type": "newsletter", "title": "Angebote", "check": ["Rabatt"], "reply": "Nein", "questions": ["Was?"]}"#;
    let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
    let mut h = with_emails("analysis-newsletter", &address, "");
    let ids = vec![Id::parse(EMAIL_ID).unwrap()];
    analyse(
        &mut h.app,
        Batch::email("jan@example.com", ids, Vec::new()),
        &at(AnalysisLevel::Medium),
    );
    let sent = until_done(&mut h);

    assert_eq!(
        shown(&of(&sent, "job").last().unwrap()["summary"]),
        "1 analysed"
    );
    let text = fs::read_to_string(h.vault().join("Email/a.md")).unwrap();
    assert!(text.contains("type: newsletter"), "{text}");
    assert!(!h.vault().join("Conversations").exists());
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
        Batch::email(
            "jan@example.com",
            vec![Id::parse(EMAIL_ID).unwrap()],
            Vec::new(),
        ),
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
    analyse(
        &mut h.app,
        Batch::email("jan@example.com", ids, Vec::new()),
        &at(AnalysisLevel::Lite),
    );
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
    analyse(
        &mut h.app,
        Batch::email("jan@example.com", ids, Vec::new()),
        &at(AnalysisLevel::Lite),
    );
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
    analyse(
        &mut h.app,
        Batch::email("jan@example.com", ids, Vec::new()),
        &at(AnalysisLevel::Lite),
    );
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
        Batch::email(
            "jan@example.com",
            vec![Id::parse(EMAIL_ID).unwrap()],
            Vec::new(),
        ),
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
    analyse(
        &mut h.app,
        Batch::email("jan@example.com", ids, Vec::new()),
        &at(AnalysisLevel::Lite),
    );
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
