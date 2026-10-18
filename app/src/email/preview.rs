//! The import page's preview: the account's labels, a search's matches and
//! their headers.

use std::collections::HashSet;

use av_google::gmail::{self, EmailFilter, EmailHeader};
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::email;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::MATCH_CAP;
use crate::app::App;
use crate::google;
use crate::protocol::Req;
use crate::texts::Text;
use av_vault::schema::remote::{GMAIL_ID, MESSAGE_ID};

// ----------------------------------------------------------------------------
/// The superseding channel of `email_search` and `email_headers`: a newer
/// preview request aborts the older one.
const PREVIEW: &str = "email-preview";
/// Matches per preview page; `email_search` answers with the first page's
/// headers.
const PAGE_SIZE: usize = 50;

// ----------------------------------------------------------------------------
/// `email_search`'s args.
#[derive(Deserialize)]
pub(crate) struct SearchRequest {
    pub(super) account: Id,
    pub(super) filter: EmailFilter,
}

// ----------------------------------------------------------------------------
/// `email_headers`' args: the ids of a page of matches.
#[derive(Deserialize)]
pub(crate) struct HeadersRequest {
    pub(super) account: Id,
    pub(super) ids: Vec<String>,
}

// ----------------------------------------------------------------------------
/// One row of the preview (`email_search`, `email_headers`).
#[derive(Serialize)]
struct HeaderInfo {
    pub(super) id: String,
    pub(super) date: DateTime,
    pub(super) from: String,
    pub(super) subject: String,
    in_vault: bool,
}

// ----------------------------------------------------------------------------
#[derive(Serialize)]
pub(super) struct MatchInfo<'a> {
    pub(super) id: &'a str,
    in_vault: bool,
}

// ----------------------------------------------------------------------------
/// `email_search`'s answer.
#[derive(Serialize)]
struct Matches<'a> {
    pub(super) matches: Vec<MatchInfo<'a>>,
    /// There are more than `matches` (`MATCH_CAP`).
    truncated: bool,
    /// The first page's headers.
    headers: Vec<HeaderInfo>,
}

// ----------------------------------------------------------------------------
#[derive(Serialize)]
struct LabelInfo<'a> {
    pub(super) id: &'a str,
    name: &'a str,
}

// ----------------------------------------------------------------------------
/// The emails the vault already holds: by Gmail id from this account, by
/// Message-ID from anywhere (another account, an imported `.eml`).
struct KnownEmails {
    pub(super) gmail: HashSet<String>,
    message: HashSet<String>,
}

impl KnownEmails {
    // ------------------------------------------------------------------------
    fn of(app: &App, account: Id) -> Self {
        Self {
            gmail: app.vault.index().remote_values(Some(&account), GMAIL_ID),
            message: app.vault.index().remote_values(None, MESSAGE_ID),
        }
    }

    // ------------------------------------------------------------------------
    fn header(&self, header: EmailHeader) -> HeaderInfo {
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
/// Answer a request that failed with `error`, `text` saying what failed.
fn fail(req: Req, error: Error, text: Text) {
    log::info!("Email request failed: {text}: {error}");
    req.fail(error, text);
}

// ----------------------------------------------------------------------------
/// `email_labels {account}`: the account's own labels, `{labels: [{id,
/// name}]}`.
pub fn email_labels(app: &mut App, req: Req, account: Id) {
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
            Err(e) => fail(req, e, Text::new("email.labels_failed")),
        },
    );
}

// ----------------------------------------------------------------------------
/// `email_search {account, filter}`: search the account. Answers `{matches:
/// [{id, in_vault}], truncated, headers}`, `headers` being the first
/// page's. Supersedes any preview request still running, which then
/// answers `cancelled`.
pub fn email_search(app: &mut App, req: Req, request: SearchRequest) {
    let account = request.account;
    let client = match app.google.connected(account) {
        Ok(client) => client,
        Err(e) => return google::refuse_account(req, e),
    };
    let query = request.filter.to_query();
    let op = Id::generate().to_string();
    log::debug!("Email search {op}: searching {account} for `{query}`");
    let work = move |cancel: &Cancel| {
        let (ids, truncated) = gmail::list_message_ids(&client, cancel, &query, MATCH_CAP)?;
        let first = &ids[..ids.len().min(PAGE_SIZE)];
        let headers = gmail::get_headers(&client, cancel, first)?;
        Ok((ids, truncated, headers))
    };
    let latest = op.clone();
    app.run_superseding(PREVIEW, &latest, work, move |app, result: Result<_>| {
        let (ids, truncated, headers) = match result {
            Ok(found) => found,
            Err(e) => return fail(req, e, Text::new("email.search_failed")),
        };
        let known = KnownEmails::of(app, account);
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
        log::info!("Email search {op}: {} matches", ids.len());
        req.ok(matches);
    });
}

// ----------------------------------------------------------------------------
/// `email_headers {account, ids}`: the preview headers of a page of emails,
/// `{headers}`. Supersedes any preview request still running.
pub fn email_headers(app: &mut App, req: Req, request: HeadersRequest) {
    let account = request.account;
    let client = match app.google.connected(account) {
        Ok(client) => client,
        Err(e) => return google::refuse_account(req, e),
    };
    let ids: Vec<String> = request.ids.into_iter().take(PAGE_SIZE).collect();
    let work = move |cancel: &Cancel| gmail::get_headers(&client, cancel, &ids);
    let op = Id::generate().to_string();
    app.run_superseding(PREVIEW, &op, work, move |app, result| {
        let headers = match result {
            Ok(headers) => headers,
            Err(e) => return fail(req, e, Text::new("email.page_failed")),
        };
        let known = KnownEmails::of(app, account);
        let headers: Vec<HeaderInfo> = headers.into_iter().map(|h| known.header(h)).collect();
        req.ok(json!({ "headers": headers }));
    });
}
