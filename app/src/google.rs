//! Google accounts' commands (api.md#google): connect one through the browser
//! sign-in, or cancel that; list its calendars and check them as sources;
//! disconnect it (`accounts::disconnect_account`). The accounts themselves:
//! `google::accounts`.

use std::collections::BTreeMap;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::PathBuf;

use av_google::auth::{self, PendingConsent};
use av_google::calendar::{self, CalendarListEntry};
use av_google::client::Client;
use av_google::identity::{self, UserInfo};
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use serde::Serialize;

use crate::app::App;
use crate::google::accounts::{Account, CalendarSource, Level};
use crate::protocol::Req;
use crate::sys;
use crate::texts::{self, Text};

pub(crate) mod accounts;

// ----------------------------------------------------------------------------
/// How often an auto-sync runs (an account's email, a calendar), in seconds.
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
/// Send the account `id` to the page, and its calendar entities, whose
/// access follows its sign-in, its access and its checked calendars
/// (`calendar::access`).
pub fn post_with_calendars(app: &App, id: Id) {
    post(app, id);
    crate::calendar::post_calendars(app, id);
}

// ----------------------------------------------------------------------------
/// Answer a request whose account has no client (`GoogleAccounts::connected`
/// failed): `not_found`, or `not_connected`.
pub fn refuse_account(req: Req, error: Error) {
    match error {
        Error::NotConnected => req.fail(error, Text::new("google.not_connected")),
        error => req.err(error),
    }
}

// ----------------------------------------------------------------------------
/// A sync of the account `id` ended (an email fetch, a calendar sync): its
/// `last_sync` is now, `record` notes what else it did (an email fetch's
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
/// The auto-syncs of one kind (an account's email, a calendar), by key: when
/// each may run next, the one whose look at Google is in flight, and those
/// whose last run failed, which said so once.
pub struct AutoSyncs<K> {
    /// When each may run next; none: at the next chance. Cleared when a
    /// vault opens.
    pub(crate) next: HashMap<K, DateTime>,
    /// The one whose list or search is in flight.
    pub(crate) listing: Option<K>,
    /// Those whose last auto-sync failed: only the first failure is shown.
    pub(crate) failing: HashSet<K>,
}

impl<K> Default for AutoSyncs<K> {
    fn default() -> Self {
        Self {
            next: HashMap::new(),
            listing: None,
            failing: HashSet::new(),
        }
    }
}

impl<K: Eq + Hash + Clone> AutoSyncs<K> {
    // ------------------------------------------------------------------------
    /// The first of `keys` whose auto-sync is due at `now`: never run, or
    /// its time has come.
    pub fn first_due(&self, keys: impl IntoIterator<Item = K>, now: DateTime) -> Option<K> {
        keys.into_iter()
            .find(|key| self.next.get(key).is_none_or(|at| *at <= now))
    }

    // ------------------------------------------------------------------------
    pub fn is_listing(&self) -> bool {
        self.listing.is_some()
    }

    // ------------------------------------------------------------------------
    /// The auto-sync of `key` looks at Google now; it runs next
    /// `SYNC_EVERY` later.
    pub fn listing(&mut self, key: K, now: DateTime) {
        self.ran(key.clone(), now);
        self.listing = Some(key);
    }

    // ------------------------------------------------------------------------
    /// The look at Google answered.
    pub fn listed(&mut self) {
        self.listing = None;
    }

    // ------------------------------------------------------------------------
    /// `key` was synced at `at`: it runs next `SYNC_EVERY` later.
    pub fn ran(&mut self, key: K, at: DateTime) {
        self.next.insert(key, at.plus_seconds(SYNC_EVERY));
    }

    // ------------------------------------------------------------------------
    /// `key` runs again at the next tick.
    pub fn again(&mut self, key: K) {
        self.next.insert(key, DateTime::now());
    }

    // ------------------------------------------------------------------------
    /// `key` is due at once.
    pub fn due(&mut self, key: &K) {
        self.next.remove(key);
    }

    // ------------------------------------------------------------------------
    /// The auto-sync of `key` failed; whether that is new, to be said.
    pub fn failed(&mut self, key: K) -> bool {
        self.failing.insert(key)
    }

    // ------------------------------------------------------------------------
    pub fn worked(&mut self, key: &K) {
        self.failing.remove(key);
    }

    // ------------------------------------------------------------------------
    pub fn is_failing(&self, key: &K) -> bool {
        self.failing.contains(key)
    }

    // ------------------------------------------------------------------------
    /// A vault opened: every auto-sync is due, and no failure said.
    pub fn clear(&mut self) {
        self.next.clear();
        self.failing.clear();
    }
}

// ----------------------------------------------------------------------------
/// What the browser's tab shows at the end of a sign-in, in the run's
/// language.
fn sign_in_pages() -> auth::SignInPages {
    let page = |heading: &str, line: &str| (heading.to_string(), line.to_string());
    auth::SignInPages {
        connected: page(
            texts::text("google.page.connected"),
            texts::text("google.page.connected_line"),
        ),
        failed: page(
            texts::text("google.page.failed"),
            texts::text("google.page.failed_line"),
        ),
        outdated: page(
            texts::text("google.page.outdated"),
            texts::text("google.page.outdated_line"),
        ),
    }
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
    let scope = accounts::scope_from_levels(&levels);
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
        let text = Text::new("google.no_client");
        return req.fail(Error::Unsupported { what }, text);
    }
    let pending = match auth::begin_consent(&scope, login_hint.as_deref(), sign_in_pages()) {
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
        accounts::describe_scope(client.scope())
    );
    match app.google.connect(target, &user.email, client) {
        Ok(id) => {
            post_with_calendars(app, id);
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
    /// Google's access for the account (`CalendarSource::access`).
    access: Option<&'a str>,
}

/// `list_calendars`' answer.
#[derive(Serialize)]
struct Calendars<'a> {
    calendars: Vec<CalendarInfo<'a>>,
}

// ----------------------------------------------------------------------------
/// `list_calendars {account}`: the calendars of the connected account, the
/// primary calendar first, then by name. Checked calendars take their
/// current names, colors and access along the way (the `account` event), and
/// get the vault's entities they lack.
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
            if let Err(e) = take_calendars(app, id, &list) {
                return req.err(e);
            }

            list.sort_by_cached_key(|c| (!c.primary, c.summary.to_lowercase()));
            let calendars = list
                .iter()
                .map(|c| CalendarInfo {
                    id: &c.id,
                    name: &c.summary,
                    color: Some(c.background_color.as_str()).filter(|c| !c.is_empty()),
                    primary: c.primary,
                    access: Some(c.access_role.as_str()).filter(|a| !a.is_empty()),
                })
                .collect();
            log::info!("{} calendars of {id}", list.len());
            req.ok(Calendars { calendars });
        },
    );
}

// ----------------------------------------------------------------------------
/// Take the account's calendars as Google lists them: its checked ones their
/// names, colors and access, and the vault's entities they lack. `not_found`
/// for an account gone since.
fn take_calendars(app: &mut App, id: Id, list: &[CalendarListEntry]) -> Result<()> {
    let refreshed = match app.google.update(id, |a| a.refresh_calendars(list)) {
        Ok(refreshed) => refreshed,
        Err(Error::NotFound) => return Err(Error::NotFound),
        // Only display names changed: a failed save isn't worth an error.
        Err(e) => {
            log::warn!("Saving the renamed calendars of {id} failed: {e}");
            false
        }
    };
    crate::calendar::ensure_entities(app, id);
    if refreshed {
        post_with_calendars(app, id);
    }
    Ok(())
}

// ----------------------------------------------------------------------------
/// `set_calendar_source {account, calendar, selected}`: check or uncheck a
/// calendar (`{id, name, color, access}` of `list_calendars`) as a source,
/// one checked getting its entity in the vault (the `entity` event). The
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
        if selected {
            crate::calendar::ensure_entities(app, id);
        }
        post_with_calendars(app, id);
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
                post_with_calendars(app, id);
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
    use crate::testing::{CONNECTED, parse, shape, with_google};

    // ------------------------------------------------------------------------
    #[test]
    fn the_calendars_listed_make_the_checked_ones_entities() {
        let checked =
            "  scopes:\n    - calendar: full\n  calendars:\n  - id: cal1\n    name: Personal\n";
        let mut h = with_google("calendars-listed", checked, &[]);
        let account = Id::parse(CONNECTED).unwrap();
        let entry = |id: &str, summary: &str| CalendarListEntry {
            id: id.into(),
            summary: summary.into(),
            ..CalendarListEntry::default()
        };
        let list = [entry("cal1", "Family"), entry("cal2", "Not checked")];
        take_calendars(&mut h.app, account, &list).unwrap();
        let sent = parse(&h.take());
        assert_eq!(shape(&sent), ["entity", "account", "entity"], "{sent:?}");
        assert_eq!(sent[0]["data"]["title"], "Family", "by its current name");

        take_calendars(&mut h.app, account, &list).unwrap();
        assert!(h.take().is_empty(), "nothing new");
        let gone = take_calendars(&mut h.app, Id::generate(), &list);
        assert!(matches!(gone, Err(Error::NotFound)));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_first_due_is_one_never_run_or_whose_time_has_come() {
        let (a, b) = (Id::generate(), Id::generate());
        let now = DateTime::from_unix_millis(1_790_000_000_000);
        let mut syncs = AutoSyncs::default();
        assert_eq!(syncs.first_due([a, b], now), Some(a), "never run: due");
        syncs.listing(a, now);
        assert!(syncs.is_listing());
        assert_eq!(syncs.next[&a], now.plus_seconds(SYNC_EVERY));
        assert_eq!(syncs.first_due([a, b], now), Some(b));
        syncs.next.insert(b, now);
        assert_eq!(syncs.first_due([a, b], now), Some(b), "due at its time");
        syncs.next.insert(b, now.plus_seconds(1));
        assert_eq!(syncs.first_due([a, b], now), None);
        syncs.due(&b);
        assert_eq!(syncs.first_due([a, b], now), Some(b), "made due");
        syncs.listed();
        assert!(!syncs.is_listing());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_failure_is_said_once_until_it_works_again() {
        let a = Id::generate();
        let mut syncs = AutoSyncs::default();
        assert!(syncs.failed(a), "said");
        assert!(!syncs.failed(a), "not again");
        assert!(syncs.is_failing(&a));
        syncs.worked(&a);
        assert!(syncs.failed(a), "said again");
        syncs.clear();
        assert!(!syncs.is_failing(&a));
    }
}
