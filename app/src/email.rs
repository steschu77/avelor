//! The email import from Gmail (email.md, `email_*`): the Fetch-email page's
//! labels, live query, match preview and paging, and the fetch job that
//! downloads, converts, and writes the selected emails into `Email/`.
//!
//! A fetch goes plan → run → apply (email.md#design): the plan decides from the
//! vault what to download, workers download and parse the emails, and each one
//! is applied on the UI thread through the vault edit.
//!
//! Every email written carries `remote: {account, remote: [{message-id},
//! {gmail-id}]}` (vault.md#remote), which is how a later preview tells the
//! emails already in the vault from the new ones.
//!
//! An account's auto-sync fetches the new email that matches its own filter
//! every 15 minutes, started by the clock (`App::on_clock`): a quiet search
//! first, then -- only if there is new email -- a fetch job like the page's.

use av_utils::id::Id;

use crate::google::AutoSyncs;
use fetch::EmailFetch;

mod apply;
mod auto_sync;
mod entities;
mod fetch;
mod preview;

pub(crate) use auto_sync::{SyncChange, auto_sync, set_email_sync, vault_opened};
pub(crate) use fetch::{FetchRequest, email_fetch, stop};
pub(crate) use preview::{
    HeadersRequest, SearchRequest, email_headers, email_labels, email_search,
};
/// The most matches a search lists.
const MATCH_CAP: usize = 2000;

// ----------------------------------------------------------------------------
/// What the app keeps for the email import: the fetch in progress (one at
/// a time), and the auto-sync's schedule.
#[derive(Default)]
pub struct EmailState {
    fetch: Option<EmailFetch>,
    /// The accounts' auto-syncs.
    auto: AutoSyncs<Id>,
}

// ----------------------------------------------------------------------------
/// Why a fetch runs: the page's Fetch, or an account's auto-sync.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Origin {
    Manual,
    AutoSync,
}

#[cfg(test)]
mod tests;
