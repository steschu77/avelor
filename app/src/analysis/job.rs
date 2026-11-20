//! The analysis as a job: its queue of entities, the answers and their writes.

use std::collections::VecDeque;

use av_llm::{ChatRequest, ChatResponse, StopReason};
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::document::Document;
use av_vault::vault::SaveOutcome;

use super::answer::{Lite, Medium, parse, parse_document};
use super::apply::{apply_document, apply_lite, medium_messages};
use super::context::{Context, DocText, EmailText, attachment_texts, context};
use super::request::{
    DOCUMENT_MEDIUM_SHAPE, DOCUMENT_SHAPE, again, document_request, request_for, shape,
};
use super::{Analysis, AnalysisLevel, email_analysis, scan_analysis};
use crate::ai;
use crate::ai::accounts::Resolved;
use crate::ai::queue::AiQueue;
use crate::app::App;
use crate::chat;
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use crate::texts::{self, Text};
use crate::watches::WatchKind;
use av_vault::markdown::plain_title;

/// A longer title of Medium's conversation is cut.
const CHAT_TITLE_CHARS: usize = 60;
/// How often an email is asked about at most while the answer isn't the JSON
/// object asked for.
const LITE_ATTEMPTS: u32 = 3;

// ----------------------------------------------------------------------------
/// One analysis: its model, the vault's context, the emails left and the
/// one being analysed.
pub(super) struct Run {
    /// What its notices start with (`Batch::notices`).
    pub(super) notices: Text,
    pub(super) ai: Resolved,
    pub(super) level: AnalysisLevel,
    pub(super) context: Context,
    queue: AiQueue<Ask>,
    counts: Counts,
    /// How many of how many failed, of what it analyses, for the failures'
    /// notice (`AiQueue::failures`).
    failed: Text,
}

// ----------------------------------------------------------------------------
/// An email or an attachment to ask about: the how-manyth time, and the
/// answer of the time before, which wasn't the JSON object.
struct Ask {
    item: Item,
    attempt: u32,
    unread: Option<String>,
}

// ----------------------------------------------------------------------------
/// What an analysis asks about.
enum Item {
    Email(EmailText),
    /// An attachment, asked about as Lite at either level.
    Document(DocText),
}

impl Item {
    // ------------------------------------------------------------------------
    pub(super) fn id(&self) -> Id {
        match self {
            Item::Email(email) => email.id,
            Item::Document(doc) => doc.id,
        }
    }

    // ------------------------------------------------------------------------
    /// Its title when the analysis was planned.
    pub(super) fn title(&self) -> &str {
        match self {
            Item::Email(email) => &email.title,
            Item::Document(doc) => &doc.title,
        }
    }

    // ------------------------------------------------------------------------
    /// The JSON object asked for at `level`.
    fn shape(&self, level: AnalysisLevel) -> &'static str {
        match self {
            Item::Email(email) => shape(email.level(level)),
            Item::Document(doc) if doc.medium(level) => DOCUMENT_MEDIUM_SHAPE,
            Item::Document(_) => DOCUMENT_SHAPE,
        }
    }

    // ------------------------------------------------------------------------
    fn request(&self, level: AnalysisLevel, model: &str, ctx: &Context) -> ChatRequest {
        match self {
            Item::Email(email) => request_for(email.level(level), model, ctx, email),
            Item::Document(doc) => document_request(level, model, ctx, doc),
        }
    }

    // ------------------------------------------------------------------------
    pub(super) fn parse(
        &self,
        answer: &str,
        level: AnalysisLevel,
        ctx: &Context,
    ) -> std::result::Result<(Lite, Option<Medium>), Text> {
        match self {
            Item::Email(email) => parse(answer, email.level(level), ctx, &email.title),
            Item::Document(doc) => parse_document(answer, level, ctx, doc),
        }
    }
}

// ----------------------------------------------------------------------------
#[derive(Default)]
struct Counts {
    /// Emails written with what was found.
    analysed: usize,
    /// Medium's conversations written.
    pub(super) conversations: usize,
    /// Emails the answer had nothing new for, or gone meanwhile.
    unchanged: usize,
}

impl Counts {
    // ------------------------------------------------------------------------
    /// "18 analysed · 4 conversations · 1 unchanged · 1 failed", with the failures
    /// and what a stop left of `queue`.
    fn summary(&self, queue: &AiQueue<Ask>) -> Vec<Text> {
        let counts = vec![
            (self.analysed, Text::new("count.analysed")),
            (self.conversations, Text::new("count.conversations")),
            (self.unchanged, Text::new("count.unchanged")),
            (queue.failed(), Text::new("count.failed")),
            (queue.left(), Text::new("count.left")),
        ];
        count_summary(counts, Text::new("job.nothing_to_analyse"))
    }
}

// ----------------------------------------------------------------------------
/// What an analysis takes, and how its job and notices name it.
pub struct Batch {
    /// The job's label: "Analysing email · <email>".
    pub(super) label: Text,
    /// What its notices start with: "Email analysis · <email>".
    pub(super) notices: Text,
    /// What it is called where its pick is refused (`AiAccounts::choose`).
    pub(super) what: Text,
    emails: Vec<Id>,
    /// Emails a fetch replaced, asked for the type they lack.
    replaced: Vec<Id>,
    pub(super) documents: Vec<Id>,
    /// The documents whose title the user gave: it stays.
    titled: Vec<Id>,
}

impl Batch {
    // ------------------------------------------------------------------------
    /// The new emails `emails` of `email`'s account, then the attachments read
    /// `documents`.
    pub fn email(email: &str, emails: Vec<Id>, documents: Vec<Id>) -> Self {
        Self {
            label: Text::new("analysis.label.email").arg("email", email),
            notices: Text::new("analysis.notices.email").arg("email", email),
            what: email_analysis(),
            emails,
            replaced: Vec::new(),
            documents,
            titled: Vec::new(),
        }
    }

    // ------------------------------------------------------------------------
    /// The emails `replaced` too, after the new ones: each that still lacks
    /// a type, for that alone (`EmailText::fill`).
    pub fn replaced(mut self, replaced: Vec<Id>) -> Self {
        self.replaced = replaced;
        self
    }

    // ------------------------------------------------------------------------
    /// The attachments read `read` too, after the emails.
    pub fn with_read(mut self, read: Vec<Id>) -> Self {
        self.documents.extend(read);
        self
    }

    // ------------------------------------------------------------------------
    /// The documents `documents` imported from the watch `watch` of the type
    /// `kind`, those in `titled` with the title the user gave.
    pub fn scans(kind: WatchKind, watch: &str, documents: Vec<Id>, titled: Vec<Id>) -> Self {
        let (label, notices) = match kind {
            // A statements watch imports no documents to analyse.
            WatchKind::Images | WatchKind::Statements => (
                Text::new("analysis.label.scans"),
                Text::new("analysis.notices.scans"),
            ),
            WatchKind::Pdf => (
                Text::new("analysis.label.pdfs"),
                Text::new("analysis.notices.pdfs"),
            ),
        };
        Self {
            label: label.arg("watch", watch),
            notices: notices.arg("watch", watch),
            what: scan_analysis(kind),
            emails: Vec::new(),
            replaced: Vec::new(),
            documents,
            titled,
        }
    }
}

// ----------------------------------------------------------------------------
/// Analyse `batch` as `analysis` says (its level and model) as a job: its
/// emails, each with its attachments' text, then the replaced ones that
/// lack a type, for that alone, then its documents (an
/// attachment as Lite). Nothing to analyse starts nothing; a refused pick
/// is a notice; what is no longer in the vault, and a document without
/// text, is left out, and so is what its classification keeps from the
/// model (`ai::policy`), which a notice counts.
pub fn analyse(app: &mut App, batch: Batch, analysis: &Analysis) {
    let index = app.vault.index();
    let refill: Vec<Id> = (batch.replaced.iter().copied())
        .filter(|id| !batch.emails.contains(id))
        .filter(|id| index.entry(id).is_some_and(|entry| lacks_type(&entry.doc)))
        .collect();
    if batch.emails.is_empty() && refill.is_empty() && batch.documents.is_empty() {
        return;
    }
    let choice = analysis.model.as_ref();
    let resolved = match app.ai.choose(choice, batch.what.clone(), DateTime::now()) {
        Ok(resolved) => resolved,
        Err(refusal) => {
            let notice = Text::new("job.notice").arg("job", batch.notices.clone());
            return app.notice(notice.arg("text", refusal.text));
        }
    };
    let index = app.vault.index();
    let local = ai::policy::is_local(resolved.provider, &resolved.address, &resolved.model);
    let default = app.settings.classification();
    let reads = |doc: &Document| ai::policy::reads_whole(doc, &default, local);
    let kept = batch
        .emails
        .iter()
        .chain(&refill)
        .chain(&batch.documents)
        .filter_map(|id| index.entry(id))
        .filter(|entry| !reads(&entry.doc))
        .count();
    if kept > 0 {
        let notice = Text::new("job.notice").arg("job", batch.notices.clone());
        app.notice(notice.arg("text", ai::policy::kept_from(kept)));
    }
    let index = app.vault.index();
    let email_item = |id: &Id, fill: bool| {
        let entry = index.entry(id).filter(|entry| reads(&entry.doc))?;
        let mut email = EmailText::of(&entry.doc);
        email.attachments = attachment_texts(index, email.id, reads);
        email.fill = fill;
        Some(Item::Email(email))
    };
    let emails = (batch.emails.iter().map(|id| email_item(id, false)))
        .chain(refill.iter().map(|id| email_item(id, true)))
        .flatten();
    let documents = batch
        .documents
        .iter()
        .filter_map(|id| index.entry(id))
        .filter(|entry| reads(&entry.doc))
        .filter(|entry| !entry.doc.content().trim().is_empty())
        .map(|entry| {
            let mut doc = DocText::of(index, &entry.doc);
            doc.keep_title = batch.titled.contains(&doc.id);
            Item::Document(doc)
        });
    let queue: VecDeque<Ask> = emails
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
    let documents = queue
        .iter()
        .filter(|ask| matches!(ask.item, Item::Document(_)))
        .count();
    let failed = match (total - documents, documents) {
        (_, 0) => Text::new("ai.failed.emails"),
        (0, _) => Text::new("ai.failed.documents"),
        _ => Text::new("ai.failed.emails_documents"),
    };
    let job = app.start_job(JobKind::EmailAnalysis, batch.label.clone(), total);
    log::info!(
        "Analysis {job} ({}): {} emails and {documents} documents with {}, {:?}",
        batch.label,
        total - documents,
        resolved.model,
        analysis.level
    );
    let run = Run {
        notices: batch.notices,
        queue: AiQueue::new(Some(resolved.account), queue),
        ai: resolved,
        level: analysis.level,
        context: context(app.vault.index()),
        counts: Counts::default(),
        failed,
    };
    app.analysis.runs.insert(job, run);
    pump(app, job);
}

// ----------------------------------------------------------------------------
/// Send the next email of the analysis `job`, or finish it if none is left.
fn pump(app: &mut App, job: Id) {
    let Some(run) = app.analysis.runs.get_mut(&job) else {
        return;
    };
    if run.queue.is_busy() {
        return;
    }
    if run.queue.is_empty() {
        return finish(app, job);
    }
    if let Err(text) = run.queue.check_limit(&app.ai) {
        return stop_at_limit(app, job, text);
    }
    let Some(ask) = run.queue.start() else {
        return;
    };
    let mut request = ask.item.request(run.level, &run.ai.model, &run.context);
    if let Some(answer) = &ask.unread {
        request = again(request, answer, ask.item.shape(run.level));
    }
    let client = run.ai.client.clone();
    app.run_job(
        job,
        // Streamed like every other call (the deltas aren't needed): one
        // code path per provider.
        move |cancel| client.chat_stream(cancel, &request, |_| {}),
        move |app, result| on_answer(app, job, result),
    );
}

// ----------------------------------------------------------------------------
/// The model answered for the email or attachment being analysed: write what
/// it found, or count the failure; then the next. An answer that isn't the JSON
/// object is asked again, up to `LITE_ATTEMPTS` times in all, unless it was
/// cut off. Every answer is logged whole with the model's reasoning (debug;
/// info when it can't be read). Ignored if the run is gone (stopped).
fn on_answer(app: &mut App, job: Id, result: Result<ChatResponse>) {
    let Some(run) = app.analysis.runs.get_mut(&job) else {
        return;
    };
    let Some(ask) = run.queue.end() else {
        return;
    };
    let (id, attempt) = (ask.item.id(), ask.attempt);
    let account = run.ai.account;
    let found = match result {
        Ok(response) => {
            let found = ask.item.parse(&response.text, run.level, &run.context);
            let (text, thinking) = (&response.text, &response.thinking);
            log::debug!(
                "Analysis {job}: {id}, attempt {attempt}, found: {found:?}, text: {text:?}, reasoning: {thinking:?}"
            );
            let cut = response.stop == StopReason::MaxTokens;
            if found.is_err() && !cut && attempt < LITE_ATTEMPTS {
                run.queue.again(Ask {
                    item: ask.item,
                    attempt: attempt + 1,
                    unread: Some(response.text),
                });
                ai::account_used(app, account);
                return pump(app, job);
            }
            found.map_err(|e| match attempt {
                _ if cut => Text::new("analysis.cut_off"),
                1 => e,
                n => Text::new("analysis.asked").arg("error", e).arg("n", n),
            })
        }
        Err(e) => Err(ai::chat_error(run.ai.provider, &run.ai.address, &e)),
    };
    let item = ask.item;
    let by = (run.ai.model.clone(), account);
    let written = found.and_then(|(lite, medium)| write(app, &item, lite, medium, by));

    let Some(run) = app.analysis.runs.get_mut(&job) else {
        return;
    };
    let counts = &mut run.counts;
    match written {
        Ok(Written {
            email: false,
            chat: false,
        }) => counts.unchanged += 1,
        Ok(Written { chat, .. }) => {
            counts.analysed += 1;
            counts.conversations += usize::from(chat);
        }
        Err(error) => {
            log::info!("Analysis {job}: {id} failed: {error}");
            run.queue.fail(item.title(), error);
        }
    }
    let (done, total) = (run.queue.done(), run.queue.total());
    // Saved per email, so a long analysis keeps what it spent.
    ai::account_used(app, account);
    app.update_job(job, done, total);
    pump(app, job);
}

// ----------------------------------------------------------------------------
/// What writing an answer changed: the email, and whether Medium's conversation was
/// written.
struct Written {
    pub(super) email: bool,
    pub(super) chat: bool,
}

// ----------------------------------------------------------------------------
/// Whether the email `doc` lacks a type, which a replaced one is
/// analysed for.
fn lacks_type(doc: &Document) -> bool {
    let roles = doc.roles();
    let doc_type = (roles.document.as_ref()).and_then(|document| document.doc_type.as_ref());
    roles.email.is_some() && doc_type.is_none()
}

// ----------------------------------------------------------------------------
/// Write `lite` into the email or document `item`, then `medium` as a conversation
/// linked to it, by the model and account `by`, titled "Email analysis ·
/// <the email's title>" or "Document analysis · <its title>". A title or a
/// document's date the user changed since the plan stays, and a title the
/// user gave; a type it has stays (`apply_lite`, `apply_document`);
/// an email filled in (`EmailText::fill`) gets only a type;
/// what is gone meanwhile gets nothing. A failed write is
/// logged.
fn write(
    app: &mut App,
    item: &Item,
    mut lite: Lite,
    medium: Option<Medium>,
    by: (String, Id),
) -> std::result::Result<Written, Text> {
    let gone = Written {
        email: false,
        chat: false,
    };
    let id = item.id();
    let Some(entry) = app.vault.index().entry(&id) else {
        return Ok(gone);
    };
    let given = matches!(item, Item::Document(doc) if doc.keep_title);
    if entry.doc.title() != item.title() || given {
        lite.title = None;
    }
    if let Item::Email(email) = item
        && email.fill
    {
        lite = Lite {
            doc_type: lite.doc_type,
            ..Lite::default()
        };
    }
    if let Item::Document(doc) = item
        && entry.doc.date() != doc.date
    {
        lite.date = None;
    }
    let failed = |e: Error| {
        log::warn!("Analysis: {id} could not be written: {e}");
        Text::new("item.write_failed")
    };
    let outcome = app.edit(|index, changes| {
        index.edit(&id, changes, |doc| match item {
            Item::Email(_) => apply_lite(doc, &lite),
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
            email: changed,
            chat: false,
        });
    };
    let args = [("title", entry.doc.title())];
    let title = match item {
        Item::Email(_) => texts::filled("analysis.chat_title.email", &args),
        Item::Document(_) => texts::filled("analysis.chat_title.document", &args),
    };
    let title = plain_title(&title, CHAT_TITLE_CHARS);
    let (model, account) = by;
    let messages = medium_messages(&medium, &model, account, DateTime::now());
    match chat::create(app, title, messages, Some(id)) {
        Ok(_) => Ok(Written {
            email: changed,
            chat: true,
        }),
        Err(e) => {
            log::warn!("Analysis: the conversation of {id} could not be written: {e}");
            Err(Text::new("analysis.chat_write_failed"))
        }
    }
}

// ----------------------------------------------------------------------------
/// Stop the analysis `job` (`jobs::stop` has cancelled its request): the
/// email being analysed and those waiting are left, and the job ends.
pub fn stop(app: &mut App, job: Id) {
    let Some(run) = app.analysis.runs.get_mut(&job) else {
        return;
    };
    run.queue.stop();
    log::info!("Analysis {job}: stopped");
    finish(app, job);
}

// ----------------------------------------------------------------------------
/// The account has reached its spend limit (`text` says so): say it once,
/// and stop the analysis as Stop does.
fn stop_at_limit(app: &mut App, job: Id, text: Text) {
    let Some(run) = app.analysis.runs.get(&job) else {
        return;
    };
    let notice = Text::new("job.notice").arg("job", run.notices.clone());
    app.notice(notice.arg("text", text));
    stop(app, job);
}

// ----------------------------------------------------------------------------
/// The analysis `job` is done or stopped: end the job with its summary,
/// record what it spent, and tell the failures in one notice.
fn finish(app: &mut App, job: Id) {
    let Some(run) = app.analysis.runs.remove(&job) else {
        return;
    };
    app.end_job_with(job, run.counts.summary(&run.queue), run.queue.limit());
    ai::account_used(app, run.ai.account);
    if let Some(notice) = run.queue.failures(run.notices.clone(), run.failed.clone()) {
        app.notice(notice);
    }
}
