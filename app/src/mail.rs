//! The email import from Gmail (mail.md, `mail_*`): the Fetch-mail page's
//! labels, live query, match preview and paging, and the fetch job that
//! downloads, converts, and writes the selected mails into `Mail/`.
//!
//! A fetch goes plan → run → apply (mail.md#design): the plan decides from the
//! vault what to download, workers download and parse the mails, and each one
//! is applied on the UI thread through the vault edit.
//!
//! Every mail written carries `remote: {account, remote: [{message-id},
//! {gmail-id}]}` (vault.md#remote), which is how a later preview tells the
//! mails already in the vault from the new ones.
//!
//! An account's auto-sync fetches the new mail that matches its own filter
//! every 15 minutes, started by the clock (`App::on_clock`): a quiet search
//! first, then -- only if there is new mail -- a fetch job like the page's.
//!
//! A fetch whose filter turns the analysis on (mail.md#analysis) hands its new
//! mails to `enrich::analyse` when it ends.
//!
//! A fetch whose filter saves attachments (mail.md#attachments) keeps the
//! wanted parts on the worker, and writes each as a document with its
//! original in `Attachments/`, `linked-to` its mail. Read on, it hands them
//! to `attachments::read` when it ends, which starts the analysis after.

use std::collections::HashMap;
use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;

use av_google::client::Client;
use av_google::gmail::{self, MailFilter, MailHeader};
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::email;
use av_vault::envelope::{DocumentType, Email, Remote, Roles, Source};
use av_vault::shared::OneOrMany;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::app::App;
use crate::attachments;
use crate::enrich;
use crate::google;
use crate::google::{SYNC_EVERY, first_due, later};
use crate::google_accounts::Analysis;
use crate::google_accounts::AttachmentSettings;
use crate::google_accounts::AutoSync;
use crate::google_accounts::MailSettings;
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use crate::protocol::Req;
use crate::sys;
use crate::watches::{READ_STEP, glob_match};
use av_vault::schema::layout;
use av_vault::schema::remote::{GMAIL_ID, MESSAGE_ID};
use av_vault::vault::Index;

// ----------------------------------------------------------------------------
/// The superseding channel of `mail_search` and `mail_headers`: a newer
/// preview request aborts the older one.
const PREVIEW: &str = "mail-preview";
/// The most matches a search lists.
const MATCH_CAP: usize = 2000;
/// Matches per preview page; `mail_search` answers with the first page's
/// headers.
const PAGE_SIZE: usize = 50;
/// Header requests in flight per page.
const HEADER_PARALLEL: usize = 8;
/// Message downloads in flight per fetch.
const FETCH_PARALLEL: usize = 4;

// ----------------------------------------------------------------------------
/// What the app keeps for the mail import: the fetch in progress (one at
/// a time), and the auto-sync's schedule.
#[derive(Default)]
pub struct MailState {
    fetch: Option<MailFetch>,
    /// Per account, when its auto-sync may run next; none: at the next
    /// chance. Cleared when a vault opens.
    next_sync: HashMap<Id, DateTime>,
    /// The account whose auto-sync search is in flight.
    syncing: Option<Id>,
    /// The accounts whose last auto-sync failed: only the first failure is
    /// shown.
    failing: HashSet<Id>,
}

// ----------------------------------------------------------------------------
/// Why a fetch runs: the page's Fetch, or an account's auto-sync.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Origin {
    Manual,
    AutoSync,
}

// ----------------------------------------------------------------------------
/// A mail fetch in progress: the ids still to download, and what the
/// finished ones came to. Stepped by `pump` / `on_downloaded`.
struct MailFetch {
    job_id: Id,
    account: Id,
    client: Arc<Client>,
    queue: VecDeque<String>,
    /// Ids whose body may replace the one already in the vault (the user
    /// confirmed it); any other mail found in the vault is skipped.
    replace: HashSet<String>,
    in_flight: usize,
    total: usize,
    done: usize,
    counts: FetchCounts,
    origin: Origin,
    /// When the fetch (an auto-sync: its search) started: what it fetched
    /// covers the mail up to then.
    started: DateTime,
    /// The analysis of the mails it creates, as the fetch's filter has it.
    analysis: Option<Analysis>,
    /// The attachments it saves, as the fetch's filter has them; none: off.
    attachments: Option<AttachmentSettings>,
}

// ----------------------------------------------------------------------------
/// A fetch to start: what `start_fetch` makes a `MailFetch` and a job of.
struct FetchPlan {
    account: Id,
    client: Arc<Client>,
    queue: VecDeque<String>,
    skipped: usize,
    replace: HashSet<String>,
    origin: Origin,
    started: DateTime,
    analysis: Option<Analysis>,
    attachments: Option<AttachmentSettings>,
}

// ----------------------------------------------------------------------------
/// What a fetch came to: the job's summary.
#[derive(Default)]
struct FetchCounts {
    /// The mails written, new.
    created: Vec<Id>,
    replaced: usize,
    skipped: usize,
    failed: usize,
    attachments: Saved,
}

impl FetchCounts {
    // ------------------------------------------------------------------------
    /// "22 new · 3 replaced · 1 failed · 5 attachments": the counts that
    /// aren't zero.
    fn summary(&self) -> String {
        let counts = [
            (self.created.len(), "new", "new"),
            (self.replaced, "replaced", "replaced"),
            (self.skipped, "skipped", "skipped"),
            (self.failed, "failed", "failed"),
            (self.attachments.created.len(), "attachment", "attachments"),
            (
                self.attachments.failed,
                "attachment failed",
                "attachments failed",
            ),
        ];
        count_summary(&counts, "nothing to fetch")
    }
}

// ----------------------------------------------------------------------------
/// `mail_search`'s args.
#[derive(Deserialize)]
pub(crate) struct SearchRequest {
    account: Id,
    filter: MailFilter,
}

// ----------------------------------------------------------------------------
/// `mail_headers`' args: the ids of a page of matches.
#[derive(Deserialize)]
pub(crate) struct HeadersRequest {
    account: Id,
    ids: Vec<String>,
}

// ----------------------------------------------------------------------------
/// `mail_fetch`'s args.
#[derive(Deserialize)]
pub(crate) struct FetchRequest {
    account: Id,
    ids: Vec<String>,
    #[serde(default)]
    replace: Vec<String>,
    /// Remembered in the account for the next time the page opens.
    #[serde(default)]
    filter: MailFilter,
    #[serde(default)]
    auto_update: bool,
    /// Remembered like the filter; with a level other than off, the new
    /// mails are analysed after the fetch.
    #[serde(default)]
    analysis: Option<Analysis>,
    /// Remembered like the filter; enabled, the mails' attachments are saved.
    #[serde(default)]
    attachments: Option<AttachmentSettings>,
}

// ----------------------------------------------------------------------------
/// One row of the preview (`mail_search`, `mail_headers`).
#[derive(Serialize)]
struct HeaderInfo {
    id: String,
    date: DateTime,
    from: String,
    subject: String,
    in_vault: bool,
}

// ----------------------------------------------------------------------------
#[derive(Serialize)]
struct MatchInfo<'a> {
    id: &'a str,
    in_vault: bool,
}

// ----------------------------------------------------------------------------
/// `mail_search`'s answer.
#[derive(Serialize)]
struct Matches<'a> {
    matches: Vec<MatchInfo<'a>>,
    /// There are more than `matches` (`MATCH_CAP`).
    truncated: bool,
    /// The first page's headers.
    headers: Vec<HeaderInfo>,
}

// ----------------------------------------------------------------------------
#[derive(Serialize)]
struct LabelInfo<'a> {
    id: &'a str,
    name: &'a str,
}

// ----------------------------------------------------------------------------
/// The mails the vault already holds: by Gmail id from this account, by
/// Message-ID from anywhere (another account, an imported `.eml`).
struct KnownMails {
    gmail: HashSet<String>,
    message: HashSet<String>,
}

impl KnownMails {
    // ------------------------------------------------------------------------
    fn of(app: &App, account: Id) -> Self {
        Self {
            gmail: app
                .vault
                .index()
                .remote_values(DocumentType::Email, Some(&account), GMAIL_ID),
            message: app
                .vault
                .index()
                .remote_values(DocumentType::Email, None, MESSAGE_ID),
        }
    }

    // ------------------------------------------------------------------------
    fn header(&self, header: MailHeader) -> HeaderInfo {
        let in_vault = self.gmail.contains(&header.id)
            || (!header.message_id.is_empty() && self.message.contains(&header.message_id));
        HeaderInfo {
            date: DateTime::from_unix_millis(header.epoch_ms),
            from: email::decode_header(&header.from),
            subject: email::decode_header(&header.subject),
            id: header.id,
            in_vault,
        }
    }
}

// ----------------------------------------------------------------------------
/// Answer a request that failed with `error`: `what` failed, in words for
/// the page.
fn fail(req: Req, error: Error, what: &str) {
    log::info!("Mail request failed: {what}: {error}");
    req.fail(error, format!("{what}."));
}

// ----------------------------------------------------------------------------
/// `mail_labels {account}`: the account's own labels, `{labels: [{id,
/// name}]}`.
pub fn mail_labels(app: &mut App, req: Req, account: Id) {
    let client = match app.google.connected(account) {
        Ok(client) => client,
        Err(e) => return google::refuse_account(req, e),
    };
    app.run(
        move |cancel| gmail::list_labels(&client, cancel),
        move |_, result| match result {
            Ok(labels) => {
                let labels: Vec<LabelInfo> = labels
                    .iter()
                    .map(|l| LabelInfo {
                        id: &l.id,
                        name: &l.name,
                    })
                    .collect();
                req.ok(json!({ "labels": labels }));
            }
            Err(e) => fail(req, e, "Could not list the labels"),
        },
    );
}

// ----------------------------------------------------------------------------
/// `mail_search {account, filter}`: search the account. Answers `{matches:
/// [{id, in_vault}], truncated, headers}`, `headers` being the first
/// page's. Supersedes any preview request still running, which then
/// answers `cancelled`.
pub fn mail_search(app: &mut App, req: Req, request: SearchRequest) {
    let account = request.account;
    let client = match app.google.connected(account) {
        Ok(client) => client,
        Err(e) => return google::refuse_account(req, e),
    };
    let query = request.filter.to_query();
    let op = Id::generate().to_string();
    log::debug!("Mail search {op}: searching {account} for `{query}`");
    let work = move |cancel: &Cancel| {
        let (ids, truncated) = gmail::list_message_ids(&client, cancel, &query, MATCH_CAP)?;
        let first = &ids[..ids.len().min(PAGE_SIZE)];
        let headers = fetch_headers(&client, cancel, first)?;
        Ok((ids, truncated, headers))
    };
    let latest = op.clone();
    app.run_superseding(PREVIEW, &latest, work, move |app, result: Result<_>| {
        let (ids, truncated, headers) = match result {
            Ok(found) => found,
            Err(e) => return fail(req, e, "Searching failed"),
        };
        let known = KnownMails::of(app, account);
        let matches = Matches {
            matches: ids
                .iter()
                .map(|id| MatchInfo {
                    id,
                    in_vault: known.gmail.contains(id),
                })
                .collect(),
            truncated,
            headers: headers.into_iter().map(|h| known.header(h)).collect(),
        };
        log::info!("Mail search {op}: {} matches", ids.len());
        req.ok(matches);
    });
}

// ----------------------------------------------------------------------------
/// `mail_headers {account, ids}`: the preview headers of a page of mails,
/// `{headers}`. Supersedes any preview request still running.
pub fn mail_headers(app: &mut App, req: Req, request: HeadersRequest) {
    let account = request.account;
    let client = match app.google.connected(account) {
        Ok(client) => client,
        Err(e) => return google::refuse_account(req, e),
    };
    let ids: Vec<String> = request.ids.into_iter().take(PAGE_SIZE).collect();
    let work = move |cancel: &Cancel| fetch_headers(&client, cancel, &ids);
    let op = Id::generate().to_string();
    app.run_superseding(PREVIEW, &op, work, move |app, result| {
        let headers = match result {
            Ok(headers) => headers,
            Err(e) => return fail(req, e, "Loading the page failed"),
        };
        let known = KnownMails::of(app, account);
        let headers: Vec<HeaderInfo> = headers.into_iter().map(|h| known.header(h)).collect();
        req.ok(json!({ "headers": headers }));
    });
}

// ----------------------------------------------------------------------------
/// The headers of `ids`, in order, `HEADER_PARALLEL` requests at a time. A
/// mail deleted since the search (404) is left out. The first failure ends
/// the walk: no more requests start, the ones in flight finish.
fn fetch_headers(client: &Client, cancel: &Cancel, ids: &[String]) -> Result<Vec<MailHeader>> {
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    // Each worker takes the next id until none is left or one failed.
    let worker = || -> Result<Vec<(usize, MailHeader)>> {
        let mut got = Vec::new();
        while !failed.load(Ordering::Relaxed) {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(id) = ids.get(i) else {
                break;
            };
            match gmail::get_header(client, cancel, id) {
                Ok(header) => got.push((i, header)),
                Err(Error::Http { status: 404, .. }) => {}
                Err(e) => {
                    failed.store(true, Ordering::Relaxed);
                    return Err(e);
                }
            }
        }
        Ok(got)
    };
    let results: Vec<_> = thread::scope(|scope| {
        let workers: Vec<_> = (0..HEADER_PARALLEL.min(ids.len()))
            .map(|_| scope.spawn(worker))
            .collect();
        workers.into_iter().map(|w| w.join()).collect()
    });
    let mut headers = Vec::new();
    for result in results {
        let got = result.map_err(|_| Error::Network {
            msg: "a header request panicked".to_string(),
        })??;
        headers.extend(got);
    }
    headers.sort_by_key(|(i, _)| *i);
    Ok(headers.into_iter().map(|(_, header)| header).collect())
}

// ----------------------------------------------------------------------------
/// `mail_fetch {account, ids, replace?, filter?, auto_update?, analysis?,
/// attachments?}`: fetch the mails into the vault as a job, answering
/// `{job}` once it started. The job's events follow, an `entity` event per
/// mail and attachment written (with `content` for a replaced body), a
/// `notice` per mail that failed, and finally the account (`account`: its
/// last fetch and sync). The filter, mode, analysis and attachments are
/// remembered in the account first. With the analysis on, or the
/// attachments read, their picks are checked first (`AiAccounts::choose`):
/// refused, nothing is fetched; else the attachments are read and the new
/// mails analysed after the fetch.
pub fn mail_fetch(app: &mut App, req: Req, request: FetchRequest) {
    if app.mail.fetch.is_some() {
        let what = "a second mail fetch".to_string();
        return req.fail(
            Error::Unsupported { what },
            "Another fetch is still running.",
        );
    }
    let account = request.account;
    let client = match app.google.connected(account) {
        Ok(client) => client,
        Err(e) => return google::refuse_account(req, e),
    };
    let analysis = request.analysis.clone().filter(Analysis::is_on);
    if let Some(analysis) = &analysis {
        let picked = analysis.model.as_ref();
        if let Err(refusal) = app
            .ai
            .choose(picked, enrich::MAIL_ANALYSIS, DateTime::now())
        {
            return req.fail(refusal.error, refusal.message);
        }
    }
    let attachments = request.attachments.clone().filter(|a| a.enabled);
    if let Some(reading) = attachments.as_ref().filter(|a| a.read) {
        let picked = reading.model(READ_STEP);
        if let Err(refusal) = app.ai.choose(picked, attachments::READING, DateTime::now()) {
            return req.fail(refusal.error, refusal.message);
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
    let job_id = start_fetch(app, format!("Fetching mail · {email}"), plan);
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
fn start_fetch(app: &mut App, label: String, plan: FetchPlan) -> Id {
    let (skipped, account) = (plan.skipped, plan.account);
    let total = plan.queue.len() + skipped;
    let job_id = app.start_job(JobKind::MailFetch, label, total);
    log::info!(
        "Mail fetch {job_id} ({:?}): {total} mails from {account}, {skipped} in the vault already",
        plan.origin
    );
    app.mail.fetch = Some(MailFetch {
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
fn plan_fetch(
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
fn find_account_email(app: &App, account: Id) -> String {
    app.google
        .get(account)
        .ok()
        .and_then(|a| a.email())
        .unwrap_or("Google account")
        .to_string()
}

// ----------------------------------------------------------------------------
/// What Fetch mail remembers in the account.
struct Remembered {
    filter: MailFilter,
    auto_update: bool,
    analysis: Option<Analysis>,
    attachments: Option<AttachmentSettings>,
}

// ----------------------------------------------------------------------------
/// Store the page's filter, mode, analysis and attachments in the account
/// file (best effort: a failed save only costs the pre-filled form next
/// time).
fn remember_settings(app: &mut App, account: Id, remembered: Remembered) {
    let remembered = app.google.update(account, |account| {
        let mail = account
            .file
            .google
            .mail
            .get_or_insert_with(MailSettings::default);
        mail.filter = remembered.filter;
        mail.auto_update = remembered.auto_update;
        mail.analysis = remembered.analysis;
        mail.attachments = remembered.attachments;
        true
    });
    if let Err(e) = remembered {
        log::warn!("Remembering the mail filter of {account} failed: {e}");
    }
}

// ----------------------------------------------------------------------------
/// Start downloads until `FETCH_PARALLEL` are in flight.
fn pump(app: &mut App) {
    let Some(fetch) = &mut app.mail.fetch else {
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
/// A downloaded mail, parsed and converted, ready to be written.
struct Downloaded {
    message_id: String,
    subject: String,
    from: String,
    to: Vec<String>,
    cc: Vec<String>,
    date: DateTime,
    body: String,
    /// The attached files to save (`attachment_wanted`).
    attachments: Vec<email::Attachment>,
}

impl Downloaded {
    // ------------------------------------------------------------------------
    /// The mail `raw` (RFC 822) as it is written: its headers, its body as
    /// markdown, dated `date`; with the attachments `keep` wants.
    fn parse(raw: &[u8], date: DateTime, keep: Option<&AttachmentSettings>) -> Self {
        let mail = email::parse(raw);
        let body = mail.to_markdown();
        let attachments = match keep {
            Some(keep) => mail
                .attachments
                .into_iter()
                .filter(|part| attachment_wanted(keep, part))
                .collect(),
            None => Vec::new(),
        };
        Self {
            body,
            to: email::split_addresses(&mail.to),
            cc: email::split_addresses(&mail.cc),
            message_id: mail.message_id,
            subject: mail.subject,
            from: mail.from,
            date,
            attachments,
        }
    }
}

// ----------------------------------------------------------------------------
/// Whether `part` is a file `settings` saves: attached by the sender (not
/// an image the HTML shows), matching one of its patterns, within its size.
fn attachment_wanted(settings: &AttachmentSettings, part: &email::Attachment) -> bool {
    let max = u64::from(settings.max_size) * 1024 * 1024;
    !part.inline
        && part.bytes.len() as u64 <= max
        && settings
            .patterns
            .iter()
            .any(|p| glob_match(p, &part.filename))
}

// ----------------------------------------------------------------------------
/// The run step: download the mail `id` and parse it, dated when Gmail got
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
/// How one mail ended up in the vault.
#[derive(Debug, PartialEq)]
enum Stored {
    /// A new entity, by its id.
    Created(Id),
    Replaced,
    /// Already in the vault, and not to be replaced.
    Skipped,
}

// ----------------------------------------------------------------------------
/// One download finished: apply it, count it, report progress, and start
/// the next -- or finish the job. Ignored if the job is gone (stopped).
fn on_downloaded(app: &mut App, job: Id, gmail_id: String, result: Result<Downloaded>) {
    let Some(fetch) = app.mail.fetch.as_mut().filter(|f| f.job_id == job) else {
        return;
    };
    fetch.in_flight -= 1;
    fetch.done += 1;
    let (account, replace) = (fetch.account, fetch.replace.contains(&gmail_id));

    let stored = result.and_then(|mail| apply_mail(app, account, &gmail_id, mail, replace));
    if let Err(e) = &stored {
        log::warn!("Could not fetch mail {gmail_id}: {e}");
        app.notice(&format!("Could not fetch mail {gmail_id}."));
    }

    let Some(fetch) = app.mail.fetch.as_mut() else {
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
/// What to do with a downloaded mail.
#[derive(Debug, PartialEq)]
enum Action {
    Create,
    /// Replace the body of the entity that has the mail.
    Replace(Id),
    Skip,
}

// ----------------------------------------------------------------------------
/// What to do with a mail the vault has as `existing` (or doesn't);
/// `replace`: the user confirmed its body may be replaced.
fn fetch_action(existing: Option<Id>, replace: bool) -> Action {
    match existing {
        None => Action::Create,
        Some(id) if replace => Action::Replace(id),
        Some(_) => Action::Skip,
    }
}

// ----------------------------------------------------------------------------
/// The entity the vault has a mail as: by its Gmail id from `account`, else
/// by its Message-ID from anywhere.
fn existing_mail(index: &Index, account: Id, gmail_id: &str, message_id: &str) -> Option<Id> {
    index
        .find_remote(DocumentType::Email, Some(&account), GMAIL_ID, gmail_id)
        .or_else(|| {
            (!message_id.is_empty())
                .then(|| index.find_remote(DocumentType::Email, None, MESSAGE_ID, message_id))
                .flatten()
        })
}

// ----------------------------------------------------------------------------
/// What saving a mail's attachments came to: the documents written, those
/// it had already whose text is still empty (to read), and the files that
/// failed.
#[derive(Debug, Default, PartialEq)]
struct Saved {
    created: Vec<Id>,
    unread: Vec<Id>,
    failed: usize,
}

// ----------------------------------------------------------------------------
/// The apply step: write `mail` into the vault as `fetch_action` says -- a
/// new entity in `Mail/`, the body of the one already there replaced, or
/// nothing -- then the attachments it doesn't have yet
/// (`save_attachments`).
fn apply_mail(
    app: &mut App,
    account: Id,
    gmail_id: &str,
    mut mail: Downloaded,
    replace: bool,
) -> Result<(Stored, Saved)> {
    let existing = existing_mail(app.vault.index(), account, gmail_id, &mail.message_id);
    let parts = std::mem::take(&mut mail.attachments);
    let (message_id, date) = (mail.message_id.clone(), mail.date);
    // A replaced body goes to the page with the entity (`App::edit`), so
    // a preview showing the old one loads it.
    let (stored, id) = match fetch_action(existing, replace) {
        Action::Skip => return Ok((Stored::Skipped, Saved::default())),
        Action::Replace(id) => {
            let outcome = app.edit(|index, changes| index.set_content(&id, mail.body, changes))?;
            app.vault.require_written(outcome)?;
            (Stored::Replaced, id)
        }
        Action::Create => {
            let doc = email_entity(account, gmail_id, mail);
            let folder = Path::new(layout::MAIL);
            let id = app.edit(|index, changes| index.create(doc, folder, changes))?;
            (Stored::Created(id), id)
        }
    };
    let to = AttachedTo {
        mail: id,
        account,
        gmail_id,
        message_id: &message_id,
        date,
    };
    Ok((stored, save_attachments(app, &to, parts)))
}

// ----------------------------------------------------------------------------
/// The mail an attachment came with, as its document names it.
struct AttachedTo<'a> {
    /// The mail's entity.
    mail: Id,
    account: Id,
    gmail_id: &'a str,
    message_id: &'a str,
    date: DateTime,
}

// ----------------------------------------------------------------------------
/// Write each of `parts` that `to.mail` has no document for yet -- one
/// linked to it whose original has the part's name and size -- as a
/// document with its original in `Attachments/`, each in a vault edit of
/// its own, the file marked as downloaded (`sys::mark_downloaded`). A part
/// that fails is logged and counted; the others are still written. The
/// documents `to.mail` has already without text are named too.
fn save_attachments(app: &mut App, to: &AttachedTo, parts: Vec<email::Attachment>) -> Saved {
    let mut saved = Saved {
        unread: unread_originals(app.vault.index(), to.mail),
        ..Saved::default()
    };
    if parts.is_empty() {
        return saved;
    }
    let has = saved_originals(app.vault.index(), to.mail);
    let folder = Path::new(layout::ATTACHMENTS);
    for part in parts {
        if has.contains(&(part.filename.clone(), part.bytes.len() as u64)) {
            continue;
        }
        let doc = attachment_entity(to, &part);
        let written = app.edit(|index, changes| {
            index.create_with_original(doc, folder, &part.filename, &part.bytes, changes)
        });
        match written {
            Ok(id) => {
                saved.created.push(id);
                if let Some(path) = app.vault.index().original_path(&id) {
                    sys::mark_downloaded(&path);
                }
            }
            Err(e) => {
                let name = &part.filename;
                log::warn!("Could not save {name} of mail {}: {e}", to.gmail_id);
                saved.failed += 1;
            }
        }
    }
    saved
}

// ----------------------------------------------------------------------------
/// The name and size of each original of the documents linked to `mail`.
fn saved_originals(index: &Index, mail: Id) -> HashSet<(String, u64)> {
    index
        .all()
        .iter()
        .filter(|entry| entry.doc.linked_to() == Some(mail))
        .filter_map(|entry| {
            let original = entry.doc.original()?;
            Some((original.name.clone()?, original.size?))
        })
        .collect()
}

// ----------------------------------------------------------------------------
/// The documents linked to `mail` that have an original and no text yet.
fn unread_originals(index: &Index, mail: Id) -> Vec<Id> {
    index
        .all()
        .iter()
        .filter(|entry| entry.doc.linked_to() == Some(mail) && entry.doc.original().is_some())
        .filter(|entry| entry.doc.content().trim().is_empty())
        .map(|entry| entry.doc.id())
        .collect()
}

// ----------------------------------------------------------------------------
/// A new document for the attached file `part` (mail.md#attachments):
/// titled by its name without the extension, dated as its mail, linked to
/// it, its source the mail's. Its original is named when it is written
/// (`Index::create_with_original`).
fn attachment_entity(to: &AttachedTo, part: &email::Attachment) -> Document {
    let roles = Roles {
        document: Some(Default::default()),
        ..Default::default()
    };
    let source = Source::Remote(mail_remote(to.account, to.gmail_id, to.message_id));
    let name = part.filename.trim();
    let title = match name.rsplit_once('.') {
        Some((stem, _)) if !stem.trim().is_empty() => stem.trim(),
        _ => name,
    };
    let mut doc = Document::create(
        roles,
        source,
        title.to_string(),
        Vec::new(),
        FileFormat::Markdown,
    );
    doc.set_date(to.date);
    doc.set_link(Some(to.mail));
    doc
}

// ----------------------------------------------------------------------------
/// The `remote:` block of a mail's entities (vault.md#remote): its
/// account, Message-ID (if it has one) and Gmail id.
fn mail_remote(account: Id, gmail_id: &str, message_id: &str) -> Remote {
    let mut remote = Remote {
        account: Some(account),
        remote: Vec::new(),
    };
    if !message_id.is_empty() {
        remote.set(MESSAGE_ID, message_id);
    }
    remote.set(GMAIL_ID, gmail_id);
    remote
}

// ----------------------------------------------------------------------------
/// A new email entity for `mail`, from `account`.
fn email_entity(account: Id, gmail_id: &str, mail: Downloaded) -> Document {
    let addresses = |list: Vec<String>| match list.is_empty() {
        true => OneOrMany::None,
        false => OneOrMany::Many(list),
    };
    let roles = Roles {
        email: Some(Email {
            subject: None,
            kind: None,
            from: Some(mail.from).filter(|f| !f.is_empty()),
            to: addresses(mail.to),
            cc: addresses(mail.cc),
            bcc: OneOrMany::None,
        }),
        ..Default::default()
    };

    let source = Source::Remote(mail_remote(account, gmail_id, &mail.message_id));

    let subject = mail.subject.trim();
    let title = if subject.is_empty() {
        "No subject"
    } else {
        subject
    };
    let mut doc = Document::create(
        roles,
        source,
        title.to_string(),
        Vec::new(),
        FileFormat::Markdown,
    );
    doc.set_date(mail.date);
    doc.set_content(mail.body);
    doc
}

// ----------------------------------------------------------------------------
/// Stop the fetch (`jobs::stop` has cancelled its downloads): nothing more
/// is written, and the job ends now. Downloads that finish anyway find no
/// fetch.
pub fn stop(app: &mut App) {
    if let Some(fetch) = &mut app.mail.fetch {
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
/// a replaced mail without text) are read if its filter says so, then the
/// analysis it asks for starts on the mails it created.
fn finish(app: &mut App) {
    let Some(fetch) = app.mail.fetch.take() else {
        return;
    };
    let stopped = app.job_stopping(fetch.job_id);
    let complete = fetch.counts.failed == 0 && !stopped;
    let (started, origin) = (fetch.started, fetch.origin);
    google::finish_sync(app, fetch.account, |account| {
        let Some(mail) = account.file.google.mail.as_mut().filter(|_| complete) else {
            return;
        };
        match (origin, &mut mail.auto_sync) {
            (Origin::Manual, _) => mail.last_fetch = Some(started),
            (Origin::AutoSync, Some(sync)) => sync.since = started,
            (Origin::AutoSync, None) => {}
        }
    });

    let counts = &fetch.counts;
    let note = match stopped {
        true => format!("stopped · {}", counts.summary()),
        false => counts.summary(),
    };
    log::info!("Mail fetch {}: {note}", fetch.job_id);
    app.end_job(fetch.job_id, &note);

    let email = find_account_email(app, fetch.account);
    let title_of = |id: Id| {
        app.vault
            .index()
            .entry(&id)
            .map(|e| e.doc.title().to_string())
    };
    if let Some((message, entity)) = new_mail_notice(&counts.created, title_of, &email) {
        app.inform(&message, entity);
    }
    let FetchCounts {
        created,
        attachments: saved,
        ..
    } = fetch.counts;
    let analysis = fetch.analysis.filter(|a| a.is_on() && !stopped);
    let then = analysis
        .filter(|_| !created.is_empty())
        .map(|analysis| (created, analysis));
    let reading = fetch.attachments.filter(|a| a.read && !stopped);
    let to_read: Vec<Id> = saved.created.into_iter().chain(saved.unread).collect();
    match reading {
        Some(reading) if !to_read.is_empty() => {
            attachments::read(app, &email, to_read, reading.model(READ_STEP), then);
        }
        _ => {
            if let Some((mails, analysis)) = then {
                enrich::analyse(app, &email, mails, &analysis);
            }
        }
    }
}

// ----------------------------------------------------------------------------
/// What the status bar says about the mails a fetch wrote (`created`):
/// nothing for none; the title of one, which a click shows; the count of
/// more, and whose they are.
fn new_mail_notice(
    created: &[Id],
    title_of: impl Fn(Id) -> Option<String>,
    email: &str,
) -> Option<(String, Option<Id>)> {
    match created {
        [] => None,
        [id] => {
            let title = title_of(*id).unwrap_or_else(|| "No subject".to_string());
            Some((format!("New mail: {title}"), Some(*id)))
        }
        many => Some((format!("{} new mails · {email}", many.len()), None)),
    }
}

// ----------------------------------------------------------------------------
/// `set_mail_sync`'s args: the fields that change.
#[derive(Deserialize)]
pub(crate) struct SyncChange {
    account: Id,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    filter: Option<MailFilter>,
    #[serde(default)]
    analysis: Option<Analysis>,
    #[serde(default)]
    attachments: Option<AttachmentSettings>,
}

// ----------------------------------------------------------------------------
/// `set_mail_sync {account, enabled?, filter?, analysis?, attachments?}`:
/// change the account's auto-sync. Its filter drops the dates (the host
/// sets them); its analysis (its pick is checked when it runs) and
/// attachments are stored as given; switching it on starts it from now --
/// only mail that arrives from then on is fetched -- and switching it off
/// keeps the filter. Answered after the `account` event.
pub fn set_mail_sync(app: &mut App, req: Req, change: SyncChange) {
    let now = DateTime::now();
    let updated = app.google.update(change.account, |account| {
        let mail = account
            .file
            .google
            .mail
            .get_or_insert_with(MailSettings::default);
        let sync = mail.auto_sync.get_or_insert_with(|| AutoSync::new(now));
        if let Some(filter) = change.filter {
            sync.filter = without_dates(filter);
        }
        if let Some(analysis) = change.analysis {
            sync.analysis = Some(analysis);
        }
        if let Some(attachments) = change.attachments {
            sync.attachments = Some(attachments);
        }
        if let Some(enabled) = change.enabled {
            if enabled && !sync.enabled {
                sync.since = now;
            }
            sync.enabled = enabled;
        }
        true
    });
    match updated {
        Ok(_) => {
            google::post(app, change.account);
            req.ok(());
        }
        Err(Error::NotFound) => req.err(Error::NotFound),
        Err(e) => {
            log::warn!("Could not save the account: {e}");
            req.fail(e, "Could not save the account.");
        }
    }
}

// ----------------------------------------------------------------------------
/// `filter` without its date conditions.
fn without_dates(mut filter: MailFilter) -> MailFilter {
    filter.since = None;
    filter.after = None;
    filter.before = None;
    filter
}

// ----------------------------------------------------------------------------
/// The Gmail query of an auto-sync: `filter`, for the mail that arrived
/// since `since`.
fn sync_query(filter: &MailFilter, since: DateTime) -> String {
    let mut filter = without_dates(filter.clone());
    filter.after = Some(since);
    filter.to_query()
}

// ----------------------------------------------------------------------------
/// A vault opened: its accounts' auto-syncs are due as soon as its scan
/// has ended.
pub(crate) fn vault_opened(app: &mut App) {
    app.mail.next_sync.clear();
    app.mail.failing.clear();
}

// ----------------------------------------------------------------------------
/// The clock ticked, or the vault scan ended (`App::on_clock`): start
/// the auto-sync of the first account that is due -- one with auto-sync on
/// and a connection, whose last run is 15 minutes ago -- unless the scan, a
/// fetch or another auto-sync still runs. One run at a time; the next tick
/// takes the next account.
pub fn auto_sync(app: &mut App, now: DateTime) {
    let mail = &app.mail;
    if app.scanning() || mail.fetch.is_some() || mail.syncing.is_some() {
        return;
    }
    let candidates: Vec<(Id, Arc<Client>, MailFilter, DateTime)> = app
        .google
        .iter()
        .filter_map(|account| {
            let sync = account.file.google.mail.as_ref()?.auto_sync.as_ref()?;
            let client = account.client.clone()?;
            sync.enabled
                .then(|| (account.id(), client, sync.filter.clone(), sync.since))
        })
        .collect();
    let ids = candidates.iter().map(|(id, ..)| *id);
    let Some(account) = first_due(ids, &mail.next_sync, now) else {
        return;
    };
    let Some((_, client, filter, since)) = candidates.into_iter().find(|(id, ..)| *id == account)
    else {
        return;
    };

    app.mail.next_sync.insert(account, later(now, SYNC_EVERY));
    app.mail.syncing = Some(account);
    let root = app.vault.root().map(Path::to_path_buf);
    let query = sync_query(&filter, since);
    let started = DateTime::now();
    log::info!("Mail auto-sync of {account}: searching `{query}`");
    app.run(
        move |cancel| gmail::list_message_ids(&client, cancel, &query, MATCH_CAP),
        move |app, result| {
            let ids = result.map(|(ids, _truncated)| ids);
            on_sync_listed(app, account, root, started, ids);
        },
    );
}

// ----------------------------------------------------------------------------
/// An auto-sync's search answered: fetch the new mails as a job -- or do
/// nothing, if there are none (no job, no write). Dropped if another vault
/// opened or auto-sync was switched off meanwhile.
fn on_sync_listed(
    app: &mut App,
    account: Id,
    root: Option<PathBuf>,
    started: DateTime,
    result: Result<Vec<String>>,
) {
    app.mail.syncing = None;
    let sync = app
        .google
        .get(account)
        .ok()
        .and_then(|a| a.file.google.mail.as_ref()?.auto_sync.as_ref())
        .filter(|sync| sync.enabled);
    // As it is now: a change made while the search ran counts.
    let analysis = sync.and_then(|sync| sync.analysis.clone());
    let attachments = sync
        .and_then(|sync| sync.attachments.clone())
        .filter(|a| a.enabled);
    if app.vault.root() != root.as_deref() || sync.is_none() {
        return log::info!("Mail auto-sync of {account}: dropped, the vault or setting changed");
    }
    let ids = match result {
        Ok(ids) => {
            app.mail.failing.remove(&account);
            ids
        }
        Err(e) => {
            log::warn!("Mail auto-sync of {account} failed: {e}");
            // Shown once, not every 15 minutes while offline.
            if app.mail.failing.insert(account) {
                let email = find_account_email(app, account);
                app.notice(&format!("Mail auto-sync of {email} failed."));
            }
            return;
        }
    };
    let known = app
        .vault
        .index()
        .remote_values(DocumentType::Email, Some(&account), GMAIL_ID);
    let (queue, _) = plan_fetch(ids, &HashSet::new(), &known);
    if queue.is_empty() {
        return log::info!("Mail auto-sync of {account}: nothing new");
    }
    if app.mail.fetch.is_some() {
        // A fetch started meanwhile: again at the next tick.
        app.mail.next_sync.insert(account, DateTime::now());
        return;
    }
    let Ok(client) = app.google.connected(account) else {
        return;
    };
    let email = find_account_email(app, account);
    let plan = FetchPlan {
        account,
        client,
        queue,
        skipped: 0,
        replace: HashSet::new(),
        origin: Origin::AutoSync,
        started,
        analysis,
        attachments,
    };
    start_fetch(app, format!("Syncing mail · {email}"), plan);
    pump(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Harness;
    use crate::testing::{CONNECTED, DISCONNECTED, parse, shape, with_google};
    use serde_json::Value;
    use std::fs;

    /// The accounts' access (`with_google`).
    const MAIL: &str = "  scopes:\n    - email: read\n";
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
    fn mail(message_id: &str, body: &str) -> Downloaded {
        let date = DateTime::from_unix_millis(1_790_000_000_000);
        let mut mail = Downloaded::parse(RAW.as_bytes(), date, None);
        mail.message_id = message_id.to_string();
        mail.body = body.to_string();
        mail
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_raw_mail_parses_into_its_parts() {
        let date = DateTime::from_unix_millis(1_790_000_000_000);
        let mail = Downloaded::parse(RAW.as_bytes(), date, None);
        assert_eq!(mail.subject, "Nebenkosten 2026");
        assert_eq!(mail.from, "\"Hausverwaltung Krause\" <info@krause.example>");
        assert_eq!(mail.to, ["Jan <jan@example.com>", "maya@example.org"]);
        assert_eq!(mail.cc, ["lisa@example.org"]);
        assert_eq!(mail.message_id, "<abc.123@krause.example>");
        assert_eq!(mail.date, date);
        assert!(
            mail.body.contains("](https://krause.example/bill)"),
            "the HTML part, its link kept: {}",
            mail.body
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_mail_becomes_an_email_entity() {
        let account = Id::parse(CONNECTED).unwrap();
        let doc = email_entity(account, "g1", mail("<m1@x>", "Body"));
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

        let mut untitled = mail("", "Body");
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
        let mut h = with_google("mail-apply", MAIL, &[("Mail/known.md", known)]);
        let account = Id::parse(CONNECTED).unwrap();

        let stored = apply_mail(&mut h.app, account, "g1", mail("<m1@x>", "First"), false);
        assert!(matches!(stored.unwrap(), (Stored::Created(_), _)));
        let sent = parse(&h.take());
        assert_eq!(shape(&sent), ["entity"]);
        let path = h.vault().join(sent[0]["data"]["path"].as_str().unwrap());
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("gmail-id: g1") && text.contains("message-id: <m1@x>"),
            "{text}"
        );

        let again = apply_mail(&mut h.app, account, "g1", mail("<m1@x>", "Second"), false);
        assert_eq!(again.unwrap().0, Stored::Skipped);
        assert!(h.take().is_empty());

        let replaced = apply_mail(&mut h.app, account, "g1", mail("<m1@x>", "Second"), true);
        assert_eq!(replaced.unwrap().0, Stored::Replaced);
        let sent = parse(&h.take());
        assert!(shape(&sent).contains(&"content"), "{sent:?}");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.trim_end().ends_with("Second"), "{text}");

        let by_message_id = apply_mail(&mut h.app, account, "g9", mail("<known@x>", "x"), false);
        assert_eq!(
            by_message_id.unwrap().0,
            Stored::Skipped,
            "known from another account"
        );
    }

    /// A mail with a PDF, a logo the HTML shows, and a zip.
    const RAW_ATTACHED: &str = "From: info@krause.example\r\n\
Subject: Nebenkosten 2026\r\n\
Message-ID: <att.1@krause.example>\r\n\
Content-Type: multipart/mixed; boundary=\"m\"\r\n\
\r\n\
--m\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>The bill is attached.</p><img src=\"cid:logo\">\r\n\
--m\r\n\
Content-Type: image/png; name=\"logo.png\"\r\n\
Content-ID: <logo>\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--m\r\n\
Content-Type: application/pdf; name=\"Abrechnung 2026.pdf\"\r\n\
Content-Disposition: attachment; filename=\"Abrechnung 2026.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjcK\r\n\
--m\r\n\
Content-Type: application/zip; name=\"belege.zip\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
UEsDBA==\r\n\
--m--\r\n";

    // ------------------------------------------------------------------------
    /// Attachment settings as the page sends them.
    fn keep(patterns: &[&str], max_size: u32) -> AttachmentSettings {
        let settings = json!({ "enabled": true, "patterns": patterns, "max-size": max_size });
        serde_json::from_value(settings).unwrap()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_wanted_attachments_are_kept_by_pattern_size_and_kind() {
        let date = DateTime::from_unix_millis(1_790_000_000_000);
        let names = |keep: Option<&AttachmentSettings>| {
            let mail = Downloaded::parse(RAW_ATTACHED.as_bytes(), date, keep);
            let names: Vec<String> = mail
                .attachments
                .iter()
                .map(|a| a.filename.clone())
                .collect();
            names
        };
        assert!(names(None).is_empty(), "off: none kept");
        assert_eq!(names(Some(&keep(&["*.PDF"], 10))), ["Abrechnung 2026.pdf"]);
        assert_eq!(
            names(Some(&keep(&["*.pdf", "*.zip", "*.png"], 10))),
            ["Abrechnung 2026.pdf", "belege.zip"],
            "never the logo the HTML shows"
        );
        assert!(
            names(Some(&keep(&["*.pdf"], 0))).is_empty(),
            "over the size"
        );

        let mail = Downloaded::parse(RAW_ATTACHED.as_bytes(), date, Some(&keep(&["*"], 10)));
        assert_eq!(mail.attachments[0].bytes, b"%PDF-1.7\n");
        assert!(mail.body.contains("Attachments: "), "{}", mail.body);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_attachment_becomes_a_document_linked_to_its_mail() {
        let (account, mail) = (connected_id(), Id::generate());
        let date = DateTime::from_unix_millis(1_790_000_000_000);
        let to = AttachedTo {
            mail,
            account,
            gmail_id: "g1",
            message_id: "<m1@x>",
            date,
        };
        let part = email::Attachment {
            filename: " Abrechnung 2026.pdf ".into(),
            content_type: "application/pdf".into(),
            inline: false,
            bytes: b"%PDF-".to_vec(),
        };
        let doc = attachment_entity(&to, &part);
        assert_eq!(doc.title(), "Abrechnung 2026");
        assert_eq!((doc.date(), doc.linked_to()), (Some(date), Some(mail)));
        assert_eq!(doc.kind(), DocumentType::Document);
        let Some(Source::Remote(remote)) = doc.source() else {
            panic!("{:?}", doc.source());
        };
        assert_eq!(remote.account, Some(account));
        assert_eq!(remote.get(MESSAGE_ID), Some("<m1@x>"));
        assert_eq!(remote.get(GMAIL_ID), Some("g1"));

        let nameless = email::Attachment {
            filename: ".pdf".into(),
            ..part
        };
        assert_eq!(attachment_entity(&to, &nameless).title(), ".pdf");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn applying_saves_each_attachment_once_beside_its_document() {
        let mut h = with_google("mail-attachments", MAIL, &[]);
        let account = connected_id();
        let date = DateTime::from_unix_millis(1_790_000_000_000);
        let attached =
            || Downloaded::parse(RAW_ATTACHED.as_bytes(), date, Some(&keep(&["*.pdf"], 10)));

        let (stored, saved) = apply_mail(&mut h.app, account, "g1", attached(), false).unwrap();
        let Stored::Created(mail) = stored else {
            panic!("{stored:?}");
        };
        assert_eq!(saved.created.len(), 1);
        assert_eq!((saved.unread.len(), saved.failed), (0, 0));
        let sent = parse(&h.take());
        assert_eq!(shape(&sent), ["entity", "entity"], "{sent:?}");
        let doc = &sent[1]["data"];
        assert_eq!(doc["linked_to"], json!(mail));
        assert!(
            doc["path"].as_str().unwrap().starts_with("Attachments"),
            "{doc}"
        );
        let original = &doc["document"]["original"];
        assert_eq!(original["name"], "Abrechnung 2026.pdf");
        assert_eq!(original["size"], 9);
        let id = Id::parse(doc["id"].as_str().unwrap()).unwrap();
        let path = h.app.vault.index().original_path(&id).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"%PDF-1.7\n");
        assert!(path.starts_with(h.vault().join("Attachments")));

        // A replaced mail gets only what it doesn't have yet, and names
        // what it has without text, to be read.
        let (stored, saved) = apply_mail(&mut h.app, account, "g1", attached(), true).unwrap();
        let unread = Saved {
            unread: vec![id],
            ..Saved::default()
        };
        assert_eq!((stored, saved), (Stored::Replaced, unread));
        let both = Some(&keep(&["*.pdf", "*.zip"], 10));
        let mail = Downloaded::parse(RAW_ATTACHED.as_bytes(), date, both);
        let (_, saved) = apply_mail(&mut h.app, account, "g1", mail, true).unwrap();
        assert_eq!(
            (saved.created.len(), saved.unread, saved.failed),
            (1, vec![id], 0)
        );
        let text = "Read.\n".to_string();
        let written = h
            .app
            .edit(|index, changes| index.set_content(&id, text, changes));
        assert!(written.is_ok());
        let (_, saved) = apply_mail(&mut h.app, account, "g1", attached(), true).unwrap();
        assert_eq!(saved.unread.len(), 1, "the zip's document only");

        // The attachments name the mail's message, but aren't taken for it.
        let index = h.app.vault.index();
        assert_eq!(
            existing_mail(index, account, "g1", ""),
            Some(index.all()[0].doc.id())
        );
        let known = KnownMails::of(&h.app, account);
        assert_eq!(known.gmail.len(), 1);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn requests_need_a_connected_account() {
        let mut h = with_google("mail-accounts", MAIL, &[]);
        let unknown = Id::generate();
        for account in [unknown.to_string(), DISCONNECTED.to_string()] {
            let requests = [
                ("mail_labels", json!({ "account": account })),
                ("mail_search", json!({ "account": account, "filter": {} })),
                ("mail_headers", json!({ "account": account, "ids": ["g1"] })),
                ("mail_fetch", json!({ "account": account, "ids": [] })),
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
        let mut h = with_google("mail-fetch", MAIL, &[]);
        let fetch = |id: u64| {
            let filter = json!({ "starred": true, "since": "week" });
            let args =
                json!({ "account": CONNECTED, "ids": [], "filter": filter, "auto_update": false });
            json!({ "id": id, "cmd": "mail_fetch", "args": args }).to_string()
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
        assert_eq!(sent[3]["data"]["note"], "nothing to fetch");
        assert_eq!(sent[4]["err"]["code"], "unsupported", "one fetch at a time");
        assert_eq!(sent[4]["err"]["message"], "Another fetch is still running.");

        let account = &sent[2]["data"];
        assert!(account["last_sync"].is_string(), "{account}");
        assert!(account["mail"]["last_fetch"].is_string(), "{account}");
        let file = h.vault().join(".avelor/accounts/google_a.yaml");
        let text = fs::read_to_string(file).unwrap();
        assert!(
            text.contains("last-sync:")
                && text.contains("last-fetch:")
                && text.contains("starred: true"),
            "{text}"
        );
        assert_eq!(account["mail"]["auto_update"], Value::Bool(false));
    }

    // ------------------------------------------------------------------------
    fn connected_id() -> Id {
        Id::parse(CONNECTED).unwrap()
    }

    // ------------------------------------------------------------------------
    /// Switch the connected account's auto-sync on, with `filter`.
    fn switch_on(h: &mut Harness, filter: Value) {
        let args = json!({ "account": CONNECTED, "enabled": true, "filter": filter });
        let sent = h.call("set_mail_sync", args);
        assert_eq!(sent.last().unwrap()["ok"], Value::Null, "{sent:?}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_auto_sync_asks_for_the_mail_since() {
        let since = DateTime::from_unix_millis(1_790_000_000_000);
        let filter = MailFilter {
            labels: vec!["Umzug".into()],
            starred: true,
            since: Some("week".into()),
            after: Some(DateTime::from_unix_millis(1_700_000_000_000)),
            before: Some(DateTime::from_unix_millis(1_800_000_000_000)),
            ..MailFilter::default()
        };
        let expected = MailFilter {
            labels: vec!["Umzug".into()],
            starred: true,
            after: Some(since),
            ..MailFilter::default()
        };
        assert_eq!(sync_query(&filter, since), expected.to_query());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn auto_sync_starts_from_now_and_switched_off_keeps_its_filter() {
        let mut h = with_google("mail-sync-set", MAIL, &[]);
        let before = DateTime::from_unix_millis(DateTime::now().as_unix() * 1000);
        let filter =
            json!({ "labels": ["Umzug"], "since": "week", "after": "2026-01-01T00:00:00Z" });
        let args = json!({ "account": CONNECTED, "enabled": true, "filter": filter });
        let sent = h.call("set_mail_sync", args);
        assert_eq!(shape(&sent), ["account", "reply"]);
        let on = sent[0]["data"]["mail"]["auto_sync"].clone();
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
        let sent = h.call("set_mail_sync", args);
        let off = &sent[0]["data"]["mail"]["auto_sync"];
        assert_eq!(off["enabled"], false);
        assert_eq!(
            (&off["filter"], &off["since"]),
            (&on["filter"], &on["since"])
        );

        let old = DateTime::from_unix_millis(1_700_000_000_000);
        h.app
            .google
            .update(connected_id(), |account| {
                let mail = account.file.google.mail.as_mut().unwrap();
                mail.auto_sync.as_mut().unwrap().since = old;
                true
            })
            .unwrap();
        let sent = h.call(
            "set_mail_sync",
            json!({ "account": CONNECTED, "enabled": true }),
        );
        let again = &sent[0]["data"]["mail"]["auto_sync"]["since"];
        let again = DateTime::from_iso8601(again.as_str().unwrap()).unwrap();
        assert!(again >= before, "switched on again: from now");

        let unknown = json!({ "account": Id::generate(), "enabled": true });
        assert_eq!(
            h.call("set_mail_sync", unknown)[0]["err"]["code"],
            "not_found"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn auto_sync_waits_for_its_time_a_fetch_and_the_vault_scan() {
        let mut h = with_google("mail-sync-wait", MAIL, &[]);
        let now = DateTime::now();
        auto_sync(&mut h.app, now);
        assert!(h.app.mail.syncing.is_none(), "not switched on");

        switch_on(&mut h, json!({}));
        let account = connected_id();
        h.app.mail.next_sync.insert(account, later(now, 60));
        auto_sync(&mut h.app, now);
        assert!(h.app.mail.syncing.is_none(), "not due yet");

        h.app.mail.next_sync.clear();
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
            attachments: None,
        };
        start_fetch(&mut h.app, "Fetching".into(), plan);
        auto_sync(&mut h.app, now);
        assert!(h.app.mail.syncing.is_none(), "a fetch runs");
        finish(&mut h.app);

        // Opening the vault again: the scan runs first.
        let root = h.vault();
        crate::api::switch_to(&mut h.app, root.clone());
        assert!(h.app.scanning());
        auto_sync(&mut h.app, now);
        assert!(h.app.mail.syncing.is_none(), "the vault scan runs");
        // Not due when the scan ends, so no search goes out in this test.
        h.app.mail.next_sync.insert(account, later(now, 60));
        h.wait_scanned();
        assert!(!h.app.scanning() && h.app.mail.syncing.is_none());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_listing_with_nothing_new_leaves_no_trace_and_a_failure_shows_once() {
        let known = format!(
            "---\nid: 01a0e1cc-a24c-7490-82fe-e75669985b31\ntitle: Known\nremote:\n  account: {CONNECTED}\n  remote:\n  - gmail-id: g1\nemail: {{}}\n---\nBody\n"
        );
        let mut h = with_google("mail-sync-listed", MAIL, &[("Mail/known.md", known)]);
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
        assert!(h.app.mail.fetch.is_none());

        let other = Some(h.vault().with_file_name("other"));
        on_sync_listed(&mut h.app, account, other, now, Ok(vec!["g2".into()]));
        assert!(h.app.mail.fetch.is_none(), "another vault: dropped");

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
            attachments: None,
        };
        let manual = start_fetch(&mut h.app, "Fetching".into(), plan);
        on_sync_listed(&mut h.app, account, root, now, Ok(vec!["g2".into()]));
        let fetch = h.app.mail.fetch.as_ref().unwrap();
        assert_eq!(fetch.job_id, manual, "the manual fetch goes on");
        let next = h.app.mail.next_sync[&account];
        assert!(next <= DateTime::now(), "again at the next tick");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_complete_auto_sync_moves_its_start_on() {
        let mut h = with_google("mail-sync-finish", MAIL, &[]);
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
            attachments: None,
        };
        start_fetch(&mut h.app, "Syncing mail".into(), plan);
        finish(&mut h.app);
        h.run_queue();
        let sent = parse(&h.take());
        assert_eq!(shape(&sent), ["job", "account", "job"], "{sent:?}");
        let account = &sent[1]["data"];
        assert_eq!(account["mail"]["auto_sync"]["since"], started.as_iso8601());
        assert_eq!(
            account["mail"]["last_fetch"],
            Value::Null,
            "the page's stays"
        );
        assert!(account["last_sync"].is_string());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn new_mail_is_told_by_its_title_or_its_count() {
        let (a, b) = (Id::generate(), Id::generate());
        let title_of = |id: Id| (id == a).then(|| "Nebenkosten 2026".to_string());
        assert_eq!(new_mail_notice(&[], title_of, "jan@example.com"), None);
        assert_eq!(
            new_mail_notice(&[a], title_of, "jan@example.com"),
            Some(("New mail: Nebenkosten 2026".to_string(), Some(a)))
        );
        assert_eq!(
            new_mail_notice(&[b], title_of, "jan@example.com"),
            Some(("New mail: No subject".to_string(), Some(b))),
            "gone meanwhile"
        );
        assert_eq!(
            new_mail_notice(&[a, b], title_of, "jan@example.com"),
            Some(("2 new mails · jan@example.com".to_string(), None))
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_fetch_that_wrote_mail_tells_the_page_after_its_end() {
        let known = format!(
            "---\nid: 01a0e1cc-a24c-7490-82fe-e75669985b32\ntitle: Nebenkosten 2026\nremote:\n  account: {CONNECTED}\n  remote:\n  - gmail-id: g1\nemail: {{}}\n---\nBody\n"
        );
        let mut h = with_google("mail-new-notice", MAIL, &[("Mail/n.md", known)]);
        let account = connected_id();
        let mail = Id::parse("01a0e1cc-a24c-7490-82fe-e75669985b32").unwrap();
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
            attachments: None,
        };
        start_fetch(&mut h.app, "Fetching".into(), plan);
        h.app.mail.fetch.as_mut().unwrap().counts.created.push(mail);
        finish(&mut h.app);
        let sent = parse(&h.take());
        assert_eq!(
            shape(&sent),
            ["job", "account", "job", "notice"],
            "{sent:?}"
        );
        assert_eq!(sent[2]["data"]["note"], "1 new");
        assert_eq!(
            sent[3]["data"],
            json!({ "level": "info", "message": "New mail: Nebenkosten 2026", "entity": mail })
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_fetch_with_the_analysis_on_needs_a_model_and_remembers_it() {
        let mut h = with_google("mail-fetch-analysis", MAIL, &[]);
        let model = json!({ "account": Id::generate(), "model": "m" });
        let fetch = |level: &str| {
            let analysis = json!({ "level": level, "model": model });
            json!({ "account": CONNECTED, "ids": [], "analysis": analysis })
        };
        let sent = h.call("mail_fetch", fetch("lite"));
        assert_eq!(shape(&sent), ["reply"], "nothing fetched: {sent:?}");
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        assert_eq!(
            sent[0]["err"]["message"],
            "For the mail analysis, the picked model\u{2019}s account is gone; pick another model."
        );
        let file = h.vault().join(".avelor/accounts/google_a.yaml");
        assert!(!fs::read_to_string(&file).unwrap().contains("analysis"));

        let sent = h.call("mail_fetch", fetch("off"));
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
        let mut h = with_google("mail-sync-analysis", MAIL, &[]);
        let analysis = json!({ "level": "medium" });
        let args = json!({ "account": CONNECTED, "enabled": true, "analysis": analysis });
        let sent = h.call("set_mail_sync", args);
        assert_eq!(shape(&sent), ["account", "reply"]);
        let sync = &sent[0]["data"]["mail"]["auto_sync"];
        assert_eq!(
            sync["analysis"],
            json!({ "level": "medium" }),
            "no model yet"
        );
        let file = h.vault().join(".avelor/accounts/google_a.yaml");
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("level: medium"), "{text}");

        let attachments = json!({ "enabled": true, "patterns": ["*.pdf"], "max-size": 5 });
        let args = json!({ "account": CONNECTED, "attachments": attachments });
        let sent = h.call("set_mail_sync", args);
        assert_eq!(
            sent[0]["data"]["mail"]["auto_sync"]["attachments"],
            attachments
        );
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("      attachments:\n        enabled: true\n"),
            "{text}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_fetch_remembers_its_attachments() {
        let mut h = with_google("mail-fetch-attachments", MAIL, &[]);
        let attachments = json!({ "enabled": false, "patterns": ["*.pdf"], "max-size": 10 });
        let args = json!({ "account": CONNECTED, "ids": [], "attachments": attachments });
        let mut sent = h.call("mail_fetch", args);
        assert!(sent.iter().any(|m| m["ok"]["job"].is_string()), "{sent:?}");
        h.run_queue();
        sent.extend(parse(&h.take()));
        let account = sent.iter().find(|m| m["ev"] == "account").unwrap();
        assert_eq!(
            account["data"]["mail"]["attachments"], attachments,
            "kept while off"
        );

        // Read on, the pick is checked first.
        let model = json!({ "account": Id::generate(), "model": "m" });
        let attachments = json!({ "enabled": true, "read": true, "models": { "read": model } });
        let args = json!({ "account": CONNECTED, "ids": [], "attachments": attachments });
        let sent = h.call("mail_fetch", args);
        assert_eq!(shape(&sent), ["reply"], "nothing fetched: {sent:?}");
        assert_eq!(
            sent[0]["err"]["message"],
            "For reading attachments, the picked model\u{2019}s account is gone; pick another model."
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_finished_fetch_reads_its_attachments_before_the_analysis() {
        let mut h = with_google("mail-read-after", MAIL, &[]);
        let account = connected_id();
        let lite = Analysis {
            level: crate::google_accounts::AnalysisLevel::Lite,
            model: None,
        };
        let reading = json!({ "enabled": true, "patterns": ["*.pdf"], "read": true });
        let client = h.app.google.connected(account).unwrap();
        let plan = FetchPlan {
            account,
            client,
            queue: VecDeque::new(),
            skipped: 0,
            replace: HashSet::new(),
            origin: Origin::Manual,
            started: DateTime::now(),
            analysis: Some(lite),
            attachments: Some(serde_json::from_value(reading).unwrap()),
        };
        start_fetch(&mut h.app, "Fetching".into(), plan);
        let fetch = h.app.mail.fetch.as_mut().unwrap();
        fetch.counts.created.push(Id::generate());
        fetch.counts.attachments.unread.push(Id::generate());
        finish(&mut h.app);
        let sent = parse(&h.take());
        // No AI account in this vault: the reading says so, then the
        // analysis.
        let notices: Vec<&str> = sent
            .iter()
            .filter(|m| m["ev"] == "notice" && m["data"]["level"] == "error")
            .filter_map(|m| m["data"]["message"].as_str())
            .collect();
        assert_eq!(notices.len(), 2, "{sent:?}");
        assert!(
            notices[0].starts_with("Reading attachments \u{b7} "),
            "{notices:?}"
        );
        assert!(
            notices[0].ends_with("For reading attachments, add an AI provider under Accounts.")
        );
        assert!(notices[1].contains("For the mail analysis, add an AI provider"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_finished_fetch_hands_its_new_mails_to_the_analysis() {
        let mut h = with_google("mail-analyse-after", MAIL, &[]);
        let account = connected_id();
        let lite = Analysis {
            level: crate::google_accounts::AnalysisLevel::Lite,
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
                attachments: None,
            };
            let job = start_fetch(&mut h.app, "Fetching".into(), plan);
            let fetch = h.app.mail.fetch.as_mut().unwrap();
            fetch.counts.created.push(Id::generate());
            if stopped {
                crate::jobs::stop(&mut h.app, job);
            } else {
                finish(&mut h.app);
            }
            let sent = parse(&h.take());
            // No AI account in this vault: the analysis says so.
            let refused = sent.iter().any(|m| {
                let message = m["data"]["message"].as_str().unwrap_or_default();
                message.contains("For the mail analysis, add an AI provider")
            });
            assert_eq!(refused, !stopped, "stopped: {stopped}, {sent:?}");
        }
    }
}
