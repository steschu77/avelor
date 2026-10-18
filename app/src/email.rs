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

use fetch::EmailFetch;

mod apply;
mod entities;
mod fetch;
mod preview;

pub(crate) use fetch::{FetchRequest, email_fetch, stop};
pub(crate) use preview::{
    HeadersRequest, SearchRequest, email_headers, email_labels, email_search,
};
/// The most matches a search lists.
const MATCH_CAP: usize = 2000;

// ----------------------------------------------------------------------------
/// What the app keeps for the email import: the fetch in progress (one at
/// a time).
#[derive(Default)]
pub struct EmailState {
    fetch: Option<EmailFetch>,
}

#[cfg(test)]
mod tests;
