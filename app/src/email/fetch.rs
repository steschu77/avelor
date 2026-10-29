//! A fetch: its plan, the downloads in flight and its end, with its notice.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use av_google::client::Client;
use av_google::gmail::{self, EmailFilter};
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::envelope::DocumentType;
use serde::Deserialize;
use serde_json::json;

use super::Origin;
use super::apply::{Saved, Stored, apply_email};
use super::entities::Downloaded;
use crate::analysis;
use crate::app::App;
use crate::attachments;
use crate::google;
use crate::google::accounts::Analysis;
use crate::google::accounts::AttachmentSettings;
use crate::google::accounts::FetchEmail;
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use crate::protocol::Req;
use crate::texts::Text;
use crate::watches::READ_STEP;
use av_vault::schema::remote::GMAIL_ID;

/// Message downloads in flight per fetch.
const FETCH_PARALLEL: usize = 4;

// ----------------------------------------------------------------------------
/// An email fetch in progress: the ids still to download, and what the
/// finished ones came to. Stepped by `pump` / `on_downloaded`.
pub(super) struct EmailFetch {
    pub(super) job_id: Id,
    pub(super) account: Id,
    pub(super) client: Arc<Client>,
    pub(super) queue: VecDeque<String>,
    /// Ids whose body may replace the one already in the vault (the user
    /// confirmed it); any other email found in the vault is skipped.
    pub(super) replace: HashSet<String>,
    in_flight: usize,
    total: usize,
    done: usize,
    pub(super) counts: FetchCounts,
    pub(super) origin: Origin,
    /// When the fetch (an auto-sync: its search) started: what it fetched
    /// covers the email up to then.
    pub(super) started: DateTime,
    /// The analysis of the emails it creates, as the fetch's filter has it.
    pub(super) analysis: Option<Analysis>,
    /// The attachments it saves, as the fetch's filter has them; none: off.
    pub(super) attachments: Option<AttachmentSettings>,
}

// ----------------------------------------------------------------------------
/// A fetch to start: what `start_fetch` makes a `EmailFetch` and a job of.
pub(super) struct FetchPlan {
    pub(super) account: Id,
    pub(super) client: Arc<Client>,
    pub(super) queue: VecDeque<String>,
    pub(super) skipped: usize,
    pub(super) replace: HashSet<String>,
    pub(super) origin: Origin,
    pub(super) started: DateTime,
    pub(super) analysis: Option<Analysis>,
    pub(super) attachments: Option<AttachmentSettings>,
}

// ----------------------------------------------------------------------------
/// What a fetch came to: the job's summary.
#[derive(Default)]
pub(super) struct FetchCounts {
    /// The emails written, new.
    pub(super) created: Vec<Id>,
    pub(super) replaced: usize,
    pub(super) skipped: usize,
    pub(super) failed: usize,
    pub(super) attachments: Saved,
}

impl FetchCounts {
    // ------------------------------------------------------------------------
    /// "22 new · 3 replaced · 1 failed · 5 attachments": the counts that
    /// aren't zero.
    fn summary(&self) -> Vec<Text> {
        let counts = vec![
            (self.created.len(), Text::new("count.new")),
            (self.replaced, Text::new("count.replaced")),
            (self.skipped, Text::new("count.skipped")),
            (self.failed, Text::new("count.failed")),
            (
                self.attachments.created.len(),
                Text::new("email.count.attachments"),
            ),
            (
                self.attachments.failed,
                Text::new("email.count.attachments_failed"),
            ),
        ];
        count_summary(counts, Text::new("job.nothing_to_fetch"))
    }
}

// ----------------------------------------------------------------------------
/// `email_fetch`'s args.
#[derive(Deserialize)]
pub(crate) struct FetchRequest {
    pub(super) account: Id,
    pub(super) ids: Vec<String>,
    #[serde(default)]
    pub(super) replace: Vec<String>,
    /// Remembered in the account for the next time the page opens.
    #[serde(default)]
    pub(super) filter: EmailFilter,
    #[serde(default)]
    auto_update: bool,
    /// Remembered like the filter; with a level other than off, the new
    /// emails are analysed after the fetch.
    #[serde(default)]
    pub(super) analysis: Option<Analysis>,
    /// Remembered like the filter; enabled, the emails' attachments are saved.
    #[serde(default)]
    pub(super) attachments: Option<AttachmentSettings>,
}

// ----------------------------------------------------------------------------
/// `email_fetch {account, ids, replace?, filter?, auto_update?, analysis?,
/// attachments?}`: fetch the emails into the vault as a job, answering
/// `{job}` once it started. The job's events follow, an `entity` event per
/// email and attachment written (with `content` for a replaced body), a
/// `notice` per email that failed, and finally the account (`account`: its
/// last fetch and sync). The filter, mode, analysis and attachments are
/// remembered in the account first. With the analysis on, or the
/// attachments read, their picks are checked first (`AiAccounts::choose`):
/// refused, nothing is fetched; else the attachments are read and the new
/// emails analysed after the fetch.
pub fn email_fetch(app: &mut App, req: Req, request: FetchRequest) {
    if app.email.fetch.is_some() {
        let what = "a second email fetch".to_string();
        return req.fail(
            Error::Unsupported { what },
            Text::new("email.fetch_running"),
        );
    }
    let account = request.account;
    let client = match app.google.connected(account) {
        Ok(client) => client,
        Err(e) => return google::refuse_account(req, e),
    };
    let analysis = request.analysis.clone().filter(Analysis::is_on);
    if let Some(analysis) = &analysis
        && let Err(refusal) = analysis.check(&app.ai, analysis::email_analysis())
    {
        return req.fail(refusal.error, refusal.text);
    }
    let attachments = request.attachments.clone().filter(|a| a.enabled);
    if let Some(reading) = attachments.as_ref().filter(|a| a.read) {
        let picked = reading.model(READ_STEP);
        if let Err(refusal) = app
            .ai
            .choose(picked, attachments::reading(), DateTime::now())
        {
            return req.fail(refusal.error, refusal.text);
        }
    }
    let remembered = Remembered {
        filter: request.filter,
        auto_update: request.auto_update,
        analysis: request.analysis,
        attachments: request.attachments,
    };
    remember_settings(app, account, remembered);

    let replace: HashSet<String> = request.replace.into_iter().collect();
    let known = app
        .vault
        .index()
        .remote_values(DocumentType::Email, Some(&account), GMAIL_ID);
    let (queue, skipped) = plan_fetch(request.ids, &replace, &known);
    let empty = queue.is_empty();
    let email = find_account_email(app, account);
    let plan = FetchPlan {
        account,
        client,
        queue,
        skipped,
        replace,
        origin: Origin::Manual,
        started: DateTime::now(),
        analysis,
        attachments,
    };
    let label = Text::new("email.fetching").arg("email", email.as_str());
    let job_id = start_fetch(app, label, plan);
    req.ok(json!({ "job": job_id }));
    if empty {
        // After the answer, so the page knows the job when it ends.
        app.ui().send_to_ui(finish);
    } else {
        pump(app);
    }
}

// ----------------------------------------------------------------------------
/// Start the fetch of `plan` as a job shown as `label`; its id. The caller
/// starts the downloads (`pump`), or finishes an empty fetch.
pub(super) fn start_fetch(app: &mut App, label: Text, plan: FetchPlan) -> Id {
    let (skipped, account) = (plan.skipped, plan.account);
    let total = plan.queue.len() + skipped;
    let job_id = app.start_job(JobKind::EmailFetch, label, total);
    log::info!(
        "Email fetch {job_id} ({:?}): {total} emails from {account}, {skipped} in the vault already",
        plan.origin
    );
    app.email.fetch = Some(EmailFetch {
        job_id,
        account,
        client: plan.client,
        queue: plan.queue,
        replace: plan.replace,
        in_flight: 0,
        total,
        done: skipped,
        counts: FetchCounts {
            skipped,
            ..FetchCounts::default()
        },
        origin: plan.origin,
        started: plan.started,
        analysis: plan.analysis,
        attachments: plan.attachments,
    });
    if skipped > 0 {
        app.update_job(job_id, skipped, total);
    }
    job_id
}

// ----------------------------------------------------------------------------
/// The plan of a fetch: the ids to download, in order, and how many need
/// no download -- in the vault already by this account's Gmail id
/// (`known`), and not to be replaced, they are skipped.
pub(super) fn plan_fetch(
    ids: Vec<String>,
    replace: &HashSet<String>,
    known: &HashSet<String>,
) -> (VecDeque<String>, usize) {
    let mut queue = VecDeque::new();
    let mut skipped = 0;
    for id in ids {
        if known.contains(&id) && !replace.contains(&id) {
            skipped += 1;
        } else {
            queue.push_back(id);
        }
    }
    (queue, skipped)
}

// ----------------------------------------------------------------------------
pub(super) fn find_account_email(app: &App, account: Id) -> String {
    app.google
        .get(account)
        .ok()
        .and_then(|a| a.email())
        .unwrap_or("Google account")
        .to_string()
}

// ----------------------------------------------------------------------------
/// What Fetch email remembers in the account.
struct Remembered {
    pub(super) filter: EmailFilter,
    auto_update: bool,
    pub(super) analysis: Option<Analysis>,
    pub(super) attachments: Option<AttachmentSettings>,
}

// ----------------------------------------------------------------------------
/// Store the page's filter, mode, analysis and attachments in the account
/// file (best effort: a failed save only costs the pre-filled form next
/// time).
fn remember_settings(app: &mut App, account: Id, remembered: Remembered) {
    let remembered = app.google.update(account, |account| {
        let email = account
            .file
            .google
            .fetch_email
            .get_or_insert_with(FetchEmail::default);
        email.filter = remembered.filter;
        email.auto_update = remembered.auto_update;
        email.analysis = remembered.analysis;
        email.attachments = remembered.attachments;
        true
    });
    if let Err(e) = remembered {
        log::warn!("Remembering the email filter of {account} failed: {e}");
    }
}

// ----------------------------------------------------------------------------
/// Start downloads until `FETCH_PARALLEL` are in flight.
pub(super) fn pump(app: &mut App) {
    let Some(fetch) = &mut app.email.fetch else {
        return;
    };
    let mut starting = Vec::new();
    while fetch.in_flight < FETCH_PARALLEL {
        let Some(id) = fetch.queue.pop_front() else {
            break;
        };
        fetch.in_flight += 1;
        starting.push(id);
    }
    let (job, client) = (fetch.job_id, Arc::clone(&fetch.client));
    let attachments = fetch.attachments.clone();
    for id in starting {
        let client = Arc::clone(&client);
        let gmail_id = id.clone();
        let attachments = attachments.clone();
        app.run_job(
            job,
            move |cancel| download(&client, cancel, &gmail_id, attachments.as_ref()),
            move |app, result| on_downloaded(app, job, id, result),
        );
    }
}

// ----------------------------------------------------------------------------
/// The run step: download the email `id` and parse it, dated when Gmail got
/// it (now, if Gmail doesn't say), with the attachments `keep` wants.
fn download(
    client: &Client,
    cancel: &Cancel,
    id: &str,
    keep: Option<&AttachmentSettings>,
) -> Result<Downloaded> {
    let raw = gmail::get_raw_message(client, cancel, id)?;
    let date = match raw.epoch_ms {
        0 => DateTime::now(),
        ms => DateTime::from_unix_millis(ms),
    };
    Ok(Downloaded::parse(&raw.bytes, date, keep))
}

// ----------------------------------------------------------------------------
/// One download finished: apply it, count it, report progress, and start
/// the next -- or finish the job. Ignored if the job is gone (stopped).
fn on_downloaded(app: &mut App, job: Id, gmail_id: String, result: Result<Downloaded>) {
    let Some(fetch) = app.email.fetch.as_mut().filter(|f| f.job_id == job) else {
        return;
    };
    fetch.in_flight -= 1;
    fetch.done += 1;
    let (account, replace) = (fetch.account, fetch.replace.contains(&gmail_id));

    let stored = result.and_then(|email| apply_email(app, account, &gmail_id, email, replace));
    if let Err(e) = &stored {
        log::warn!("Could not fetch email {gmail_id}: {e}");
        app.notice(Text::new("email.fetch_failed").arg("id", gmail_id.as_str()));
    }

    let Some(fetch) = app.email.fetch.as_mut() else {
        return;
    };
    let counts = &mut fetch.counts;
    match stored {
        Ok((stored, attachments)) => {
            match stored {
                Stored::Created(id) => counts.created.push(id),
                Stored::Replaced => counts.replaced += 1,
                Stored::Skipped => counts.skipped += 1,
            }
            counts.attachments.created.extend(attachments.created);
            counts.attachments.unread.extend(attachments.unread);
            counts.attachments.failed += attachments.failed;
        }
        Err(_) => counts.failed += 1,
    }
    let (done, total) = (fetch.done, fetch.total);
    let finished = fetch.queue.is_empty() && fetch.in_flight == 0;
    app.update_job(job, done, total);
    if finished {
        finish(app);
    } else {
        pump(app);
    }
}

// ----------------------------------------------------------------------------
/// Stop the fetch (`jobs::stop` has cancelled its downloads): nothing more
/// is written, and the job ends now. Downloads that finish anyway find no
/// fetch.
pub fn stop(app: &mut App) {
    if let Some(fetch) = &mut app.email.fetch {
        fetch.queue.clear();
        finish(app);
    }
}

// ----------------------------------------------------------------------------
/// The fetch is done or stopped: finish the account's sync -- remembering
/// when it started (the page's "Since last time", or where the next
/// auto-sync starts) only if it fetched all and nothing failed, so the next
/// one still covers what it didn't write -- and end the job with its
/// summary. Unless it was stopped, the attachments it saved (and those of
/// a replaced email without text) are read if its filter says so, then the
/// analysis it asks for starts on the emails it created.
pub(super) fn finish(app: &mut App) {
    let Some(fetch) = app.email.fetch.take() else {
        return;
    };
    let stopped = app.job_stopping(fetch.job_id);
    let complete = fetch.counts.failed == 0 && !stopped;
    let (started, origin) = (fetch.started, fetch.origin);
    google::finish_sync(app, fetch.account, |account| {
        let Some(email) = account
            .file
            .google
            .fetch_email
            .as_mut()
            .filter(|_| complete)
        else {
            return;
        };
        match (origin, &mut email.auto_sync) {
            (Origin::Manual, _) => email.last_fetch = Some(started),
            (Origin::AutoSync, Some(sync)) => sync.since = started,
            (Origin::AutoSync, None) => {}
        }
    });

    let counts = &fetch.counts;
    app.end_job_with(fetch.job_id, counts.summary(), false);

    let email = find_account_email(app, fetch.account);
    let title_of = |id: Id| {
        app.vault
            .index()
            .entry(&id)
            .map(|e| e.doc.title().to_string())
    };
    if let Some((text, entity)) = new_email_notice(&counts.created, title_of, &email) {
        app.inform(text, entity);
    }
    let FetchCounts {
        created,
        attachments: saved,
        ..
    } = fetch.counts;
    let analysis = fetch.analysis.filter(|a| a.is_on() && !stopped);
    let then = analysis.map(|analysis| (created, analysis));
    let reading = fetch.attachments.filter(|a| a.read && !stopped);
    let to_read: Vec<Id> = saved.created.into_iter().chain(saved.unread).collect();
    match reading {
        Some(reading) if !to_read.is_empty() => {
            attachments::read(app, &email, to_read, reading.model(READ_STEP), then);
        }
        _ => {
            if let Some((emails, analysis)) = then {
                analysis::analyse(app, &email, emails, Vec::new(), &analysis);
            }
        }
    }
}

// ----------------------------------------------------------------------------
/// What the status bar says about the emails a fetch wrote (`created`):
/// nothing for none; the title of one, which a click shows; the count of
/// more, and whose they are.
pub(super) fn new_email_notice(
    created: &[Id],
    title_of: impl Fn(Id) -> Option<String>,
    email: &str,
) -> Option<(Text, Option<Id>)> {
    match created {
        [] => None,
        [id] => {
            let text = match title_of(*id) {
                Some(title) => Text::new("email.new_one").arg("title", title),
                None => Text::new("email.new_one").arg("title", Text::new("email.no_subject")),
            };
            Some((text, Some(*id)))
        }
        many => {
            let text = Text::new("email.new_many").arg("n", many.len());
            Some((text.arg("email", email), None))
        }
    }
}
