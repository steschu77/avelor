//! The analysis as a job: its queue of entities, the answers and their writes.

use std::collections::VecDeque;

use av_llm::{ChatResponse, StopReason};
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::vault::SaveOutcome;

use super::answer::{Lite, Medium, parse};
use super::apply::{apply_lite, medium_messages};
use super::context::{Context, EmailText, context};
use super::email_analysis;
use super::request::{again, request_for};
use crate::ai;
use crate::ai::accounts::Resolved;
use crate::ai::queue::AiQueue;
use crate::app::App;
use crate::chat;
use crate::google::accounts::{Analysis, AnalysisLevel};
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use crate::texts::{self, Text};
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
    /// Whose email it is, for the notices.
    pub(super) email: String,
    pub(super) ai: Resolved,
    pub(super) level: AnalysisLevel,
    pub(super) context: Context,
    queue: AiQueue<Ask>,
    counts: Counts,
}

// ----------------------------------------------------------------------------
/// An email to ask about: the how-manyth time, and the answer of the time
/// before, which wasn't the JSON object.
struct Ask {
    pub(super) email: EmailText,
    attempt: u32,
    unread: Option<String>,
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
/// Analyse the emails `ids` of `email`'s account as `analysis` says (its
/// level and model) as a job ("Analysing email · <email>"). A refused pick is
/// a notice; emails no longer in the vault are left out.
pub fn analyse(app: &mut App, email: &str, ids: Vec<Id>, analysis: &Analysis) {
    let choice = analysis.model.as_ref();
    let resolved = match app.ai.choose(choice, email_analysis(), DateTime::now()) {
        Ok(resolved) => resolved,
        Err(refusal) => {
            let notice = Text::new("job.notice").arg(
                "job",
                Text::new("analysis.notices.email").arg("email", email),
            );
            return app.notice(notice.arg("text", refusal.text));
        }
    };
    let queue: VecDeque<Ask> = ids
        .iter()
        .filter_map(|id| app.vault.index().entry(id))
        .map(|entry| Ask {
            email: EmailText::of(&entry.doc),
            attempt: 1,
            unread: None,
        })
        .collect();
    if queue.is_empty() {
        return;
    }
    let total = queue.len();
    let label = Text::new("analysis.label.email").arg("email", email);
    let job = app.start_job(JobKind::EmailAnalysis, label, total);
    log::info!(
        "Email analysis {job}: {total} emails of {email} with {}, {:?}",
        resolved.model,
        analysis.level
    );
    let run = Run {
        email: email.to_string(),
        queue: AiQueue::new(Some(resolved.account), queue),
        ai: resolved,
        level: analysis.level,
        context: context(app.vault.index()),
        counts: Counts::default(),
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
    let mut request = request_for(run.level, &run.ai.model, &run.context, &ask.email);
    if let Some(answer) = &ask.unread {
        request = again(request, answer, run.level);
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
/// The model answered for the email being analysed: write what it found, or
/// count the failure; then the next email. An answer that isn't the JSON
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
    let (id, attempt) = (ask.email.id, ask.attempt);
    let account = run.ai.account;
    let found = match result {
        Ok(response) => {
            let found = parse(&response.text, run.level, &run.context, &ask.email.title);
            let (text, stop, thinking) = (&response.text, &response.stop, &response.thinking);
            match &found {
                Ok(_) => log::debug!(
                    "Email analysis {job}: {id}, attempt {attempt}, answered ({stop:?}): {text:?}, reasoning: {thinking:?}"
                ),
                Err(_) => log::info!(
                    "Email analysis {job}: {id}, attempt {attempt}, an answer that isn't the JSON object ({stop:?}): {text:?}, reasoning: {thinking:?}"
                ),
            }
            let cut = response.stop == StopReason::MaxTokens;
            if found.is_err() && !cut && attempt < LITE_ATTEMPTS {
                run.queue.again(Ask {
                    email: ask.email,
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
    let email = ask.email;
    let by = (run.ai.model.clone(), account);
    let written = found.and_then(|(lite, medium)| write(app, &email, lite, medium, by));

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
            log::info!("Email analysis {job}: {} failed: {error}", email.id);
            run.queue.fail(&email.title, error);
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
/// Write `lite` into `email`, then `medium` as a conversation linked to it, by the
/// model and account `by`, titled "Email analysis · <the email's title>". A
/// title the user changed since the plan stays; an email gone meanwhile gets
/// nothing. A failed write is logged.
fn write(
    app: &mut App,
    email: &EmailText,
    mut lite: Lite,
    medium: Option<Medium>,
    by: (String, Id),
) -> std::result::Result<Written, Text> {
    let gone = Written {
        email: false,
        chat: false,
    };
    let Some(entry) = app.vault.index().entry(&email.id) else {
        return Ok(gone);
    };
    if entry.doc.title() != email.title {
        lite.title = None;
    }
    let id = email.id;
    let failed = |e: Error| {
        log::warn!("Email analysis: {id} could not be written: {e}");
        Text::new("item.write_failed")
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
            email: changed,
            chat: false,
        });
    };
    let title = plain_title(
        &texts::filled("analysis.chat_title.email", &[("title", entry.doc.title())]),
        CHAT_TITLE_CHARS,
    );
    let (model, account) = by;
    let messages = medium_messages(&medium, &model, account, DateTime::now());
    match chat::create(app, title, messages, Some(id)) {
        Ok(_) => Ok(Written {
            email: changed,
            chat: true,
        }),
        Err(e) => {
            log::warn!("Email analysis: the chat of {id} could not be written: {e}");
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
    log::info!("Email analysis {job}: stopped");
    finish(app, job);
}

// ----------------------------------------------------------------------------
/// The account has reached its spend limit (`text` says so): say it once,
/// and stop the analysis as Stop does.
fn stop_at_limit(app: &mut App, job: Id, text: Text) {
    let Some(run) = app.analysis.runs.get(&job) else {
        return;
    };
    let notice = Text::new("job.notice").arg(
        "job",
        Text::new("analysis.notices.email").arg("email", run.email.as_str()),
    );
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
    if let Some(notice) = run.queue.failures(
        Text::new("analysis.notices.email").arg("email", run.email.as_str()),
        Text::new("ai.failed.emails"),
    ) {
        app.notice(notice);
    }
}
