//! Google accounts' commands (api.md#google): connect one through the browser
//! sign-in, or cancel that; list its calendars and check them as sources;
//! disconnect it (`accounts::disconnect_account`). The accounts themselves:
//! `google_accounts`.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;
use std::path::PathBuf;

use av_google::auth::{self, PendingConsent};
use av_google::calendar;
use av_google::client::Client;
use av_google::identity::{self, UserInfo};
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use serde::Serialize;

use crate::app::App;
use crate::google_accounts::{self, Account, CalendarSource, Level};
use crate::protocol::Req;
use crate::sys;

// ----------------------------------------------------------------------------
/// How often an auto-sync runs (an account's mail, a calendar), in seconds.
pub const SYNC_EVERY: i64 = 15 * 60;

// ----------------------------------------------------------------------------
/// Send `account` to the page: the `account` event.
pub fn post_account(app: &App, account: &Account) {
    app.post_event("account", account.meta());
}

// ----------------------------------------------------------------------------
/// Send the account `id` to the page, if there is one.
pub fn post(app: &App, id: Id) {
    if let Ok(account) = app.google.get(id) {
        post_account(app, account);
    }
}

// ----------------------------------------------------------------------------
/// Answer a request whose account has no client (`GoogleAccounts::connected`
/// failed): `not_found`, or `not_connected`.
pub fn refuse_account(req: Req, error: Error) {
    match error {
        Error::NotConnected => req.fail(error, "The account isn't connected."),
        error => req.err(error),
    }
}

// ----------------------------------------------------------------------------
/// A sync of the account `id` ended (a mail fetch): its
/// `last_sync` is now, `record` notes what else it did (a mail fetch's
/// `last_fetch`), the account file is saved, and the page gets the account.
/// A failed save is logged; what the sync wrote stays in the vault.
pub fn finish_sync(app: &mut App, id: Id, record: impl FnOnce(&mut Account)) {
    let updated = app.google.update(id, |account| {
        account.file.last_sync = Some(DateTime::now());
        record(account);
        true
    });
    match updated {
        Ok(_) => post(app, id),
        Err(e) => log::warn!("Saving the last sync of {id} failed: {e}"),
    }
}

// ----------------------------------------------------------------------------
pub fn later(at: DateTime, seconds: i64) -> DateTime {
    DateTime::from_unix_millis((at.as_unix() + seconds) * 1000)
}

// ----------------------------------------------------------------------------
/// The first of `keys` whose auto-sync is due at `now`, by `next` (when
/// each may run next; none: at once).
pub fn first_due<K: Eq + Hash>(
    keys: impl IntoIterator<Item = K>,
    next: &HashMap<K, DateTime>,
    now: DateTime,
) -> Option<K> {
    keys.into_iter()
        .find(|key| next.get(key).is_none_or(|at| *at <= now))
}

// ----------------------------------------------------------------------------
/// `connect_google {account?, levels}`: connect a new Google account
/// (`account` absent) or reconnect one, asking for `levels` (`{"email":
/// "read", …}`). Opens the browser for the sign-in; `cancel_connect`,
/// another connect or a vault switch cancels it. Answers the account's id,
/// after the `account` event.
pub fn connect_google(
    app: &mut App,
    req: Req,
    target: Option<Id>,
    levels: BTreeMap<String, Level>,
) {
    let scope = google_accounts::scope_from_levels(&levels);
    if app.vault.root().is_none() {
        return req.err(Error::InvalidPath {
            path: PathBuf::new(),
        }); // no vault to keep the account in
    }
    // A reconnect preselects the account's email in the browser.
    let login_hint = match target.map(|id| app.google.get(id)) {
        None => None,
        Some(Ok(account)) => account.email().map(str::to_string),
        Some(Err(e)) => return req.err(e),
    };
    if !auth::has_client() {
        let what = "signing in to Google without an OAuth client".to_string();
        let message =
            "This build can\u{2019}t sign in to Google: it was built without an OAuth client.";
        return req.fail(Error::Unsupported { what }, message);
    }
    let pending = match auth::begin_consent(&scope, login_hint.as_deref()) {
        Ok(pending) => pending,
        Err(e) => return req.err(e),
    };

    let connect = Id::generate();
    match target {
        Some(id) => log::info!("Connect {connect}: reconnecting {id}"),
        None => log::info!("Connect {connect}: connecting a new account"),
    }
    app.begin_connect(connect, pending.cancel_handle());
    // The shell may run a modal loop: the browser opens after the handler.
    let url = pending.url.clone();
    app.on_ui(
        move |_| sys::open_url(&url),
        move |app, opened| {
            if let Err(e) = opened {
                log::warn!("Connect {connect}: could not open the browser: {e}");
                app.end_connect(connect);
                return req.err(e);
            }
            // The wait ends on the connect's own `Cancel` (`begin_connect`),
            // not the task's.
            app.run(
                move |_| {
                    let code = pending.wait_code();
                    (pending, code)
                },
                move |app, (pending, code)| {
                    on_consent_code(app, req, connect, target, pending, code)
                },
            );
        },
    );
}

// ----------------------------------------------------------------------------
/// The browser sign-in returned (or was cancelled): redeem the code and
/// ask who signed in.
fn on_consent_code(
    app: &mut App,
    req: Req,
    connect: Id,
    target: Option<Id>,
    pending: PendingConsent,
    code: Result<Option<String>>,
) {
    if !app.is_connect_pending(connect) {
        log::info!("Connect {connect}: sign-in ended, but it was cancelled or replaced");
        return req.err(Error::Cancelled);
    }
    let code = match code {
        Ok(Some(code)) => code,
        Ok(None) => {
            app.end_connect(connect);
            return req.err(Error::Cancelled);
        }
        Err(e) => {
            app.end_connect(connect);
            return req.err(e);
        }
    };
    log::info!("Connect {connect}: got an auth code, redeeming it");
    // Under the connect's `Cancel`, so cancelling the connect stops these
    // requests too.
    app.run(
        move |_| {
            let cancel = pending.cancel_handle();
            let client = pending.redeem(&cancel, &code)?;
            let user = identity::get_user_info(&client, &cancel)?;
            Ok((client, user))
        },
        move |app, result| on_consent_redeemed(app, req, connect, target, result),
    );
}

// ----------------------------------------------------------------------------
/// Store the connected account (`GoogleAccounts::connect`): a reconnect, or
/// a new connect of an email that is already configured, updates that
/// account (same id); otherwise a new account file is created.
fn on_consent_redeemed(
    app: &mut App,
    req: Req,
    connect: Id,
    target: Option<Id>,
    result: Result<(Client, UserInfo)>,
) {
    if !app.is_connect_pending(connect) {
        log::info!("Connect {connect}: redeemed, but it was cancelled or replaced");
        return req.err(Error::Cancelled);
    }
    app.end_connect(connect);
    let (client, user) = match result {
        Ok(connected) => connected,
        Err(e) => return req.err(e),
    };
    log::info!(
        "Connect {connect}: signed in as {}, granted {}",
        user.email,
        google_accounts::describe_scope(client.scope())
    );
    match app.google.connect(target, &user.email, client) {
        Ok(id) => {
            post(app, id);
            req.ok(Connected { id });
        }
        Err(e) => {
            log::warn!("Connect {connect}: could not store the account: {e}");
            req.err(e);
        }
    }
}

/// `connect_google`'s answer.
#[derive(Serialize)]
struct Connected {
    id: Id,
}

// ----------------------------------------------------------------------------
/// `cancel_connect`: abandon the pending sign-in, which then answers
/// `cancelled`.
pub fn cancel_connect(app: &mut App, req: Req) {
    app.cancel_connect();
    req.ok(());
}

// ----------------------------------------------------------------------------
/// One calendar of `list_calendars`' answer.
#[derive(Serialize)]
struct CalendarInfo<'a> {
    id: &'a str,
    name: &'a str,
    color: Option<&'a str>,
    primary: bool,
}

/// `list_calendars`' answer.
#[derive(Serialize)]
struct Calendars<'a> {
    calendars: Vec<CalendarInfo<'a>>,
}

// ----------------------------------------------------------------------------
/// `list_calendars {account}`: the calendars of the connected account, the
/// primary calendar first, then by name. Checked calendars take their
/// current names and colors along the way (the `account` event).
pub fn list_calendars(app: &mut App, req: Req, id: Id) {
    let client = match app.google.connected(id) {
        Ok(client) => client,
        Err(e) => return req.err(e),
    };
    log::info!("Listing the calendars of {id}");
    app.run(
        move |cancel| calendar::list_calendars(&client, cancel),
        move |app, result| {
            let mut list = match result {
                Ok(list) => list,
                Err(e) => return req.err(e),
            };
            match app.google.update(id, |a| a.refresh_calendars(&list)) {
                Ok(true) => post(app, id),
                Ok(false) => {}
                Err(Error::NotFound) => return req.err(Error::NotFound),
                // Only display names changed: a failed save isn't worth an error.
                Err(e) => log::warn!("Saving the renamed calendars of {id} failed: {e}"),
            }

            list.sort_by_cached_key(|c| (!c.primary, c.summary.to_lowercase()));
            let calendars = list
                .iter()
                .map(|c| CalendarInfo {
                    id: &c.id,
                    name: &c.summary,
                    color: Some(c.background_color.as_str()).filter(|c| !c.is_empty()),
                    primary: c.primary,
                })
                .collect();
            log::info!("{} calendars of {id}", list.len());
            req.ok(Calendars { calendars });
        },
    );
}

// ----------------------------------------------------------------------------
/// `set_calendar_source {account, calendar, selected}`: check or uncheck a
/// calendar (`{id, name, color}` of `list_calendars`) as a source. The
/// account follows as the `account` event, also when nothing changed.
pub fn set_calendar_source(
    app: &mut App,
    req: Req,
    id: Id,
    calendar: CalendarSource,
    selected: bool,
) {
    log::info!(
        "{} calendar {} ({}) of {id}",
        if selected { "Checking" } else { "Unchecking" },
        calendar.id,
        calendar.name
    );
    let set = app
        .google
        .update(id, |a| a.set_calendar(calendar, selected));
    if set.is_ok() {
        post(app, id);
    }
    req.answer(set.map(|_| ()));
}

// ----------------------------------------------------------------------------
/// The Google half of `disconnect_account`: revoke the token at Google
/// (best effort), then forget it but keep the account, marked
/// `disconnected`, so connecting the same Google account again keeps its
/// id. Answered after the `account` event.
pub fn disconnect(app: &mut App, req: Req, id: Id) {
    let account = match app.google.get(id) {
        Ok(account) => account,
        Err(e) => return req.err(e),
    };
    log::info!(
        "Disconnecting {id} ({})",
        account.email().unwrap_or("no email")
    );
    let token = account
        .client
        .as_ref()
        .map(|c| c.refresh_token().to_string());
    app.run(
        move |cancel| revoke(cancel, id, token),
        move |app, ()| {
            let disconnected = app.google.disconnect(id);
            if disconnected.is_ok() {
                post(app, id);
            }
            req.answer(disconnected);
        },
    );
}

// ----------------------------------------------------------------------------
/// Revoke `token` at Google, if there is one; failures are only logged
/// (the token file is deleted either way).
fn revoke(cancel: &Cancel, id: Id, token: Option<String>) {
    let Some(token) = token else {
        return;
    };
    match auth::revoke(cancel, &token) {
        Ok(()) => log::info!("Revoked the token of account {id}"),
        Err(e) => log::warn!("Revoking the token of account {id} failed: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn the_first_due_is_one_never_run_or_whose_time_has_come() {
        let (a, b) = (Id::generate(), Id::generate());
        let now = DateTime::from_unix_millis(1_790_000_000_000);
        let mut next = HashMap::new();
        assert_eq!(first_due([a, b], &next, now), Some(a), "never run: due");
        next.insert(a, later(now, SYNC_EVERY));
        assert_eq!(first_due([a, b], &next, now), Some(b));
        next.insert(b, now);
        assert_eq!(first_due([a, b], &next, now), Some(b), "due at its time");
        next.insert(b, later(now, 1));
        assert_eq!(first_due([a, b], &next, now), None);
        assert_eq!(later(now, 60).as_unix(), now.as_unix() + 60);
    }
}
