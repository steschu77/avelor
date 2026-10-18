//! Enriching entities with an AI (mail.md#analysis): the mail analysis -- Lite
//! gives each new mail its kind, a clean title, labels in use and active
//! projects; Medium also points to check, a quick reply and questions, as a
//! chat linked to the mail.
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

// ----------------------------------------------------------------------------
/// What the vault offers an analysis to pick from: the labels in use and
/// the active projects.
#[derive(Debug, Default)]
pub struct Context {
    /// As the vault spells them, the most used first.
    pub labels: Vec<String>,
    /// Id and title, in the vault's order.
    pub projects: Vec<(Id, String)>,
}

// ----------------------------------------------------------------------------
/// The context of the vault `index`: its labels by how many entities carry them (a
/// label in other case counts as the same, spelled as first seen), and its
/// projects that aren't archived; each list capped (the labels used most,
/// the first projects).
pub fn context(index: &Index) -> Context {
    let mut labels: Vec<(String, usize)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut projects = Vec::new();
    for entry in index.all() {
        let doc = &entry.doc;
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
    Context {
        labels: labels.into_iter().map(|(label, _)| label).collect(),
        projects,
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
}

impl MailText {
    // ------------------------------------------------------------------------
    pub fn of(doc: &Document) -> Self {
        let email = doc.roles().email.as_ref();
        let content = doc.content();
        let mut body: String = content.chars().take(BODY_CHARS).collect();
        if body.len() < content.len() {
            body.push_str("\n[\u{2026}]");
        }
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
        }
    }
}

// ----------------------------------------------------------------------------
/// The request of the analysis at `level` for `mail` to `model`: the
/// instructions, then the lists of `ctx` and the mail.
pub fn request_for(
    level: AnalysisLevel,
    model: &str,
    ctx: &Context,
    mail: &MailText,
) -> ChatRequest {
    let list = |names: Vec<&str>| match names.is_empty() {
        true => "(none)\n".to_string(),
        false => names.iter().map(|n| format!("- {n}\n")).collect(),
    };
    let labels = list(ctx.labels.iter().map(String::as_str).collect());
    let projects = list(ctx.projects.iter().map(|(_, t)| t.as_str()).collect());
    let date = mail.date.map(|d| d.as_iso8601()).unwrap_or_default();
    let text = format!(
        "Labels:\n{labels}\nProjects:\n{projects}\nThe mail:\nSubject: {}\nFrom: {}\nTo: {}\nDate: {date}\n\n{}",
        mail.subject, mail.from, mail.to, mail.body
    );
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
/// The JSON object the analysis at `level` asks for.
fn shape(level: AnalysisLevel) -> &'static str {
    match level {
        AnalysisLevel::Medium => MEDIUM_SHAPE,
        _ => LITE_SHAPE,
    }
}

// ----------------------------------------------------------------------------
/// `request` asked again after `answer`, which wasn't the JSON object: the
/// answer and a reminder of the object `level` asks for follow it. An empty
/// answer is just asked again (providers refuse an empty turn).
pub fn again(request: ChatRequest, answer: &str, level: AnalysisLevel) -> ChatRequest {
    if answer.trim().is_empty() {
        return request;
    }
    let reminder = format!(
        "That wasn't the JSON object asked for. Answer again with only the JSON object {}, \
nothing before or after it.",
        shape(level)
    );
    request
        .message(Message::assistant(answer))
        .message(Message::user(reminder))
}

// ----------------------------------------------------------------------------
/// What Lite found for a mail, in the vault's terms.
#[derive(Debug, Default, PartialEq)]
pub struct Lite {
    /// `None`: the title stays.
    pub title: Option<String>,
    /// One of `KINDS`.
    pub kind: Option<String>,
    /// Labels in use, as the vault spells them.
    pub labels: Vec<String>,
    /// Active projects.
    pub projects: Vec<Id>,
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
    let refused = || "The answer wasn't the JSON object asked for.".to_string();
    let (Some(start), Some(end)) = (answer.find('{'), answer.rfind('}')) else {
        return Err(refused());
    };
    let object: Value = match answer.get(start..=end).map(serde_json::from_str) {
        Some(Ok(object @ Value::Object(_))) => object,
        _ => return Err(refused()),
    };

    let new_title = object["title"]
        .as_str()
        .map(|t| cut(t, TITLE_CHARS))
        .filter(|t| !t.is_empty() && t != title.trim());
    let kind = object["kind"]
        .as_str()
        .and_then(|kind| KINDS.into_iter().find(|known| same(known, kind)))
        .map(str::to_string);

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

    let lite = Lite {
        title: new_title,
        kind,
        labels,
        projects,
    };
    let medium = match level {
        AnalysisLevel::Medium => medium_of(&object, lite.kind.as_deref()),
        _ => None,
    };
    Ok((lite, medium))
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
}

// ----------------------------------------------------------------------------
/// A mail to ask about: the how-manyth time, and the answer of the time
/// before, which wasn't the JSON object.
struct Ask {
    mail: MailText,
    attempt: u32,
    unread: Option<String>,
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
/// Analyse the mails `ids` of `email`'s account as `analysis` says (its
/// level and model) as a job ("Analysing mail · <email>"). A refused pick is
/// a notice; mails no longer in the vault are left out.
pub fn analyse(app: &mut App, email: &str, ids: Vec<Id>, analysis: &Analysis) {
    let choice = analysis.model.as_ref();
    let resolved = match app.ai.choose(choice, MAIL_ANALYSIS, DateTime::now()) {
        Ok(resolved) => resolved,
        Err(refusal) => {
            return app.notice(&format!("Mail analysis · {email}: {}", refusal.message));
        }
    };
    let queue: VecDeque<Ask> = ids
        .iter()
        .filter_map(|id| app.vault.index().entry(id))
        .map(|entry| Ask {
            mail: MailText::of(&entry.doc),
            attempt: 1,
            unread: None,
        })
        .collect();
    if queue.is_empty() {
        return;
    }
    let total = queue.len();
    let label = format!("Analysing mail · {email}");
    let job = app.start_job(JobKind::MailAnalysis, label, total);
    log::info!(
        "Mail analysis {job}: {total} mails of {email} with {}, {:?}",
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
    let mut request = request_for(run.level, &run.model, &run.context, &ask.mail);
    if let Some(answer) = &ask.unread {
        request = again(request, answer, run.level);
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
/// The model answered for the mail being analysed: write what it found, or
/// count the failure; then the next mail. An answer that isn't the JSON
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
    let (id, attempt) = (ask.mail.id, ask.attempt);
    let account = run.account;
    let found = match result {
        Ok(response) => {
            let found = parse(&response.text, run.level, &run.context, &ask.mail.title);
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
                    mail: ask.mail,
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
    let mail = ask.mail;
    let by = (run.model.clone(), account);
    let written = found.and_then(|(lite, medium)| write(app, &mail, lite, medium, by));

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
            log::info!("Mail analysis {job}: {} failed: {error}", mail.id);
            counts.failed += 1;
            counts
                .first_error
                .get_or_insert_with(|| format!("\u{201c}{}\u{201d}: {error}", mail.title));
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
/// Write `lite` into `mail`, then `medium` as a chat linked to it, by the
/// model and account `by`, titled "Mail analysis · <the mail's title>". A
/// title the user changed since the plan stays; a mail gone meanwhile gets
/// nothing. A failed write is logged.
fn write(
    app: &mut App,
    mail: &MailText,
    mut lite: Lite,
    medium: Option<Medium>,
    by: (String, Id),
) -> std::result::Result<Written, String> {
    let gone = Written {
        mail: false,
        chat: false,
    };
    let Some(entry) = app.vault.index().entry(&mail.id) else {
        return Ok(gone);
    };
    if entry.doc.title() != mail.title {
        lite.title = None;
    }
    let id = mail.id;
    let failed = |e: Error| {
        log::warn!("Mail analysis: {id} could not be written: {e}");
        "could not be written.".to_string()
    };
    let outcome = app.edit(|index, changes| index.edit(&id, changes, |doc| apply_lite(doc, &lite)));
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
        let mails = if total == 1 { "mail" } else { "mails" };
        app.notice(&format!(
            "Mail analysis · {}: {failed} of {total} {mails} couldn't be analysed; {error}",
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
        };
        let request = request_for(AnalysisLevel::Lite, "m", &ctx(), &mail);
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
        let asked = again(medium.clone(), "Sorry.", AnalysisLevel::Medium);
        let Some(av_llm::Part::Text(reminder)) = asked.messages[2].parts.first() else {
            panic!("{:?}", asked.messages);
        };
        assert!(reminder.contains(MEDIUM_SHAPE), "{reminder}");
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
        analyse(&mut h.app, "jan@example.com", ids, &at(AnalysisLevel::Lite));
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
    fn a_newsletter_gets_no_chat() {
        let answer = r#"{"kind": "newsletter", "title": "Angebote", "check": ["Rabatt"], "reply": "Nein", "questions": ["Was?"]}"#;
        let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
        let mut h = with_mails("enrich-newsletter", &address, "");
        let ids = vec![Id::parse(MAIL_ID).unwrap()];
        analyse(
            &mut h.app,
            "jan@example.com",
            ids,
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
        analyse(&mut h.app, "jan@example.com", ids, &at(AnalysisLevel::Lite));
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
        analyse(&mut h.app, "jan@example.com", ids, &at(AnalysisLevel::Lite));
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
        analyse(&mut h.app, "jan@example.com", ids, &at(AnalysisLevel::Lite));
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
        analyse(&mut h.app, "jan@example.com", ids, &at(AnalysisLevel::Lite));
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
