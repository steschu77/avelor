//! The email auto-sync: a quiet search on the clock, then a fetch of what is
//! new.

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use av_google::client::Client;
use av_google::gmail::{self, EmailFilter};
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use serde::Deserialize;

use super::fetch::{FetchPlan, find_account_email, plan_fetch, pump, start_fetch};
use super::{MATCH_CAP, Origin};
use crate::app::App;
use crate::google;
use crate::google::accounts::AutoSync;
use crate::google::accounts::FetchEmail;
use crate::protocol::Req;
use crate::texts::Text;
use av_vault::schema::remote::GMAIL_ID;

// ----------------------------------------------------------------------------
/// `set_email_sync`'s args: the fields that change.
#[derive(Deserialize)]
pub(crate) struct SyncChange {
    pub(super) account: Id,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    pub(super) filter: Option<EmailFilter>,
}

// ----------------------------------------------------------------------------
/// `set_email_sync {account, enabled?, filter?}`: change the
/// account's auto-sync. Its filter drops the dates (the host sets them);
/// switching it on starts it from now -- only email that arrives from then on
/// is fetched -- and switching it off keeps the filter. Answered after the
/// `account` event.
pub fn set_email_sync(app: &mut App, req: Req, change: SyncChange) {
    let now = DateTime::now();
    let updated = app.google.update(change.account, |account| {
        let email = account
            .file
            .google
            .fetch_email
            .get_or_insert_with(FetchEmail::default);
        let sync = email.auto_sync.get_or_insert_with(|| AutoSync::new(now));
        if let Some(filter) = change.filter {
            sync.filter = without_dates(filter);
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
        Err(e) => req.save_failed(e, Text::new("save.account")),
    }
}

// ----------------------------------------------------------------------------
/// `filter` without its date conditions.
fn without_dates(mut filter: EmailFilter) -> EmailFilter {
    filter.since = None;
    filter.after = None;
    filter.before = None;
    filter
}

// ----------------------------------------------------------------------------
/// The Gmail query of an auto-sync: `filter`, for the email that arrived
/// since `since`.
pub(super) fn sync_query(filter: &EmailFilter, since: DateTime) -> String {
    let mut filter = without_dates(filter.clone());
    filter.after = Some(since);
    filter.to_query()
}

// ----------------------------------------------------------------------------
/// A vault opened: its accounts' auto-syncs are due as soon as its indexing
/// has ended.
pub(crate) fn vault_opened(app: &mut App) {
    app.email.auto.clear();
}

// ----------------------------------------------------------------------------
/// The clock ticked, or the indexing ended (`App::on_clock`): start
/// the auto-sync of the first account that is due -- one with auto-sync on
/// and a connection, whose last run is 15 minutes ago -- unless the indexing, a
/// fetch or another auto-sync still runs. One run at a time; the next tick
/// takes the next account.
pub fn auto_sync(app: &mut App, now: DateTime) {
    let email = &app.email;
    if app.indexing() || email.fetch.is_some() || email.auto.is_listing() {
        return;
    }
    let candidates: Vec<(Id, Arc<Client>, EmailFilter, DateTime)> = app
        .google
        .iter()
        .filter_map(|account| {
            let sync = account
                .file
                .google
                .fetch_email
                .as_ref()?
                .auto_sync
                .as_ref()?;
            let client = account.client.clone()?;
            sync.enabled
                .then(|| (account.id(), client, sync.filter.clone(), sync.since))
        })
        .collect();
    let ids = candidates.iter().map(|(id, ..)| *id);
    let Some(account) = email.auto.first_due(ids, now) else {
        return;
    };
    let Some((_, client, filter, since)) = candidates.into_iter().find(|(id, ..)| *id == account)
    else {
        return;
    };

    app.email.auto.listing(account, now);
    let root = app.vault.root().map(Path::to_path_buf);
    let query = sync_query(&filter, since);
    let started = DateTime::now();
    log::info!("Email auto-sync of {account}: searching `{query}`");
    app.run(
        move |cancel| gmail::list_message_ids(&client, cancel, &query, MATCH_CAP),
        move |app, result| {
            let ids = result.map(|(ids, _truncated)| ids);
            on_sync_listed(app, account, root, started, ids);
        },
    );
}

// ----------------------------------------------------------------------------
/// An auto-sync's search answered: fetch the new emails as a job -- or do
/// nothing, if there are none (no job, no write). Dropped if another vault
/// opened or auto-sync was switched off meanwhile.
pub(super) fn on_sync_listed(
    app: &mut App,
    account: Id,
    root: Option<PathBuf>,
    started: DateTime,
    result: Result<Vec<String>>,
) {
    app.email.auto.listed();
    let sync = app
        .google
        .get(account)
        .ok()
        .and_then(|a| a.file.google.fetch_email.as_ref()?.auto_sync.as_ref())
        .filter(|sync| sync.enabled);
    if app.vault.root() != root.as_deref() || sync.is_none() {
        return log::info!("Email auto-sync of {account}: dropped, the vault or setting changed");
    }
    let ids = match result {
        Ok(ids) => {
            app.email.auto.worked(&account);
            ids
        }
        Err(e) => {
            log::warn!("Email auto-sync of {account} failed: {e}");
            // Shown once, not every 15 minutes while offline.
            if app.email.auto.failed(account) {
                let email = find_account_email(app, account);
                app.notice(Text::new("email.auto_sync_failed").arg("email", email));
            }
            return;
        }
    };
    let known = app.vault.index().remote_values(Some(&account), GMAIL_ID);
    let (queue, _) = plan_fetch(ids, &HashSet::new(), &known);
    if queue.is_empty() {
        return log::info!("Email auto-sync of {account}: nothing new");
    }
    if app.email.fetch.is_some() {
        // A fetch started meanwhile: again at the next tick.
        app.email.auto.again(account);
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
        analysis: None,
    };
    start_fetch(app, Text::new("email.syncing").arg("email", email), plan);
    pump(app);
}
