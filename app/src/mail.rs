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

use std::collections::{HashSet, VecDeque};
use std::path::Path;
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
use av_vault::envelope::{Email, Remote, Roles, Source};
use av_vault::shared::OneOrMany;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::app::App;
use crate::google;
use crate::google_accounts::MailSettings;
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use crate::protocol::Req;
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
/// a time).
#[derive(Default)]
pub struct MailState {
    fetch: Option<MailFetch>,
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
    /// When the fetch started: what it fetched covers the mail up to then.
    started: DateTime,
}

// ----------------------------------------------------------------------------
/// A fetch to start: what `start_fetch` makes a `MailFetch` and a job of.
struct FetchPlan {
    account: Id,
    client: Arc<Client>,
    queue: VecDeque<String>,
    skipped: usize,
    replace: HashSet<String>,
    started: DateTime,
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
}

impl FetchCounts {
    // ------------------------------------------------------------------------
    /// "22 new · 3 replaced · 1 failed": the counts that aren't zero.
    fn summary(&self) -> String {
        let counts = [
            (self.created.len(), "new", "new"),
            (self.replaced, "replaced", "replaced"),
            (self.skipped, "skipped", "skipped"),
            (self.failed, "failed", "failed"),
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
            gmail: app.vault.index().remote_values(Some(&account), GMAIL_ID),
            message: app.vault.index().remote_values(None, MESSAGE_ID),
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
/// `mail_fetch {account, ids, replace?, filter?, auto_update?}`:
/// fetch the mails into the vault as a job, answering `{job}` once it
/// started. The job's events follow, an `entity` event per mail written
/// (with `content` for a replaced body), a `notice` per mail that failed,
/// and finally the account (`account`: its last fetch and sync). The filter
/// and mode are remembered in the account first.
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
    let remembered = Remembered {
        filter: request.filter,
        auto_update: request.auto_update,
    };
    remember_settings(app, account, remembered);

    let replace: HashSet<String> = request.replace.into_iter().collect();
    let known = app.vault.index().remote_values(Some(&account), GMAIL_ID);
    let (queue, skipped) = plan_fetch(request.ids, &replace, &known);
    let empty = queue.is_empty();
    let email = find_account_email(app, account);
    let plan = FetchPlan {
        account,
        client,
        queue,
        skipped,
        replace,
        started: DateTime::now(),
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
    log::info!("Mail fetch {job_id}: {total} mails from {account}, {skipped} in the vault already");
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
        started: plan.started,
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
}

// ----------------------------------------------------------------------------
/// Store the page's filter and mode in the account file (best
/// effort: a failed save only costs the pre-filled form next time).
fn remember_settings(app: &mut App, account: Id, remembered: Remembered) {
    let remembered = app.google.update(account, |account| {
        let mail = account
            .file
            .google
            .mail
            .get_or_insert_with(MailSettings::default);
        mail.filter = remembered.filter;
        mail.auto_update = remembered.auto_update;
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
    for id in starting {
        let client = Arc::clone(&client);
        let gmail_id = id.clone();
        app.run_job(
            job,
            move |cancel| download(&client, cancel, &gmail_id),
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
}

impl Downloaded {
    // ------------------------------------------------------------------------
    /// The mail `raw` (RFC 822) as it is written: its headers, its body as
    /// markdown, dated `date`.
    fn parse(raw: &[u8], date: DateTime) -> Self {
        let mail = email::parse(raw);
        Self {
            body: mail.to_markdown(),
            to: email::split_addresses(&mail.to),
            cc: email::split_addresses(&mail.cc),
            message_id: mail.message_id,
            subject: mail.subject,
            from: mail.from,
            date,
        }
    }
}

// ----------------------------------------------------------------------------
/// The run step: download the mail `id` and parse it, dated when Gmail got
/// it (now, if Gmail doesn't say).
fn download(client: &Client, cancel: &Cancel, id: &str) -> Result<Downloaded> {
    let raw = gmail::get_raw_message(client, cancel, id)?;
    let date = match raw.epoch_ms {
        0 => DateTime::now(),
        ms => DateTime::from_unix_millis(ms),
    };
    Ok(Downloaded::parse(&raw.bytes, date))
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
        Ok(Stored::Created(id)) => counts.created.push(id),
        Ok(Stored::Replaced) => counts.replaced += 1,
        Ok(Stored::Skipped) => counts.skipped += 1,
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
        .find_remote(Some(&account), GMAIL_ID, gmail_id)
        .or_else(|| {
            (!message_id.is_empty())
                .then(|| index.find_remote(None, MESSAGE_ID, message_id))
                .flatten()
        })
}

// ----------------------------------------------------------------------------
/// The apply step: write `mail` into the vault as `fetch_action` says -- a
/// new entity in `Mail/`, the body of the one already there replaced, or
/// nothing.
fn apply_mail(
    app: &mut App,
    account: Id,
    gmail_id: &str,
    mail: Downloaded,
    replace: bool,
) -> Result<Stored> {
    let existing = existing_mail(app.vault.index(), account, gmail_id, &mail.message_id);
    // A replaced body goes to the page with the entity (`App::edit`), so
    // a preview showing the old one loads it.
    match fetch_action(existing, replace) {
        Action::Skip => Ok(Stored::Skipped),
        Action::Replace(id) => {
            let outcome = app.edit(|index, changes| index.set_content(&id, mail.body, changes))?;
            app.vault.require_written(outcome)?;
            Ok(Stored::Replaced)
        }
        Action::Create => {
            let doc = email_entity(account, gmail_id, mail);
            let folder = Path::new(layout::MAIL);
            let id = app.edit(|index, changes| index.create(doc, folder, changes))?;
            Ok(Stored::Created(id))
        }
    }
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

    let mut remote = Remote {
        account: Some(account),
        remote: Vec::new(),
    };
    if !mail.message_id.is_empty() {
        remote.set(MESSAGE_ID, &mail.message_id);
    }
    remote.set(GMAIL_ID, gmail_id);
    let source = Source::Remote(remote);

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
/// when it started (the page's "Since last time") only if it fetched all
/// and nothing failed, so the next one still covers what it didn't write --
/// and end the job with its summary.
fn finish(app: &mut App) {
    let Some(fetch) = app.mail.fetch.take() else {
        return;
    };
    let stopped = app.job_stopping(fetch.job_id);
    let complete = fetch.counts.failed == 0 && !stopped;
    let started = fetch.started;
    google::finish_sync(app, fetch.account, |account| {
        let Some(mail) = account.file.google.mail.as_mut().filter(|_| complete) else {
            return;
        };
        mail.last_fetch = Some(started);
    });

    let counts = &fetch.counts;
    let note = match stopped {
        true => format!("stopped · {}", counts.summary()),
        false => counts.summary(),
    };
    log::info!("Mail fetch {}: {note}", fetch.job_id);
    app.end_job(fetch.job_id, &note);
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let mut mail = Downloaded::parse(RAW.as_bytes(), date);
        mail.message_id = message_id.to_string();
        mail.body = body.to_string();
        mail
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_raw_mail_parses_into_its_parts() {
        let date = DateTime::from_unix_millis(1_790_000_000_000);
        let mail = Downloaded::parse(RAW.as_bytes(), date);
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
        assert!(matches!(stored.unwrap(), Stored::Created(_)));
        let sent = parse(&h.take());
        assert_eq!(shape(&sent), ["entity"]);
        let path = h.vault().join(sent[0]["data"]["path"].as_str().unwrap());
        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("gmail-id: g1") && text.contains("message-id: <m1@x>"),
            "{text}"
        );

        let again = apply_mail(&mut h.app, account, "g1", mail("<m1@x>", "Second"), false);
        assert_eq!(again.unwrap(), Stored::Skipped);
        assert!(h.take().is_empty());

        let replaced = apply_mail(&mut h.app, account, "g1", mail("<m1@x>", "Second"), true);
        assert_eq!(replaced.unwrap(), Stored::Replaced);
        let sent = parse(&h.take());
        assert!(shape(&sent).contains(&"content"), "{sent:?}");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.trim_end().ends_with("Second"), "{text}");

        let by_message_id = apply_mail(&mut h.app, account, "g9", mail("<known@x>", "x"), false);
        assert_eq!(
            by_message_id.unwrap(),
            Stored::Skipped,
            "known from another account"
        );
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
}
