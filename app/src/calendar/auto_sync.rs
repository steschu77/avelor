//! A calendar's auto-sync: a quiet list of Google's changes, then a sync when
//! there is something to do.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use av_google::auth::ScopeAccess;
use av_google::calendar_sync;
use av_google::calendar_sync::Changes;
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::document::Document;
use av_vault::envelope::Source;

use super::plan::{
    build_job, calendar_entity, calendar_job, find_calendar, first_sync_start, local_event,
};
use super::{SourceKey, find_source, start};
use crate::app::App;
use crate::google::SYNC_EVERY;
use crate::texts::Text;
use av_vault::schema::remote::CALENDAR_ID;

/// How long after it last looked at Google a calendar syncs when the
/// window is active again, in seconds.
const ACTIVATED_AFTER: i64 = 2 * 60;

// ----------------------------------------------------------------------------
/// The calendars whose auto-sync may run, in the accounts' order: switched
/// on, of a connected account with calendar access.
pub(super) fn auto_sync_sources(app: &App) -> Vec<SourceKey> {
    app.google
        .iter()
        .filter(|account| {
            let client = account.client.as_ref();
            client.is_some_and(|c| c.scope().calendar != ScopeAccess::None)
        })
        .flat_map(|account| {
            let sources = account.file.google.calendars.iter();
            let on = sources.filter(|source| source.auto_sync);
            on.map(|source| (account.id(), source.id.clone()))
        })
        .collect()
}

// ----------------------------------------------------------------------------
/// The clock ticked, or the indexing ended (`App::on_clock`): list
/// the changes of the first calendar that is due -- one with auto-sync on,
/// of a connected account with calendar access, last synced 15 minutes ago
/// or made due sooner, none of whose events is in edit mode -- unless the
/// indexing, a sync or another auto-sync's list still runs. One at a time; the
/// next tick takes the next calendar. The list writes nothing;
/// `on_auto_listed` decides on a job.
pub fn auto_sync(app: &mut App, now: DateTime) {
    let state = &app.calendar;
    if app.indexing() || state.sync.is_some() || state.auto.is_listing() {
        return;
    }
    let editing = editing_sources(app);
    let sources = auto_sync_sources(app);
    let sources = sources.into_iter().filter(|key| !editing.contains(key));
    let Some(key) = state.auto.first_due(sources, now) else {
        return;
    };
    let (account, calendar_id) = key.clone();
    let client = app.google.connected(account);
    let source = find_source(app, account, &calendar_id);
    let (Ok(client), Some(source)) = (client, source) else {
        return;
    };

    app.calendar.auto.listing(key.clone(), now);
    let calendar = find_calendar(app, account, &calendar_id);
    let cal = calendar_job(app, account, calendar, &source);
    let time_min = first_sync_start(now).as_iso8601();
    let root = app.vault.root().map(Path::to_path_buf);
    log::info!("Calendar auto-sync of {calendar_id} ({account}): listing");
    app.run(
        move |cancel| calendar_sync::list_changes(&client, cancel, &cal, &time_min),
        move |app, result| on_auto_listed(app, key, root, result),
    );
}

// ----------------------------------------------------------------------------
/// An auto-sync's list answered: sync the calendar as a job with what was
/// listed -- or do nothing, if that changes nothing and nothing waits to be
/// pushed (no job, no write). Dropped if another vault opened or the
/// calendar's auto-sync was switched off meanwhile.
pub(super) fn on_auto_listed(
    app: &mut App,
    key: SourceKey,
    root: Option<PathBuf>,
    result: Result<Changes>,
) {
    app.calendar.auto.listed();
    let (account, calendar_id) = key.clone();
    let source = find_source(app, account, &calendar_id);
    let on = auto_sync_sources(app).contains(&key);
    let (Some(source), true) = (source, on && app.vault.root() == root.as_deref()) else {
        return log::info!(
            "Calendar auto-sync of {calendar_id}: dropped, the vault or setting changed"
        );
    };
    let changes = match result {
        Ok(changes) => changes,
        Err(e) => return auto_sync_failed(app, key, &source.name, &e),
    };
    if app.calendar.sync.is_some() || editing_sources(app).contains(&key) {
        // A sync started meanwhile, or one of its events is being edited:
        // again at the next tick, or once the edit ends.
        app.calendar.auto.again(key);
        return;
    }
    let Ok(client) = app.google.connected(account) else {
        return;
    };
    // Without a calendar entity there was no sync token: the full list is
    // work, and the entity is made for it.
    let calendar = match calendar_entity(app, account, &source) {
        Ok(calendar) => calendar,
        Err(e) => return auto_sync_failed(app, key, &source.name, &e),
    };
    let push = client.scope().calendar == ScopeAccess::Full;
    let built = build_job(app, account, calendar, &source, push, Some(changes));
    if !built.0.has_work() {
        app.calendar.auto.worked(&key);
        return log::info!("Calendar auto-sync of {calendar_id}: nothing new");
    }
    start(app, client, account, calendar, &source, built);
}

// ----------------------------------------------------------------------------
/// An auto-sync of `key` failed before its job: said once, not every 15
/// minutes while offline.
fn auto_sync_failed(app: &mut App, key: SourceKey, name: &str, error: &Error) {
    log::warn!("Calendar auto-sync of {name} failed: {error}");
    if app.calendar.auto.failed(key) {
        app.notice(Text::new("calendar.auto_sync_failed").arg("name", name));
    }
}

// ----------------------------------------------------------------------------
/// Entities the vault now holds differently: written by the app
/// (`App::edit`), saved from an editor, taken in from outside, or
/// leaving edit mode (`documents::set_editing`). An event that waits to be
/// pushed makes its calendar's auto-sync due -- unless it is in edit mode,
/// whose end is the sync point. Only for a calendar that syncs on its own,
/// two-way.
pub(crate) fn changed<'a>(app: &mut App, ids: impl IntoIterator<Item = &'a Id>) {
    for id in ids {
        if app.documents.is_editing(id) {
            continue;
        }
        if let Some(key) = waiting_push(app, id) {
            make_due(app, key, &format!("{id} waits to be pushed"));
        }
    }
}

// ----------------------------------------------------------------------------
/// The event `id` was deleted, and its calendar would delete it in Google
/// (`waiting_delete`, asked before): its auto-sync is due.
pub(crate) fn deleted(app: &mut App, id: &Id, key: SourceKey) {
    make_due(app, key, &format!("{id} was deleted"));
}

// ----------------------------------------------------------------------------
/// The auto-sync of `key` is due now, for the reason `why`.
fn make_due(app: &mut App, key: SourceKey, why: &str) {
    log::info!("Calendar auto-sync of {}: due, {why}", key.1);
    app.calendar.auto.due(&key);
}

// ----------------------------------------------------------------------------
/// The calendar of the event `id`, if its auto-sync is on and two-way and
/// would push the event: one never synced, or changed since.
pub(super) fn waiting_push(app: &App, id: &Id) -> Option<SourceKey> {
    let entry = app.vault.index().entry(id)?;
    let key = two_way_auto_sync(app, &entry.doc)?;
    let local = local_event(&entry.doc, &entry.local, key.0);
    local.wants_push().then_some(key)
}

// ----------------------------------------------------------------------------
/// The calendar of the event `id`, if its auto-sync is on and two-way and
/// would delete the event in Google once it is deleted here: one synced
/// with it, not cancelled. Asked before the delete (`deleted`).
pub(crate) fn waiting_delete(app: &App, id: &Id) -> Option<SourceKey> {
    let entry = app.vault.index().entry(id)?;
    let key = two_way_auto_sync(app, &entry.doc)?;
    let local = local_event(&entry.doc, &entry.local, key.0);
    (local.remote_event_id.is_some() && !local.cancelled).then_some(key)
}

// ----------------------------------------------------------------------------
/// The calendar source of the event `doc`, if its auto-sync is on and
/// two-way.
fn two_way_auto_sync(app: &App, doc: &Document) -> Option<SourceKey> {
    let key = source_of(app, doc)?;
    let client = app.google.connected(key.0).ok()?;
    let two_way = client.scope().calendar == ScopeAccess::Full;
    (two_way && auto_sync_sources(app).contains(&key)).then_some(key)
}

// ----------------------------------------------------------------------------
/// The calendar source of the event `doc`: the account and the Google
/// calendar its calendar entity names.
fn source_of(app: &App, doc: &Document) -> Option<SourceKey> {
    let calendar = doc.roles().event.as_ref()?.calendar?;
    let entry = app.vault.index().entry(&calendar)?;
    let Some(Source::Remote(remote)) = entry.doc.source() else {
        return None;
    };
    Some((remote.account?, remote.get(CALENDAR_ID)?.to_string()))
}

// ----------------------------------------------------------------------------
/// The calendars one of whose events the page shows in edit mode: their
/// auto-sync waits, so a change from Google never lands in an open editor.
fn editing_sources(app: &App) -> HashSet<SourceKey> {
    let entries = app
        .documents
        .editing()
        .filter_map(|id| app.vault.index().entry(id));
    entries.filter_map(|e| source_of(app, &e.doc)).collect()
}

// ----------------------------------------------------------------------------
/// The window is active again (`App::on_activated`): a calendar whose
/// auto-sync last looked at Google 2 minutes ago or more is due -- a change
/// just made in Google Calendar shows soon -- and the first due runs now.
pub(crate) fn activated(app: &mut App, now: DateTime) {
    // Each look sets the next one `SYNC_EVERY` out (`auto_sync`, `start`):
    // one at least `ACTIVATED_AFTER` ago left the next one that much nearer.
    let stale = now.plus_seconds(SYNC_EVERY - ACTIVATED_AFTER);
    let sources = auto_sync_sources(app);
    app.calendar.auto.due_if_older(&sources, stale);
    auto_sync(app, now);
}
