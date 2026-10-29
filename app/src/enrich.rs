//! Enriching entities with an AI (mail.md#analysis): the mail analysis -- Lite
//! gives each new mail its kind, a clean title, labels in use and active
//! projects; Medium also points to check, a quick reply and questions, as a
//! chat linked to the mail. The attachments read go along with their mail,
//! and are analysed after the mails as Lite does a mail: a clean title,
//! their own date, a genre, labels and projects.
//!
//! An analysis goes plan → run → apply (architecture.md#conventions), one mail
//! after another, as a job: the plan picks the model (`AiAccounts::choose`),
//! gathers the vault's labels and projects and each mail's text; the run asks
//! the model; the apply reads its answer and writes it through the vault edit
//! (`chat::create` for Medium's chat). The spend limit is checked before each
//! mail.

use std::collections::{HashMap, VecDeque};

use av_llm::{ChatRequest, ChatResponse, Client, Message, Provider, StopReason};
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::document::Document;
use av_vault::envelope::ConversationMessage;
use av_vault::envelope::DocumentType;
use av_vault::shared::Genre;
use av_vault::vault::SaveOutcome;
use serde_json::Value;

use crate::ai;
use crate::app::App;
use crate::chat;
use crate::google_accounts::{Analysis, AnalysisLevel};
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use av_vault::markdown::plain_title;
use av_vault::vault::Index;

/// What the mail analysis is called in a refused pick's words
/// (`AiAccounts::choose`).
pub const MAIL_ANALYSIS: &str = "the mail analysis";
/// The most of a mail's body that is sent, in characters.
const BODY_CHARS: usize = 8000;
/// The most of an attachment's text that goes with its mail, in characters,
/// and the most attachments that do.
const ATTACHMENT_CHARS: usize = 2000;
const ATTACHMENTS_SENT: usize = 3;
/// The genre never offered: a note is the user's own (it puts a document
/// under Notes).
const NOTE: &str = "note";
const DAY_SECONDS: i64 = 86_400;
/// The most labels and projects the request lists.
const LABELS_LISTED: usize = 100;
const PROJECTS_LISTED: usize = 50;
/// The most labels and projects one mail gets.
const LABELS_PER_MAIL: usize = 3;
const PROJECTS_PER_MAIL: usize = 2;
/// A longer title is cut.
const TITLE_CHARS: usize = 120;
/// The most points to check and questions one mail gets, and how long
/// each may be; a longer quick reply is cut too.
const CHECKS_PER_MAIL: usize = 3;
const QUESTIONS_PER_MAIL: usize = 3;
const POINT_CHARS: usize = 300;
const REPLY_CHARS: usize = 4000;
/// A longer title of Medium's chat is cut.
const CHAT_TITLE_CHARS: usize = 60;
/// The kinds of mail (vault.md#email).
pub const KINDS: [&str; 5] = ["request", "reply", "notification", "newsletter", "other"];
/// How often a mail is asked about at most while the answer isn't the JSON
/// object asked for.
const LITE_ATTEMPTS: u32 = 3;
/// Why an answer that was cut off isn't asked for again: it would be cut off
/// the same way.
const CUT_OFF: &str = "The model ran out of room before it finished its answer (its context \
window or its output limit).";

/// The JSON object each level asks for.
const LITE_SHAPE: &str =
    "{\"kind\": \"...\", \"title\": \"...\", \"labels\": [\"...\"], \"projects\": [\"...\"]}";
const MEDIUM_SHAPE: &str = "{\"kind\": \"...\", \"title\": \"...\", \"labels\": [\"...\"], \
\"projects\": [\"...\"], \"check\": [\"...\"], \"reply\": \"...\", \"questions\": [\"...\"]}";

const LITE_RULES: &str = "
- kind: what the mail is: \"request\" (it asks its recipient to do, answer, pay or decide \
something), \"reply\" (it answers a mail of the recipient's), \"notification\" (sent \
automatically: receipts, shipping, account and security alerts, reminders), \"newsletter\" (news, \
offers or marketing sent to many) or \"other\".
- title: a clean title for the mail, in the mail's language, at most 80 characters: what it is \
about, without \"Re:\", \"Fwd:\", ticket or reference numbers, tags in brackets or the sender's \
name.
- labels: up to 3 labels from the list of labels that clearly fit the mail; none if unsure.
- projects: up to 2 projects from the list of projects the mail clearly belongs to; none if \
unsure.";

const MEDIUM_RULES: &str = "
- check: up to 3 points a closer look at the mail should check, one sentence each: an amount, a \
deadline, a claim, a condition; none if there is nothing to check, and none for a newsletter.
- reply: a short reply to the mail in its language, ready to send, without a signature or \
placeholders; only if the mail expects an answer from its recipient, else null; never for a \
notification or a newsletter.
- questions: 1 to 3 short questions the recipient might ask about this mail; none for a \
newsletter.
Write the points and the questions in the mail's language too.";

const LISTS_ONLY: &str = "\nUse only labels and projects from the lists, spelled as given.";

/// What an attachment is asked for, at either level.
const DOCUMENT_SHAPE: &str = "{\"title\": \"...\", \"date\": \"YYYY-MM-DD\", \"genre\": \"...\", \
\"labels\": [\"...\"], \"projects\": [\"...\"]}";

const DOCUMENT_RULES: &str = "
- title: a clean title for the document, in its language, at most 80 characters: what it is \
and who it is from, without file names or reference numbers.
- date: the date the document itself carries (a letter's, a bill's), as YYYY-MM-DD; null if it \
has none.
- genre: the one genre from the list of genres that fits the document; null if none fits.
- labels: up to 3 labels from the list of labels that clearly fit the document; none if unsure.
- projects: up to 2 projects from the list of projects the document clearly belongs to; none \
if unsure.
Use only genres, labels and projects from the lists, spelled as given.";

// ----------------------------------------------------------------------------
/// What the vault offers an analysis to pick from: the labels in use, the
/// active projects, and the genres for a document.
#[derive(Debug, Default)]
pub struct Context {
    /// As the vault spells them, the most used first.
    pub labels: Vec<String>,
    /// Id and title, in the vault's order.
    pub projects: Vec<(Id, String)>,
    /// The well-known genres, then those the vault's documents use, each
    /// as first seen; never `NOTE`.
    pub genres: Vec<String>,
}

// ----------------------------------------------------------------------------
/// The context of the vault `index`: its labels by how many entities carry them (a
/// label in other case counts as the same, spelled as first seen), and its
/// projects that aren't archived; each list capped (the labels used most,
/// the first projects); the genres a document may get.
pub fn context(index: &Index) -> Context {
    let mut labels: Vec<(String, usize)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut projects = Vec::new();
    let mut genres: Vec<String> = Genre::well_known().map(str::to_string).collect();
    for entry in index.all() {
        let doc = &entry.doc;
        let role = doc.roles().document.as_ref();
        if let Some(genre) = role.and_then(|role| role.genre.as_ref()) {
            genres.push(genre.as_str().to_string());
        }
        for tag in doc.tags() {
            let key = tag.to_lowercase();
            match seen.get(&key) {
                Some(&i) => labels[i].1 += 1,
                None => {
                    seen.insert(key, labels.len());
                    labels.push((tag.clone(), 1));
                }
            }
        }
        if doc.kind() == DocumentType::Project && !doc.archived() {
            projects.push((doc.id(), doc.title().to_string()));
        }
    }
    labels.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    labels.truncate(LABELS_LISTED);
    projects.truncate(PROJECTS_LISTED);
    let mut offered: Vec<String> = Vec::new();
    for genre in genres {
        if !same(&genre, NOTE) && !offered.iter().any(|g| same(g, &genre)) {
            offered.push(genre);
        }
    }
    Context {
        labels: labels.into_iter().map(|(label, _)| label).collect(),
        projects,
        genres: offered,
    }
}

// ----------------------------------------------------------------------------
/// A mail as the analysis sends it.
#[derive(Debug, Clone)]
pub struct MailText {
    pub id: Id,
    /// Its title when the analysis was planned: the subject, for a mail
    /// just fetched.
    pub title: String,
    /// The subject as the mail came: `email.subject`, else the title.
    pub subject: String,
    pub from: String,
    /// The recipients, joined.
    pub to: String,
    pub date: Option<DateTime>,
    /// Cut at `BODY_CHARS`.
    pub body: String,
    /// Its attachments' names and read text (`attachment_texts`).
    pub attachments: Vec<(String, String)>,
}

impl MailText {
    // ------------------------------------------------------------------------
    /// `doc` as a mail, without its attachments.
    pub fn of(doc: &Document) -> Self {
        let email = doc.roles().email.as_ref();
        let body = cut_text(doc.content(), BODY_CHARS);
        let subject = email.and_then(|e| e.subject.as_deref());
        Self {
            id: doc.id(),
            title: doc.title().to_string(),
            subject: subject.unwrap_or(doc.title()).to_string(),
            from: email.and_then(|e| e.from.clone()).unwrap_or_default(),
            to: email
                .map(|e| e.to.as_slice().join(", "))
                .unwrap_or_default(),
            date: doc.date(),
            body,
            attachments: Vec::new(),
        }
    }
}

// ----------------------------------------------------------------------------
/// `text` cut at `most` characters, marked where it was cut.
fn cut_text(text: &str, most: usize) -> String {
    let mut cut: String = text.chars().take(most).collect();
    if cut.len() < text.len() {
        cut.push_str("\n[\u{2026}]");
    }
    cut
}

// ----------------------------------------------------------------------------
/// What goes with the mail `mail` of its attachments: the documents linked
/// to it that have an original and text, each with the name its file came
/// with and its text cut at `ATTACHMENT_CHARS`; the first
/// `ATTACHMENTS_SENT`.
fn attachment_texts(index: &Index, mail: Id) -> Vec<(String, String)> {
    index
        .all()
        .iter()
        .filter(|entry| entry.doc.linked_to() == Some(mail))
        .filter_map(|entry| {
            let original = entry.doc.original()?;
            let text = entry.doc.content().trim();
            let name = original.name.clone();
            let name = name.unwrap_or_else(|| entry.doc.title().to_string());
            (!text.is_empty()).then(|| (name, cut_text(text, ATTACHMENT_CHARS)))
        })
        .take(ATTACHMENTS_SENT)
        .collect()
}

// ----------------------------------------------------------------------------
/// An attachment as the analysis sends it: its read text, and the mail it
/// came with.
#[derive(Debug, Clone)]
pub struct DocText {
    pub id: Id,
    /// Its title when the analysis was planned: its file's name, for one
    /// just saved.
    pub title: String,
    /// The name its file came with.
    pub name: String,
    /// Its date when the analysis was planned: its mail's, for one just
    /// saved.
    pub date: Option<DateTime>,
    /// The subject and sender of the mail it is linked to.
    pub mail: Option<(String, String)>,
    /// Cut at `BODY_CHARS`.
    pub body: String,
}

impl DocText {
    // ------------------------------------------------------------------------
    /// `doc`, an attachment, with the mail of `index` it is linked to.
    pub fn of(index: &Index, doc: &Document) -> Self {
        let mail = doc.linked_to().and_then(|id| index.get(&id));
        let mail = mail
            .filter(|mail| mail.roles().email.is_some())
            .map(|mail| {
                let mail = MailText::of(mail);
                (mail.subject, mail.from)
            });
        let name = doc.original().and_then(|original| original.name.clone());
        Self {
            id: doc.id(),
            title: doc.title().to_string(),
            name: name.unwrap_or_else(|| doc.title().to_string()),
            date: doc.date(),
            mail,
            body: cut_text(doc.content(), BODY_CHARS),
        }
    }
}

// ----------------------------------------------------------------------------
/// `names` as the request lists them: a line each, or "(none)".
fn listed(names: Vec<&str>) -> String {
    match names.is_empty() {
        true => "(none)\n".to_string(),
        false => names.iter().map(|n| format!("- {n}\n")).collect(),
    }
}

// ----------------------------------------------------------------------------
/// The request of the analysis at `level` for `mail` to `model`: the
/// instructions, then the lists of `ctx`, the mail, and the text of its
/// attachments read.
pub fn request_for(
    level: AnalysisLevel,
    model: &str,
    ctx: &Context,
    mail: &MailText,
) -> ChatRequest {
    let labels = listed(ctx.labels.iter().map(String::as_str).collect());
    let projects = listed(ctx.projects.iter().map(|(_, t)| t.as_str()).collect());
    let date = mail.date.map(|d| d.as_iso8601()).unwrap_or_default();
    let mut text = format!(
        "Labels:\n{labels}\nProjects:\n{projects}\nThe mail:\nSubject: {}\nFrom: {}\nTo: {}\nDate: {date}\n\n{}",
        mail.subject, mail.from, mail.to, mail.body
    );
    if !mail.attachments.is_empty() {
        text.push_str("\n\nIts attached files, as read:");
        for (name, read) in &mail.attachments {
            text.push_str(&format!("\n\n--- {name} ---\n{read}"));
        }
    }
    let medium = level == AnalysisLevel::Medium;
    let task = match medium {
        true => "You sort incoming mail and prepare it for its recipient.",
        false => "You sort incoming mail.",
    };
    let shape = shape(level);
    let mut system =
        format!("{task} Answer with one JSON object and nothing else: {shape}.{LITE_RULES}");
    if medium {
        system.push_str(MEDIUM_RULES);
    }
    system.push_str(LISTS_ONLY);
    ChatRequest::new(model)
        .system(system)
        .message(Message::user(text))
        // Sorting a mail needs no reasoning, and on a small context window
        // the reasoning leaves no room for the answer.
        .think(false)
        .context(ai::CONTEXT_TOKENS)
}

// ----------------------------------------------------------------------------
/// The request of the analysis for the attachment `doc` to `model`, as
/// Lite at either level: the instructions, then the lists of `ctx` and the
/// document.
pub fn document_request(model: &str, ctx: &Context, doc: &DocText) -> ChatRequest {
    let genres = listed(ctx.genres.iter().map(String::as_str).collect());
    let labels = listed(ctx.labels.iter().map(String::as_str).collect());
    let projects = listed(ctx.projects.iter().map(|(_, t)| t.as_str()).collect());
    let mail = match &doc.mail {
        Some((subject, from)) if !from.is_empty() => {
            format!("Attached to the mail: {subject} (from {from})\n")
        }
        Some((subject, _)) => format!("Attached to the mail: {subject}\n"),
        None => String::new(),
    };
    let text = format!(
        "Genres:\n{genres}\nLabels:\n{labels}\nProjects:\n{projects}\nThe document:\nFile: {}\n{mail}\n{}",
        doc.name, doc.body
    );
    let system = format!(
        "You sort documents. Answer with one JSON object and nothing else: {DOCUMENT_SHAPE}.{DOCUMENT_RULES}"
    );
    ChatRequest::new(model)
        .system(system)
        .message(Message::user(text))
        .think(false)
        .context(ai::CONTEXT_TOKENS)
}

// ----------------------------------------------------------------------------
/// The JSON object the analysis at `level` asks for.
fn shape(level: AnalysisLevel) -> &'static str {
    match level {
        AnalysisLevel::Medium => MEDIUM_SHAPE,
        _ => LITE_SHAPE,
    }
}

// ----------------------------------------------------------------------------
/// `request` asked again after `answer`, which wasn't the JSON object: the
/// answer and a reminder of the object `shape` follow it. An empty answer
/// is just asked again (providers refuse an empty turn).
pub fn again(request: ChatRequest, answer: &str, shape: &str) -> ChatRequest {
    if answer.trim().is_empty() {
        return request;
    }
    let reminder = format!(
        "That wasn't the JSON object asked for. Answer again with only the JSON object {shape}, \
nothing before or after it."
    );
    request
        .message(Message::assistant(answer))
        .message(Message::user(reminder))
}

// ----------------------------------------------------------------------------
/// What Lite found for a mail or an attachment, in the vault's terms.
#[derive(Debug, Default, PartialEq)]
pub struct Lite {
    /// `None`: the title stays.
    pub title: Option<String>,
    /// A mail's: one of `KINDS`.
    pub kind: Option<String>,
    /// Labels in use, as the vault spells them.
    pub labels: Vec<String>,
    /// Active projects.
    pub projects: Vec<Id>,
    /// An attachment's: one of `Context::genres`.
    pub genre: Option<String>,
    /// An attachment's own date.
    pub date: Option<DateTime>,
}

// ----------------------------------------------------------------------------
/// What Medium found beyond Lite, for the mail's chat: a quick reply or a
/// point to check, at least.
#[derive(Debug, Default, PartialEq)]
pub struct Medium {
    pub check: Vec<String>,
    pub reply: Option<String>,
    pub questions: Vec<String>,
}

// ----------------------------------------------------------------------------
/// Read the `answer` of the analysis at `level` against `ctx` for a mail
/// titled `title`: the JSON object in it (code fences and text around it
/// ignored); a title trimmed and cut, dropped if empty or the mail's own;
/// a kind from `KINDS`, in any case; the labels and projects from the lists
/// only, matched in any case, each once, the first few kept. Medium's
/// points, reply and questions trimmed and cut, the first few kept; none
/// for a newsletter, no reply for a notification, and no chat without a
/// reply or a point. An answer without such an object is refused, in
/// words.
pub fn parse(
    answer: &str,
    level: AnalysisLevel,
    ctx: &Context,
    title: &str,
) -> std::result::Result<(Lite, Option<Medium>), String> {
    let object = object_in(answer)?;
    let kind = object["kind"]
        .as_str()
        .and_then(|kind| KINDS.into_iter().find(|known| same(known, kind)))
        .map(str::to_string);
    let lite = Lite {
        kind,
        ..shared_part(&object, ctx, title)
    };
    let medium = match level {
        AnalysisLevel::Medium => medium_of(&object, lite.kind.as_deref()),
        _ => None,
    };
    Ok((lite, medium))
}

// ----------------------------------------------------------------------------
/// Read the `answer` of the analysis for the attachment `doc` against `ctx`
/// as `parse` reads a mail's: its genre from the list only, in any case;
/// its date as `YYYY-MM-DD`, that day at 12:00 UTC, the same day in every
/// time zone, dropped before 1900 or after the day after the document's
/// date (its mail's; now without one).
pub fn parse_document(
    answer: &str,
    ctx: &Context,
    doc: &DocText,
) -> std::result::Result<Lite, String> {
    let object = object_in(answer)?;
    let genre = object["genre"]
        .as_str()
        .and_then(|genre| ctx.genres.iter().find(|known| same(known, genre)))
        .cloned();
    let day = |date: DateTime| date.as_unix().div_euclid(DAY_SECONDS);
    let latest = day(doc.date.unwrap_or_else(DateTime::now)) + 1;
    let date = object["date"]
        .as_str()
        .and_then(day_of)
        .filter(|date| day(*date) <= latest);
    Ok(Lite {
        genre,
        date,
        ..shared_part(&object, ctx, &doc.title)
    })
}

// ----------------------------------------------------------------------------
/// The day `text` names as `YYYY-MM-DD` (anything after it ignored), at
/// 12:00 UTC; `None` if it names none, or one before 1900.
fn day_of(text: &str) -> Option<DateTime> {
    let mut parts = text.trim().get(..10)?.split('-');
    let mut next = || parts.next()?.parse::<i32>().ok();
    let (year, month, day) = (next()?, next()?, next()?);
    if year < 1900 {
        return None;
    }
    DateTime::from_ymd_hms(year, month, day, 12, 0, 0).ok()
}

// ----------------------------------------------------------------------------
/// The JSON object in `answer`, code fences and text around it ignored; an
/// answer without one is refused, in words.
fn object_in(answer: &str) -> std::result::Result<Value, String> {
    let refused = || "The answer wasn't the JSON object asked for.".to_string();
    let (Some(start), Some(end)) = (answer.find('{'), answer.rfind('}')) else {
        return Err(refused());
    };
    match answer.get(start..=end).map(serde_json::from_str) {
        Some(Ok(object @ Value::Object(_))) => Ok(object),
        _ => Err(refused()),
    }
}

// ----------------------------------------------------------------------------
/// What a mail's answer `object` and an attachment's share: a title trimmed
/// and cut, dropped if empty or `title`; the labels and projects from the
/// lists of `ctx` only, matched in any case, each once, the first few kept.
fn shared_part(object: &Value, ctx: &Context, title: &str) -> Lite {
    let new_title = object["title"]
        .as_str()
        .map(|t| cut(t, TITLE_CHARS))
        .filter(|t| !t.is_empty() && t != title.trim());

    let mut labels: Vec<String> = Vec::new();
    for name in names(&object["labels"]) {
        let found = ctx.labels.iter().find(|l| same(l, name));
        if let Some(label) = found.filter(|l| !labels.contains(l)) {
            labels.push(label.clone());
        }
    }
    labels.truncate(LABELS_PER_MAIL);

    let mut projects: Vec<Id> = Vec::new();
    for name in names(&object["projects"]) {
        let found = ctx.projects.iter().find(|(_, t)| same(t, name));
        if let Some((id, _)) = found.filter(|(id, _)| !projects.contains(id)) {
            projects.push(*id);
        }
    }
    projects.truncate(PROJECTS_PER_MAIL);

    Lite {
        title: new_title,
        labels,
        projects,
        ..Lite::default()
    }
}

// ----------------------------------------------------------------------------
/// Medium's part of the answer `object` for a mail of `kind` (see `parse`).
fn medium_of(object: &Value, kind: Option<&str>) -> Option<Medium> {
    if kind == Some("newsletter") {
        return None;
    }
    let points = |key: &str, most: usize| -> Vec<String> {
        let points = names(&object[key]).into_iter().map(|p| cut(p, POINT_CHARS));
        points.filter(|p| !p.is_empty()).take(most).collect()
    };
    let reply = object["reply"]
        .as_str()
        .map(|reply| cut(reply, REPLY_CHARS))
        .filter(|reply| !reply.is_empty() && kind != Some("notification"));
    let medium = Medium {
        check: points("check", CHECKS_PER_MAIL),
        reply,
        questions: points("questions", QUESTIONS_PER_MAIL),
    };
    (medium.reply.is_some() || !medium.check.is_empty()).then_some(medium)
}

// ----------------------------------------------------------------------------
/// `text` trimmed and cut at `most` characters.
fn cut(text: &str, most: usize) -> String {
    let cut: String = text.trim().chars().take(most).collect();
    cut.trim_end().to_string()
}

// ----------------------------------------------------------------------------
/// The names in `value`: a list of strings, or a single one.
fn names(value: &Value) -> Vec<&str> {
    match value {
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        Value::String(name) => vec![name.as_str()],
        _ => Vec::new(),
    }
}

// ----------------------------------------------------------------------------
/// Whether two names are the same in any case, around spaces.
fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

// ----------------------------------------------------------------------------
/// Write `lite` into the mail `doc`: a new title, the old one kept as
/// `email.subject` if it has none; its kind as `email.kind`; the labels
/// added to its own (in any case once); the projects joined.
pub fn apply_lite(doc: &mut Document, lite: &Lite) {
    if let Some(mut email) = doc.roles().email.clone() {
        let before = email.clone();
        if lite.title.is_some() && email.subject.is_none() {
            email.subject = Some(doc.title().to_string());
        }
        if lite.kind.is_some() {
            email.kind = lite.kind.clone();
        }
        if email != before {
            doc.set_email(Some(email));
        }
    }
    if let Some(title) = &lite.title {
        doc.set_title(title.clone());
    }
    add_labels_and_projects(doc, lite);
}

// ----------------------------------------------------------------------------
/// Write `lite` into the attachment's document `doc`: a new title and date;
/// its genre, unless it has one; the labels and projects as for a mail.
pub fn apply_document(doc: &mut Document, lite: &Lite) {
    if let Some(title) = &lite.title {
        doc.set_title(title.clone());
    }
    if let Some(date) = lite.date {
        doc.set_date(date);
    }
    let role = doc.roles().document.as_ref();
    let has_genre = role.is_some_and(|role| role.genre.is_some());
    if let Some(genre) = lite.genre.as_deref().filter(|_| !has_genre) {
        doc.set_genre(Some(Genre::parse(genre)));
    }
    add_labels_and_projects(doc, lite);
}

// ----------------------------------------------------------------------------
/// `lite`'s labels added to `doc`'s own (in any case once), its projects
/// joined.
fn add_labels_and_projects(doc: &mut Document, lite: &Lite) {
    let mut tags = doc.tags().to_vec();
    let before = tags.len();
    for label in &lite.labels {
        if !tags.iter().any(|t| same(t, label)) {
            tags.push(label.clone());
        }
    }
    if tags.len() > before {
        doc.set_tags(tags);
    }
    for project in &lite.projects {
        doc.join_project(*project);
    }
}

// ----------------------------------------------------------------------------
/// The messages of Medium's chat about a mail: the points to check as a
/// list, then the quick reply as a message of its own, each following the
/// one before; the questions are offered under the last. Written by `model`
/// through `account` at `now`.
pub fn medium_messages(
    medium: &Medium,
    model: &str,
    account: Id,
    now: DateTime,
) -> Vec<ConversationMessage> {
    let points = medium.check.iter().map(|p| format!("- {p}"));
    let points = Some(points.collect::<Vec<_>>().join("\n")).filter(|p| !p.is_empty());
    let mut messages: Vec<ConversationMessage> = Vec::new();
    for content in [points, medium.reply.clone()].into_iter().flatten() {
        messages.push(ConversationMessage {
            id: Some(Id::generate()),
            parent: messages.last().and_then(|m| m.id),
            model: Some(model.to_string()),
            account: Some(account),
            date: Some(now),
            content: Some(content),
            ..Default::default()
        });
    }
    if let Some(last) = messages.last_mut() {
        last.suggestions = medium.questions.clone();
    }
    messages
}

// ----------------------------------------------------------------------------
/// The analyses running (`App::enrich`), by job.
#[derive(Default)]
pub struct EnrichState {
    runs: HashMap<Id, Run>,
}

// ----------------------------------------------------------------------------
/// One analysis: its model, the vault's context, the mails left and the
/// one being analysed.
struct Run {
    /// Whose mail it is, for the notices.
    email: String,
    account: Id,
    client: Client,
    provider: Provider,
    address: String,
    model: String,
    level: AnalysisLevel,
    context: Context,
    queue: VecDeque<Ask>,
    current: Option<Ask>,
    total: usize,
    counts: Counts,
    /// What it analyses, for the failures' notice: one, and more than one.
    noun: (&'static str, &'static str),
}

// ----------------------------------------------------------------------------
/// A mail or an attachment to ask about: the how-manyth time, and the
/// answer of the time before, which wasn't the JSON object.
struct Ask {
    item: Item,
    attempt: u32,
    unread: Option<String>,
}

// ----------------------------------------------------------------------------
/// What an analysis asks about.
enum Item {
    Mail(MailText),
    /// An attachment, asked about as Lite at either level.
    Document(DocText),
}

impl Item {
    // ------------------------------------------------------------------------
    fn id(&self) -> Id {
        match self {
            Item::Mail(mail) => mail.id,
            Item::Document(doc) => doc.id,
        }
    }

    // ------------------------------------------------------------------------
    /// Its title when the analysis was planned.
    fn title(&self) -> &str {
        match self {
            Item::Mail(mail) => &mail.title,
            Item::Document(doc) => &doc.title,
        }
    }

    // ------------------------------------------------------------------------
    /// The JSON object asked for at `level`.
    fn shape(&self, level: AnalysisLevel) -> &'static str {
        match self {
            Item::Mail(_) => shape(level),
            Item::Document(_) => DOCUMENT_SHAPE,
        }
    }

    // ------------------------------------------------------------------------
    fn request(&self, level: AnalysisLevel, model: &str, ctx: &Context) -> ChatRequest {
        match self {
            Item::Mail(mail) => request_for(level, model, ctx, mail),
            Item::Document(doc) => document_request(model, ctx, doc),
        }
    }

    // ------------------------------------------------------------------------
    fn parse(
        &self,
        answer: &str,
        level: AnalysisLevel,
        ctx: &Context,
    ) -> std::result::Result<(Lite, Option<Medium>), String> {
        match self {
            Item::Mail(mail) => parse(answer, level, ctx, &mail.title),
            Item::Document(doc) => parse_document(answer, ctx, doc).map(|lite| (lite, None)),
        }
    }
}

// ----------------------------------------------------------------------------
#[derive(Default)]
struct Counts {
    /// Mails written with what was found.
    analysed: usize,
    /// Medium's chats written.
    chats: usize,
    /// Mails the answer had nothing new for, or gone meanwhile.
    unchanged: usize,
    failed: usize,
    /// Mails not analysed: stopped, or the spend limit reached.
    left: usize,
    /// The first failure, in words, for the notice.
    first_error: Option<String>,
    limit: bool,
}

impl Counts {
    // ------------------------------------------------------------------------
    /// "18 analysed · 4 chats · 1 unchanged · 1 failed".
    fn summary(&self) -> String {
        let counts = [
            (self.analysed, "analysed", "analysed"),
            (self.chats, "chat", "chats"),
            (self.unchanged, "unchanged", "unchanged"),
            (self.failed, "failed", "failed"),
            (self.left, "left", "left"),
        ];
        count_summary(&counts, "nothing to analyse")
    }
}

// ----------------------------------------------------------------------------
/// Analyse the mails `mails` of `email`'s account, each with its
/// attachments' text, then the attachments read `documents`, as `analysis`
/// says (its level and model; an attachment as Lite) as a job ("Analysing
/// mail · <email>"). Nothing to analyse starts nothing; a refused pick is a
/// notice; what is no longer in the vault, and an attachment without text,
/// is left out.
pub fn analyse(
    app: &mut App,
    email: &str,
    mails: Vec<Id>,
    documents: Vec<Id>,
    analysis: &Analysis,
) {
    if mails.is_empty() && documents.is_empty() {
        return;
    }
    let choice = analysis.model.as_ref();
    let resolved = match app.ai.choose(choice, MAIL_ANALYSIS, DateTime::now()) {
        Ok(resolved) => resolved,
        Err(refusal) => {
            return app.notice(&format!("Mail analysis · {email}: {}", refusal.message));
        }
    };
    let index = app.vault.index();
    let mails = mails.iter().filter_map(|id| index.entry(id)).map(|entry| {
        let mut mail = MailText::of(&entry.doc);
        mail.attachments = attachment_texts(index, mail.id);
        Item::Mail(mail)
    });
    let documents = documents
        .iter()
        .filter_map(|id| index.entry(id))
        .filter(|entry| !entry.doc.content().trim().is_empty())
        .map(|entry| Item::Document(DocText::of(index, &entry.doc)));
    let queue: VecDeque<Ask> = mails
        .chain(documents)
        .map(|item| Ask {
            item,
            attempt: 1,
            unread: None,
        })
        .collect();
    if queue.is_empty() {
        return;
    }
    let total = queue.len();
    let attachments = queue
        .iter()
        .filter(|ask| matches!(ask.item, Item::Document(_)))
        .count();
    let noun = match (total - attachments, attachments) {
        (_, 0) => ("mail", "mails"),
        (0, _) => ("attachment", "attachments"),
        _ => ("mail or attachment", "mails and attachments"),
    };
    let label = format!("Analysing mail · {email}");
    let job = app.start_job(JobKind::MailAnalysis, label, total);
    log::info!(
        "Mail analysis {job}: {} mails and {attachments} attachments of {email} with {}, {:?}",
        total - attachments,
        resolved.model,
        analysis.level
    );
    let run = Run {
        email: email.to_string(),
        account: resolved.account,
        client: resolved.client,
        provider: resolved.provider,
        address: resolved.address,
        model: resolved.model,
        level: analysis.level,
        context: context(app.vault.index()),
        queue,
        current: None,
        total,
        counts: Counts::default(),
        noun,
    };
    app.enrich.runs.insert(job, run);
    pump(app, job);
}

// ----------------------------------------------------------------------------
/// Send the next mail of the analysis `job`, or finish it if none is left.
fn pump(app: &mut App, job: Id) {
    let Some(run) = app.enrich.runs.get(&job) else {
        return;
    };
    if run.current.is_some() {
        return;
    }
    if run.queue.is_empty() {
        return finish(app, job);
    }
    // The mails analysed so far are counted (`account_used`): at its limit
    // the account takes no more.
    if let Err(refusal) = app.ai.check_limit(run.account, DateTime::now()) {
        return stop_at_limit(app, job, &refusal.message);
    }
    let Some(run) = app.enrich.runs.get_mut(&job) else {
        return;
    };
    let Some(ask) = run.queue.pop_front() else {
        return;
    };
    let mut request = ask.item.request(run.level, &run.model, &run.context);
    if let Some(answer) = &ask.unread {
        request = again(request, answer, ask.item.shape(run.level));
    }
    let client = run.client.clone();
    run.current = Some(ask);
    app.run_job(
        job,
        // Streamed like every other call (the deltas aren't needed): one
        // code path per provider.
        move |cancel| client.chat_stream(cancel, &request, |_| {}),
        move |app, result| on_answer(app, job, result),
    );
}

// ----------------------------------------------------------------------------
/// The model answered for the mail or attachment being analysed: write what
/// it found, or count the failure; then the next. An answer that isn't the JSON
/// object is asked again, up to `LITE_ATTEMPTS` times in all, unless it was
/// cut off. Every answer is logged whole with the model's reasoning (debug;
/// info when it can't be read). Ignored if the run is gone (stopped).
fn on_answer(app: &mut App, job: Id, result: Result<ChatResponse>) {
    let Some(run) = app.enrich.runs.get_mut(&job) else {
        return;
    };
    let Some(ask) = run.current.take() else {
        return;
    };
    let (id, attempt) = (ask.item.id(), ask.attempt);
    let account = run.account;
    let found = match result {
        Ok(response) => {
            let found = ask.item.parse(&response.text, run.level, &run.context);
            let (text, stop, thinking) = (&response.text, &response.stop, &response.thinking);
            match &found {
                Ok(_) => log::debug!(
                    "Mail analysis {job}: {id}, attempt {attempt}, answered ({stop:?}): {text:?}, reasoning: {thinking:?}"
                ),
                Err(_) => log::info!(
                    "Mail analysis {job}: {id}, attempt {attempt}, an answer that isn't the JSON object ({stop:?}): {text:?}, reasoning: {thinking:?}"
                ),
            }
            let cut = response.stop == StopReason::MaxTokens;
            if found.is_err() && !cut && attempt < LITE_ATTEMPTS {
                run.queue.push_front(Ask {
                    item: ask.item,
                    attempt: attempt + 1,
                    unread: Some(response.text),
                });
                ai::account_used(app, account);
                return pump(app, job);
            }
            found.map_err(|e| match attempt {
                _ if cut => CUT_OFF.to_string(),
                1 => e,
                n => format!("{e} (asked {n} times)"),
            })
        }
        Err(e) => Err(ai::chat_error(run.provider, &run.address, &e)),
    };
    let item = ask.item;
    let by = (run.model.clone(), account);
    let written = found.and_then(|(lite, medium)| write(app, &item, lite, medium, by));

    let Some(run) = app.enrich.runs.get_mut(&job) else {
        return;
    };
    let counts = &mut run.counts;
    match written {
        Ok(Written {
            mail: false,
            chat: false,
        }) => counts.unchanged += 1,
        Ok(Written { chat, .. }) => {
            counts.analysed += 1;
            counts.chats += usize::from(chat);
        }
        Err(error) => {
            log::info!("Mail analysis {job}: {id} failed: {error}");
            counts.failed += 1;
            counts
                .first_error
                .get_or_insert_with(|| format!("\u{201c}{}\u{201d}: {error}", item.title()));
        }
    }
    let done = counts.analysed + counts.unchanged + counts.failed;
    let total = run.total;
    // Saved per mail, so a long analysis keeps what it spent.
    ai::account_used(app, account);
    app.update_job(job, done, total);
    pump(app, job);
}

// ----------------------------------------------------------------------------
/// What writing an answer changed: the mail, and whether Medium's chat was
/// written.
struct Written {
    mail: bool,
    chat: bool,
}

// ----------------------------------------------------------------------------
/// Write `lite` into the mail or attachment `item`, then a mail's `medium`
/// as a chat linked to it, by the model and account `by`, titled "Mail
/// analysis · <the mail's title>". A title or an attachment's date the user
/// changed since the plan stays; what is gone meanwhile gets nothing. A
/// failed write is logged.
fn write(
    app: &mut App,
    item: &Item,
    mut lite: Lite,
    medium: Option<Medium>,
    by: (String, Id),
) -> std::result::Result<Written, String> {
    let gone = Written {
        mail: false,
        chat: false,
    };
    let id = item.id();
    let Some(entry) = app.vault.index().entry(&id) else {
        return Ok(gone);
    };
    if entry.doc.title() != item.title() {
        lite.title = None;
    }
    if let Item::Document(doc) = item
        && entry.doc.date() != doc.date
    {
        lite.date = None;
    }
    let failed = |e: Error| {
        log::warn!("Mail analysis: {id} could not be written: {e}");
        "could not be written.".to_string()
    };
    let outcome = app.edit(|index, changes| {
        index.edit(&id, changes, |doc| match item {
            Item::Mail(_) => apply_lite(doc, &lite),
            Item::Document(_) => apply_document(doc, &lite),
        })
    });
    let changed = match outcome {
        Ok(SaveOutcome::Saved) => true,
        Ok(SaveOutcome::Unchanged) => false,
        Err(Error::NotFound) => return Ok(gone),
        Ok(outcome) => match app.vault.require_written(outcome) {
            Ok(()) => true,
            Err(e) => return Err(failed(e)),
        },
        Err(e) => return Err(failed(e)),
    };
    let (Some(medium), Some(entry)) = (medium, app.vault.index().entry(&id)) else {
        return Ok(Written {
            mail: changed,
            chat: false,
        });
    };
    let title = plain_title(
        &format!("Mail analysis \u{b7} {}", entry.doc.title()),
        CHAT_TITLE_CHARS,
    );
    let (model, account) = by;
    let messages = medium_messages(&medium, &model, account, DateTime::now());
    match chat::create(app, title, messages, Some(id)) {
        Ok(_) => Ok(Written {
            mail: changed,
            chat: true,
        }),
        Err(e) => {
            log::warn!("Mail analysis: the chat of {id} could not be written: {e}");
            Err("the chat could not be written.".to_string())
        }
    }
}

// ----------------------------------------------------------------------------
/// Stop the analysis `job` (`jobs::stop` has cancelled its request): the
/// mail being analysed and those waiting are left, and the job ends.
pub fn stop(app: &mut App, job: Id) {
    let Some(run) = app.enrich.runs.get_mut(&job) else {
        return;
    };
    let left = run.queue.len() + usize::from(run.current.is_some());
    run.queue.clear();
    run.current = None;
    run.counts.left += left;
    log::info!("Mail analysis {job}: stopped");
    finish(app, job);
}

// ----------------------------------------------------------------------------
/// The account has reached its spend limit (`message` says so): say it
/// once, and stop the analysis as Stop does.
fn stop_at_limit(app: &mut App, job: Id, message: &str) {
    let Some(run) = app.enrich.runs.get_mut(&job) else {
        return;
    };
    run.counts.limit = true;
    let notice = format!("Mail analysis · {}: {message}", run.email);
    app.notice(&notice);
    stop(app, job);
}

// ----------------------------------------------------------------------------
/// The analysis `job` is done or stopped: end the job with its summary,
/// record what it spent, and tell the failures in one notice.
fn finish(app: &mut App, job: Id) {
    let Some(run) = app.enrich.runs.remove(&job) else {
        return;
    };
    let counts = &run.counts;
    let mut note = counts.summary();
    if counts.limit {
        note.push_str(" · spend limit reached");
    } else if app.job_stopping(job) {
        note = format!("stopped · {note}");
    }
    log::info!("Mail analysis {job}: {note}");
    app.end_job(job, &note);
    ai::account_used(app, run.account);
    if let Some(error) = &counts.first_error {
        let (failed, total) = (counts.failed, run.total);
        let what = if total == 1 { run.noun.0 } else { run.noun.1 };
        app.notice(&format!(
            "Mail analysis · {}: {failed} of {total} {what} couldn't be analysed; {error}",
            run.email
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_accounts::ModelChoice;
    use crate::testing::{FakeOllama, Harness, parse as parse_sent};
    use av_vault::document::FileFormat;
    use av_vault::envelope::{Email, Roles, Source};
    use av_vault::shared::OneOrMany;
    use serde_json::json;
    use std::fs;
    use std::sync::{Arc, Mutex};

    const OLLAMA_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b60";
    const MAIL_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b61";
    const MAIL_2: &str = "01a0e1cc-a24c-7490-82fe-e75669985b62";
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
            genres: vec!["bill".into(), "letter".into(), "Steuerbescheid".into()],
        }
    }

    // ------------------------------------------------------------------------
    fn mail_file(id: &str, subject: &str, tags: &str) -> String {
        format!(
            "---\nid: {id}\ntitle: '{subject}'\n{tags}email:\n  from: Stadtwerke <info@stadtwerke.example>\n---\nIhre Abschlagszahlung ist fällig.\n"
        )
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_context_holds_the_labels_in_use_and_the_active_projects() {
        let files = [
            (
                "Mail/a.md",
                mail_file(MAIL_ID, "A", "tags: [Umzug, Rechnungen]\n"),
            ),
            ("Mail/b.md", mail_file(MAIL_2, "B", "tags: [rechnungen]\n")),
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
                "---\ntitle: S\ndocument:\n  genre: Steuerbescheid\n---\n".to_string(),
            ),
            (
                "Documents/b.md",
                "---\ntitle: B\ndocument:\n  genre: BILL\n---\n".to_string(),
            ),
        ];
        let h = Harness::with_vault("enrich-context", &files);
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
        assert!(ctx.genres.starts_with(&["bill".to_string()]));
        assert!(!ctx.genres.iter().any(|g| g == "note"), "never a note");
        assert_eq!(
            ctx.genres.last().map(String::as_str),
            Some("Steuerbescheid")
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_request_lists_the_choices_and_cuts_the_body() {
        let mail = MailText {
            id: Id::generate(),
            title: "Abschlag".into(),
            subject: "AW: [#4711] Abschlag".into(),
            from: "info@stadtwerke.example".into(),
            to: "jan@example.com".into(),
            date: None,
            body: "Body".into(),
            attachments: Vec::new(),
        };
        let request = request_for(AnalysisLevel::Lite, "m", &ctx(), &mail);
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
            system.contains("- kind:") && !system.contains("- check:"),
            "{system}"
        );
        let medium = request_for(AnalysisLevel::Medium, "m", &ctx(), &mail);
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
        let empty = request_for(AnalysisLevel::Lite, "m", &Context::default(), &mail);
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
        let text = MailText::of(&doc);
        assert_eq!(text.body.chars().count(), BODY_CHARS + 4, "cut, marked");
        assert!(text.body.ends_with("\n[\u{2026}]"));
        assert_eq!((text.subject.as_str(), text.to.as_str()), ("T", ""));

        doc.set_email(Some(Email {
            subject: Some("AW: T".into()),
            to: OneOrMany::from_vec(vec!["a@example.com".into(), "b@example.com".into()]),
            ..Default::default()
        }));
        let text = MailText::of(&doc);
        assert_eq!(
            (text.title.as_str(), text.subject.as_str(), text.to.as_str()),
            ("T", "AW: T", "a@example.com, b@example.com"),
            "the subject as the mail came"
        );
    }

    // ------------------------------------------------------------------------
    /// The attachment `Rechnung.pdf` of the mail "Abschlag", read to `body`,
    /// dated `date`.
    fn doc_text(body: &str, date: Option<DateTime>) -> DocText {
        DocText {
            id: Id::generate(),
            title: "Rechnung".into(),
            name: "Rechnung.pdf".into(),
            date,
            mail: Some(("Abschlag".into(), "info@stadtwerke.example".into())),
            body: body.into(),
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_attachment_is_asked_for_as_a_document() {
        let request = document_request("m", &ctx(), &doc_text("Betrag 42 EUR", None));
        let system = request.system.as_deref().unwrap();
        assert!(system.starts_with("You sort documents.") && system.contains(DOCUMENT_SHAPE));
        assert!(
            system.contains("- genre:") && !system.contains("- kind:"),
            "{system}"
        );
        assert_eq!(request.think, Some(false));
        let Some(av_llm::Part::Text(text)) = request.messages[0].parts.first() else {
            panic!("{:?}", request.messages);
        };
        assert!(
            text.starts_with("Genres:\n- bill\n- letter\n- Steuerbescheid\n\nLabels:\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "The document:\nFile: Rechnung.pdf\nAttached to the mail: Abschlag (from info@stadtwerke.example)\n\nBetrag 42 EUR"
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
    fn an_attachments_answer_takes_a_listed_genre_and_its_own_date() {
        let mail_date = DateTime::from_ymd_hms(2026, 10, 12, 9, 30, 0).unwrap();
        let doc = doc_text("", Some(mail_date));
        let read = |answer: &str| parse_document(answer, &ctx(), &doc).unwrap();
        let lite = read(
            r#"{"title": "Stadtwerke Abschlag Oktober", "date": "2026-10-01", "genre": "BILL", "labels": ["rechnungen"], "projects": ["Auto"]}"#,
        );
        assert_eq!(lite.title.as_deref(), Some("Stadtwerke Abschlag Oktober"));
        assert_eq!(lite.genre.as_deref(), Some("bill"), "as the list spells it");
        assert_eq!(
            lite.date,
            Some(DateTime::from_ymd_hms(2026, 10, 1, 12, 0, 0).unwrap()),
            "that day at noon UTC"
        );
        assert_eq!(lite.labels, ["Rechnungen"]);
        assert_eq!(lite.projects, [Id::parse(OLD).unwrap()]);
        assert_eq!(lite.kind, None);

        assert_eq!(
            read(r#"{"genre": "Mahnung"}"#).genre,
            None,
            "only from the list"
        );
        assert_eq!(read(r#"{"genre": null, "date": null}"#), Lite::default());
        let date = |date: &str| read(&format!(r#"{{"date": "{date}"}}"#)).date;
        assert!(
            date("2026-10-13T08:00:00Z").is_some(),
            "a day after the mail"
        );
        assert_eq!(date("2026-10-14"), None, "later than the mail");
        assert_eq!(date("1899-12-31"), None);
        assert_eq!(date("2026-13-01"), None);
        assert_eq!(date("1.10.2026"), None);
        assert_eq!(
            read(r#"{"title": "Rechnung"}"#).title,
            None,
            "its own title"
        );
        assert!(parse_document("Sorry.", &ctx(), &doc).is_err());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn applying_to_an_attachment_keeps_a_genre_it_has() {
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
            genre: Some("bill".into()),
            date: Some(date),
            labels: vec!["Rechnungen".into()],
            ..Lite::default()
        };
        apply_document(&mut doc, &lite);
        assert_eq!(
            (doc.title(), doc.date()),
            ("Stadtwerke Abschlag", Some(date))
        );
        let genre = |doc: &Document| doc.roles().document.clone().unwrap().genre;
        assert_eq!(genre(&doc), Some(Genre::Bill));
        assert_eq!(doc.tags(), ["Rechnungen"]);
        let letter = Lite {
            genre: Some("letter".into()),
            ..Lite::default()
        };
        apply_document(&mut doc, &letter);
        assert_eq!(genre(&doc), Some(Genre::Bill), "a genre it has stays");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_answer_is_read_against_the_lists() {
        let answer = "Sure! ```json\n{\"title\": \"  Abschlag Oktober \", \"labels\": [\"rechnungen\", \"Unbekannt\", \"Rechnungen\", \"Umzug\", \"Steuer\", \"Bank\"], \"projects\": [\"wohnung köln\", \"Garten\", \"Auto\", \"Wohnung Köln\"]}\n``` Done.";
        let (lite, medium) = parse(answer, AnalysisLevel::Lite, &ctx(), "AW: Abschlag").unwrap();
        assert_eq!(lite.title.as_deref(), Some("Abschlag Oktober"));
        assert_eq!((lite.kind, medium), (None, None));
        assert_eq!(
            lite.labels,
            ["Rechnungen", "Umzug", "Steuer"],
            "known, once, three"
        );
        let projects = [Id::parse(FLAT).unwrap(), Id::parse(OLD).unwrap()];
        assert_eq!(lite.projects, projects);

        let lite = |answer: &str, title: &str| parse(answer, AnalysisLevel::Lite, &ctx(), title);
        let same = lite(
            r#"{"kind": " Request", "title": " Abschlag ", "labels": "Bank"}"#,
            "Abschlag",
        );
        assert_eq!(
            same.unwrap().0,
            Lite {
                title: None,
                kind: Some("request".into()),
                labels: vec!["Bank".into()],
                projects: vec![],
                ..Lite::default()
            },
            "the mail's own title is no change; one label as a string; the kind in any case"
        );
        let unknown = lite(r#"{"kind": "spam"}"#, "t").unwrap().0;
        assert_eq!(unknown.kind, None, "only the known kinds");
        let long = format!(r#"{{"title": "{}"}}"#, "x".repeat(200));
        let cut = lite(&long, "t").unwrap().0.title.unwrap();
        assert_eq!(cut.chars().count(), TITLE_CHARS);
        assert_eq!(lite(r#"{"title": "  "}"#, "t").unwrap().0.title, None);

        for bad in ["Sorry, I can't.", "{not json}", "[1, 2]", "} {"] {
            let refused = lite(bad, "t");
            assert_eq!(
                refused,
                Err("The answer wasn't the JSON object asked for.".to_string()),
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
            r#"{{"kind": "request", "check": [" Der Betrag ", "", "{long}", "Die Frist", "Zu viel"], "reply": " Danke! ", "questions": "Warum?"}}"#
        );
        let found = medium(&answer).unwrap();
        assert_eq!(found.check.len(), CHECKS_PER_MAIL, "{found:?}");
        assert_eq!(found.check[0], "Der Betrag");
        assert_eq!(found.check[1].chars().count(), POINT_CHARS, "cut");
        assert_eq!(found.reply.as_deref(), Some("Danke!"));
        assert_eq!(found.questions, ["Warum?"]);

        let notification =
            medium(r#"{"kind": "notification", "check": ["Der Betrag"], "reply": "Danke!"}"#);
        assert_eq!(
            notification,
            Some(Medium {
                check: vec!["Der Betrag".into()],
                reply: None,
                questions: vec![],
            }),
            "no reply to a notification"
        );
        let newsletter = r#"{"kind": "newsletter", "check": ["Rabatt"], "reply": "Nein", "questions": ["Was?"]}"#;
        assert_eq!(medium(newsletter), None, "nothing for a newsletter");
        let nothing = r#"{"kind": "request", "reply": null, "check": [], "questions": ["Was?"]}"#;
        assert_eq!(medium(nothing), None, "questions alone make no chat");
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
    fn applying_keeps_the_subject_and_adds_to_what_the_mail_has() {
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
        doc.set_tags(vec!["umzug".into()]);
        let flat = Id::parse(FLAT).unwrap();
        doc.join_project(flat);
        let lite = Lite {
            title: Some("Abschlag Oktober".into()),
            kind: Some("request".into()),
            labels: vec!["Umzug".into(), "Rechnungen".into()],
            projects: vec![flat, Id::parse(OLD).unwrap()],
            ..Lite::default()
        };
        apply_lite(&mut doc, &lite);
        assert_eq!(doc.title(), "Abschlag Oktober");
        let email = doc.roles().email.clone().unwrap();
        assert_eq!(email.subject.as_deref(), Some("AW: Abschlag"));
        assert_eq!(email.kind.as_deref(), Some("request"));
        assert_eq!(doc.tags(), ["umzug", "Rechnungen"]);
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
        assert_eq!(email.kind.as_deref(), Some("request"), "no kind keeps it");
    }

    // ------------------------------------------------------------------------
    /// A vault with an Ollama account at `address` offering `m` (with
    /// `extra` lines in its block), two mails, a mail with labels and a
    /// project.
    fn with_mails(name: &str, address: &str, extra: &str) -> Harness {
        let account =
            format!("id: {OLLAMA_ID}\nollama:\n  address: {address}\n  offered: [m]\n{extra}");
        let files = [
            (".avelor/accounts/ollama.yaml", account),
            ("Mail/a.md", mail_file(MAIL_ID, "AW: [#4711] Abschlag", "")),
            ("Mail/b.md", mail_file(MAIL_2, "Abschlag", "")),
            (
                "Mail/c.md",
                mail_file(&Id::generate().to_string(), "C", "tags: [Rechnungen]\n"),
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
        let answer = r#"{"kind": "request", "title": "Abschlag Oktober", "labels": ["rechnungen"], "projects": ["Wohnung"], "reply": "Danke!"}"#;
        let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
        let mut h = with_mails("enrich-run", &address, "");
        let ids = vec![Id::parse(MAIL_ID).unwrap()];
        analyse(
            &mut h.app,
            "jan@example.com",
            ids,
            Vec::new(),
            &at(AnalysisLevel::Lite),
        );
        let sent = until_done(&mut h);

        let jobs = of(&sent, "job");
        assert_eq!(jobs[0]["label"], "Analysing mail · jan@example.com");
        assert_eq!(jobs.last().unwrap()["note"], "1 analysed");
        assert!(of(&sent, "notice").is_empty(), "{sent:?}");
        assert_eq!(of(&sent, "entity")[0]["title"], "Abschlag Oktober");
        let text = fs::read_to_string(h.vault().join("Mail/a.md")).unwrap();
        assert!(text.contains("title: Abschlag Oktober"), "{text}");
        assert!(text.contains("subject: 'AW: [#4711] Abschlag'"), "{text}");
        assert!(
            text.contains("- Rechnungen") && text.contains(FLAT),
            "{text}"
        );
        assert!(text.contains("kind: request"), "{text}");
        assert!(
            !h.vault().join("Conversations").exists(),
            "Lite writes no chat"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn medium_writes_a_chat_linked_to_the_mail() {
        let answer = r#"{"kind": "request", "title": "Abschlag Oktober", "check": ["Der Betrag ist neu."], "reply": "Danke, ist bezahlt.", "questions": ["Warum ist er hoeher?"]}"#;
        let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
        let mut h = with_mails("enrich-medium", &address, "");
        let mail = Id::parse(MAIL_ID).unwrap();
        analyse(
            &mut h.app,
            "jan@example.com",
            vec![mail],
            Vec::new(),
            &at(AnalysisLevel::Medium),
        );
        let sent = until_done(&mut h);

        assert_eq!(
            of(&sent, "job").last().unwrap()["note"],
            "1 analysed · 1 chat"
        );
        assert!(of(&sent, "notice").is_empty(), "{sent:?}");
        let text = fs::read_to_string(h.vault().join("Mail/a.md")).unwrap();
        assert!(text.contains("kind: request"), "{text}");
        let chats: Vec<_> = fs::read_dir(h.vault().join("Conversations"))
            .unwrap()
            .collect();
        assert_eq!(chats.len(), 1);

        let chat = h
            .app
            .vault
            .index()
            .all()
            .iter()
            .find(|e| e.doc.linked_to() == Some(mail))
            .unwrap();
        let id = chat.doc.id();
        assert_eq!(chat.doc.title(), "Mail analysis · Abschlag Oktober");
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
    fn a_mail_and_its_attachment_are_analysed_in_one_job() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let answers = [
            r#"{"kind": "notification", "title": "Abschlag Oktober"}"#,
            r#"{"title": "Stadtwerke Rechnung", "date": "2026-10-01", "genre": "bill"}"#,
        ];
        let address = FakeOllama::new(&["m"])
            .answering_in_turn(&answers)
            .recording(&requests)
            .start();
        let account = format!("id: {OLLAMA_ID}\nollama:\n  address: {address}\n  offered: [m]\n");
        let bill = Id::generate();
        let attachment = format!(
            "---\nid: {bill}\ntitle: Rechnung\ndate: 2026-10-12T09:30:00Z\nlinked-to: {MAIL_ID}\ndocument:\n  original:\n    file: r.pdf\n    name: Rechnung.pdf\n---\nBetrag: 42,00 EUR\n"
        );
        let files = [
            (".avelor/accounts/ollama.yaml", account),
            ("Mail/a.md", mail_file(MAIL_ID, "Abschlag", "")),
            ("Attachments/r.md", attachment),
        ];
        let mut h = Harness::with_vault("enrich-attachment", &files);
        let mail = vec![Id::parse(MAIL_ID).unwrap()];
        analyse(
            &mut h.app,
            "jan@example.com",
            mail,
            vec![bill],
            &at(AnalysisLevel::Medium),
        );
        let sent = until_done(&mut h);

        let jobs = of(&sent, "job");
        assert_eq!(jobs[0]["total"], 2);
        assert_eq!(jobs.last().unwrap()["note"], "2 analysed");
        assert!(of(&sent, "notice").is_empty(), "{sent:?}");
        let requests = requests.lock().unwrap();
        let user = |i: usize| {
            requests[i]["messages"][1]["content"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert!(
            user(0).ends_with(
                "Its attached files, as read:\n\n--- Rechnung.pdf ---\nBetrag: 42,00 EUR"
            ),
            "{}",
            user(0)
        );
        assert!(user(1).contains("File: Rechnung.pdf\nAttached to the mail: Abschlag"));
        let text = fs::read_to_string(h.vault().join("Attachments/r.md")).unwrap();
        assert!(text.contains("title: Stadtwerke Rechnung"), "{text}");
        assert!(text.contains("date: 2026-10-01T12:00:00Z"), "{text}");
        assert!(text.contains("genre: bill"), "{text}");
        assert!(
            !h.vault().join("Conversations").exists(),
            "Medium writes no chat for an attachment, nor for this mail"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn nothing_to_analyse_starts_nothing() {
        let mut h = with_mails("enrich-nothing", "http://127.0.0.1:9", "");
        let unpicked = Analysis {
            level: AnalysisLevel::Lite,
            model: None,
        };
        analyse(
            &mut h.app,
            "jan@example.com",
            Vec::new(),
            Vec::new(),
            &unpicked,
        );
        assert!(h.take().is_empty(), "not even the refused pick");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_newsletter_gets_no_chat() {
        let answer = r#"{"kind": "newsletter", "title": "Angebote", "check": ["Rabatt"], "reply": "Nein", "questions": ["Was?"]}"#;
        let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
        let mut h = with_mails("enrich-newsletter", &address, "");
        let ids = vec![Id::parse(MAIL_ID).unwrap()];
        analyse(
            &mut h.app,
            "jan@example.com",
            ids,
            Vec::new(),
            &at(AnalysisLevel::Medium),
        );
        let sent = until_done(&mut h);

        assert_eq!(of(&sent, "job").last().unwrap()["note"], "1 analysed");
        let text = fs::read_to_string(h.vault().join("Mail/a.md")).unwrap();
        assert!(text.contains("kind: newsletter"), "{text}");
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
        let mut h = with_mails("enrich-again", &address, "");
        analyse(
            &mut h.app,
            "jan@example.com",
            vec![Id::parse(MAIL_ID).unwrap()],
            Vec::new(),
            &at(AnalysisLevel::Lite),
        );
        let sent = until_done(&mut h);

        assert_eq!(of(&sent, "job").last().unwrap()["note"], "1 analysed");
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
        let mut h = with_mails("enrich-cut", &address, "");
        let ids = vec![Id::parse(MAIL_ID).unwrap()];
        analyse(
            &mut h.app,
            "jan@example.com",
            ids,
            Vec::new(),
            &at(AnalysisLevel::Lite),
        );
        let sent = until_done(&mut h);

        assert_eq!(of(&sent, "job").last().unwrap()["note"], "1 failed");
        let message = of(&sent, "notice")[0]["message"].as_str().unwrap();
        assert!(message.ends_with(CUT_OFF), "{message}");
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
        let mut h = with_mails("enrich-bad", &address, "");
        let before = fs::read_to_string(h.vault().join("Mail/a.md")).unwrap();
        let ids = vec![Id::parse(MAIL_ID).unwrap(), Id::parse(MAIL_2).unwrap()];
        analyse(
            &mut h.app,
            "jan@example.com",
            ids,
            Vec::new(),
            &at(AnalysisLevel::Lite),
        );
        let sent = until_done(&mut h);

        assert_eq!(of(&sent, "job").last().unwrap()["note"], "2 failed");
        let notices = of(&sent, "notice");
        assert_eq!(notices.len(), 1, "{sent:?}");
        let message = notices[0]["message"].as_str().unwrap();
        assert!(
            message
                .starts_with("Mail analysis · jan@example.com: 2 of 2 mails couldn't be analysed;"),
            "{message}"
        );
        assert!(
            message.contains("wasn't the JSON object asked for. (asked 3 times)"),
            "{message}"
        );
        assert_eq!(
            requests.lock().unwrap().len(),
            6,
            "each mail asked three times"
        );
        assert_eq!(
            fs::read_to_string(h.vault().join("Mail/a.md")).unwrap(),
            before
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_spend_limit_stops_it_between_mails() {
        let answer = r#"{"title": "Abschlag Oktober"}"#;
        let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
        let mut h = with_mails("enrich-limit", &address, "");
        let ids = vec![Id::parse(MAIL_ID).unwrap(), Id::parse(MAIL_2).unwrap()];
        analyse(
            &mut h.app,
            "jan@example.com",
            ids,
            Vec::new(),
            &at(AnalysisLevel::Lite),
        );
        // The first mail is on its way; then the limit is reached.
        let account = Id::parse(OLLAMA_ID).unwrap();
        h.app
            .ai
            .update(account, |s| s.spend_limit = Some(0.0))
            .unwrap();
        let sent = until_done(&mut h);

        let note = of(&sent, "job").last().unwrap()["note"].clone();
        assert_eq!(note, "1 analysed · 1 left · spend limit reached");
        let notices = of(&sent, "notice");
        let message = notices[0]["message"].as_str().unwrap();
        assert!(
            message
                .starts_with("Mail analysis · jan@example.com: Ollama has reached its spend limit"),
            "{message}"
        );
        let text = fs::read_to_string(h.vault().join("Mail/b.md")).unwrap();
        assert!(
            text.contains("title: 'Abschlag'"),
            "the second mail stays: {text}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_refused_pick_is_told_and_starts_nothing() {
        let mut h = with_mails("enrich-no-pick", "http://127.0.0.1:9", "");
        let unpicked = Analysis {
            level: AnalysisLevel::Lite,
            model: None,
        };
        analyse(
            &mut h.app,
            "jan@example.com",
            vec![Id::parse(MAIL_ID).unwrap()],
            Vec::new(),
            &unpicked,
        );
        let sent = parse_sent(&h.take());
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(
            sent[0]["data"],
            json!({ "level": "error", "message": "Mail analysis · jan@example.com: For the mail analysis, pick a model." })
        );
        assert!(h.app.enrich.runs.is_empty());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn stop_ends_it_and_a_late_answer_is_dropped() {
        let answer = r#"{"title": "Abschlag Oktober"}"#;
        let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
        let mut h = with_mails("enrich-stop", &address, "");
        let ids = vec![Id::parse(MAIL_ID).unwrap(), Id::parse(MAIL_2).unwrap()];
        analyse(
            &mut h.app,
            "jan@example.com",
            ids,
            Vec::new(),
            &at(AnalysisLevel::Lite),
        );
        let job = *h.app.enrich.runs.keys().next().unwrap();
        crate::jobs::stop(&mut h.app, job);
        let sent = parse_sent(&h.take());
        let last = of(&sent, "job").last().unwrap()["note"].clone();
        assert_eq!(last, "stopped · 2 left");
        assert!(h.app.enrich.runs.is_empty());
        // Whatever the cancelled request brings back finds no run.
        h.run_queue();
        let text = fs::read_to_string(h.vault().join("Mail/a.md")).unwrap();
        assert!(text.contains("title: 'AW: [#4711] Abschlag'"), "{text}");
    }
}
