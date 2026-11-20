//! A sync's effects applied to the vault: events created, changed, cancelled
//! and restored.

use std::path::{Path, PathBuf};

use av_google::calendar::Event as GoogleEvent;
use av_google::calendar_sync::{self, Effect, SyncOutput};
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::document::Document;
use av_vault::envelope::{Event, Remote, Roles, Source};
use av_vault::filename;

use super::mapping::{
    apply_to, event_entity, from_google, payload_of, take_attendees, take_exceptions, to_google,
};
use super::{CANCELLED, CalendarSync};
use crate::app::App;
use crate::documents;
use crate::google;
use crate::jobs;
use crate::texts::Text;
use av_vault::schema::layout;
use av_vault::schema::remote::{CALENDAR_ID, ETAG, EVENT_ID, SNAPSHOT, SYNC_TOKEN, TIME_MIN};

/// What a sync came to, for the job's summary.
#[derive(Default)]
pub(super) struct Counts {
    pub(super) created: usize,
    pub(super) updated: usize,
    pub(super) pushed: usize,
    pub(super) deleted: usize,
    pub(super) restored: usize,
    pub(super) conflicts: usize,
    pub(super) skipped: usize,
    pub(super) failed: usize,
}

impl Counts {
    // ------------------------------------------------------------------------
    /// "3 new · 1 updated · 1 conflict": the counts that aren't zero.
    pub(super) fn summary(&self) -> Vec<Text> {
        let counts = vec![
            (self.created, Text::new("count.new")),
            (self.updated, Text::new("count.updated")),
            (self.pushed, Text::new("count.pushed")),
            (self.deleted, Text::new("count.deleted")),
            (self.restored, Text::new("count.restored")),
            (self.conflicts, Text::new("count.conflicts")),
            (self.skipped, Text::new("calendar.count.skipped")),
            (self.failed, Text::new("count.failed")),
        ];
        jobs::count_summary(counts, Text::new("job.nothing_new"))
    }
}

// ----------------------------------------------------------------------------
/// How applying one effect went.
enum Applied {
    Done,
    /// The event changed in the vault while the sync ran: left for the
    /// next run, which plans it again.
    Skipped,
}

// ----------------------------------------------------------------------------
/// The sync ran, or was stopped and returned what it did so far: apply its
/// effects in order, report, and end the job. Ignored if the job is gone.
pub(super) fn apply(app: &mut App, job_id: Id, result: Result<SyncOutput>) {
    let Some(sync) = app.calendar.sync.take_if(|s| s.job_id == job_id) else {
        return;
    };
    let output = match result {
        Ok(output) => output,
        Err(Error::Cancelled) => return app.end_job(job_id, vec![Text::new("job.stopped")]),
        Err(Error::Expired) => return app.end_job(job_id, vec![Text::new("calendar.reconnect")]),
        Err(e) => {
            let failed = Text::new("job.failed").arg("error", e.to_string());
            return app.end_job(job_id, vec![failed]);
        }
    };

    // What the stop cancelled isn't a failure; the next sync does it.
    let report = output.report;
    let failures: Vec<_> = report
        .failures
        .iter()
        .filter(|f| !matches!(f.error, Error::Cancelled))
        .collect();
    let mut counts = Counts {
        pushed: report.pushed,
        deleted: report.deleted,
        conflicts: report.conflicts.len(),
        skipped: report.skipped_exceptions,
        failed: failures.len(),
        ..Counts::default()
    };
    // An auto-sync of a calendar that failed before says it only once.
    let key = (sync.account, sync.source.clone());
    let quiet = sync.auto_sync && app.calendar.auto.is_failing(&key);
    // The host's words in the status bar, the error in the log
    // (`App::notice`), with the text's key: not the event's subject.
    let fail = |app: &App, text: Text, error: &Error| {
        log::warn!("{}: {error}", text.key);
        if !quiet {
            app.notice(text);
        }
    };
    for failure in failures {
        let text = Text::new("calendar.sync_failed").arg("subject", failure.subject.as_str());
        fail(app, text, &failure.error);
    }

    // A sync token only moves on once everything before it is applied.
    let mut clean = true;
    for effect in output.effects {
        if matches!(effect, Effect::SetSyncToken { .. }) && !clean {
            log::info!("Calendar sync {job_id}: sync token kept for the next run");
            continue;
        }
        match apply_effect(app, &sync, effect, &mut counts) {
            Ok(Applied::Done) => {}
            Ok(Applied::Skipped) => clean = false,
            Err(e) => {
                clean = false;
                counts.failed += 1;
                fail(app, Text::new("calendar.write_failed"), &e);
            }
        }
    }
    if counts.failed == 0 {
        app.calendar.auto.worked(&key);
    } else if sync.auto_sync {
        app.calendar.auto.failed(key);
    }
    finish(app, sync, &counts);
}

// ----------------------------------------------------------------------------
fn apply_effect(
    app: &mut App,
    sync: &CalendarSync,
    effect: Effect,
    counts: &mut Counts,
) -> Result<Applied> {
    let account = sync.account;
    match effect {
        Effect::CreateLocal {
            remote,
            remote_calendar_id,
            exceptions,
            ..
        } => {
            create_event(app, sync, &remote, &exceptions, &remote_calendar_id)?;
            counts.created += 1;
        }
        Effect::OverwriteLocal {
            path,
            remote,
            remote_calendar_id,
        } => {
            let Some(id) = unchanged_event(app, sync, &path) else {
                return Ok(Applied::Skipped);
            };
            if take_remote(app, account, id, &remote, &remote_calendar_id)? {
                counts.updated += 1;
            }
        }
        Effect::CancelLocal { path, etag } => {
            let Some(id) = unchanged_event(app, sync, &path) else {
                return Ok(Applied::Skipped);
            };
            if cancel_event(app, account, id, etag.as_deref())? {
                counts.updated += 1;
            }
        }
        Effect::Conflict {
            path,
            remote,
            remote_calendar_id,
        } => {
            let Some(id) = unchanged_event(app, sync, &path) else {
                return Ok(Applied::Skipped);
            };
            keep_local_version(app, id)?;
            match remote {
                Some(remote) => take_remote(app, account, id, &remote, &remote_calendar_id)?,
                None => cancel_event(app, account, id, None)?,
            };
        }
        Effect::SetRemote {
            path,
            remote_event_id,
            etag,
            snapshot,
            remote_calendar_id,
            remote,
        } => {
            // Bookkeeping (and pulled attendees) only: applied even if the
            // event changed meanwhile, which then still counts as changed.
            let Some(&(id, _)) = sync.events.get(&path) else {
                return Ok(Applied::Skipped);
            };
            let before = app
                .vault
                .index()
                .entry(&id)
                .and_then(|e| e.doc.roles().event.clone());
            app.edit_written(|index, changes| {
                index.edit(&id, changes, |doc| {
                    take_attendees(doc, &remote);
                    set_bookkeeping(doc, account, |r| {
                        r.set(EVENT_ID, &remote_event_id);
                        r.set(CALENDAR_ID, &remote_calendar_id);
                        if let Some(etag) = &etag {
                            r.set(ETAG, etag);
                        }
                        r.set(SNAPSHOT, &snapshot);
                    });
                })
            })?;
            let after = app
                .vault
                .index()
                .entry(&id)
                .and_then(|e| e.doc.roles().event.clone());
            let attendees = |e: Option<Event>| e.map(|e| e.attendees).unwrap_or_default();
            if attendees(before) != attendees(after) {
                counts.updated += 1;
            }
        }
        Effect::SetExceptions { path, exceptions } => {
            // Pulled only: applied even if the event changed meanwhile.
            let Some(&(id, _)) = sync.events.get(&path) else {
                return Ok(Applied::Skipped);
            };
            if set_exceptions(app, id, &exceptions)? {
                counts.updated += 1;
            }
        }
        Effect::CancelTrashed { path, etag } => {
            let Some(&(id, _)) = sync.events.get(&path) else {
                return Ok(Applied::Skipped);
            };
            cancel_trashed(app, account, id, &path, etag.as_deref())?;
        }
        Effect::RestoreLocal {
            path,
            remote,
            remote_calendar_id,
            exceptions,
        } => {
            let Some(&(id, _)) = sync.events.get(&path) else {
                return Ok(Applied::Skipped);
            };
            let back = Back {
                remote: &remote,
                instances: &exceptions,
                remote_calendar_id: &remote_calendar_id,
            };
            match restore_event(app, sync, id, &path, back)? {
                true => counts.restored += 1,
                false => counts.created += 1,
            }
        }
        Effect::SetSyncToken {
            token, time_min, ..
        } => {
            app.edit_written(|index, changes| {
                index.edit(&sync.calendar, changes, |doc| {
                    set_bookkeeping(doc, account, |r| {
                        r.set(SYNC_TOKEN, &token);
                        r.set(TIME_MIN, &time_min);
                    });
                })
            })?;
        }
    }
    Ok(Applied::Done)
}

// ----------------------------------------------------------------------------
/// The id of the event at `path` -- if it is still what the job planned
/// with (the user didn't edit it while the sync ran).
fn unchanged_event(app: &App, sync: &CalendarSync, path: &Path) -> Option<Id> {
    let (id, planned) = sync.events.get(path)?;
    let doc = &app.vault.index().entry(id)?.doc;
    (calendar_sync::snapshot(&payload_of(doc)) == *planned).then_some(*id)
}

// ----------------------------------------------------------------------------
/// Change `doc`'s `remote:` block from `account` (a fresh one if it has
/// none from it, e.g. an event made in the vault and pushed).
pub(super) fn set_bookkeeping(doc: &mut Document, account: Id, change: impl FnOnce(&mut Remote)) {
    let mut remote = doc.remote_from(Some(&account)).cloned().unwrap_or(Remote {
        account: Some(account),
        remote: Vec::new(),
    });
    change(&mut remote);
    doc.set_source(Some(Source::Remote(remote)));
}

// ----------------------------------------------------------------------------
/// A new event entity for `remote` (with Google's `instances` of it, if it
/// is a recurring event) in the calendar's events folder.
fn create_event(
    app: &mut App,
    sync: &CalendarSync,
    remote: &GoogleEvent,
    instances: &[GoogleEvent],
    remote_calendar_id: &str,
) -> Result<()> {
    let mut doc = event_entity(remote, sync.calendar, sync.account, remote_calendar_id);
    take_exceptions(&mut doc, instances);
    let folder = events_folder(app, sync.calendar);
    app.edit(|index, changes| index.create(doc, &folder, changes))?;
    Ok(())
}

// ----------------------------------------------------------------------------
/// Where the events of the calendar entity `calendar` go: a folder named
/// after it, beside it.
fn events_folder(app: &App, calendar: Id) -> PathBuf {
    let entry = app.vault.index().entry(&calendar);
    let beside = entry
        .and_then(|e| e.local.parent())
        .unwrap_or(Path::new(layout::CALENDAR));
    let title = entry.map_or("", |e| e.doc.title());
    beside.join(filename::folder_name(title))
}

// ----------------------------------------------------------------------------
/// Let the remote version win the event `id`: synced fields, attendees and
/// bookkeeping. Whether the file changed beyond the bookkeeping.
fn take_remote(
    app: &mut App,
    account: Id,
    id: Id,
    remote: &GoogleEvent,
    remote_calendar_id: &str,
) -> Result<bool> {
    let fields = from_google(remote);
    let snapshot = calendar_sync::snapshot(&to_google(&fields));
    let Some(entry) = app.vault.index().entry(&id) else {
        return Err(Error::NotFound);
    };
    let (body, event) = (
        entry.doc.content().to_string(),
        entry.doc.roles().event.clone(),
    );
    let title = entry.doc.title().to_string();

    app.edit_written(|index, changes| {
        index.edit(&id, changes, |doc| {
            apply_to(&fields, doc);
            take_attendees(doc, remote);
            set_bookkeeping(doc, account, |r| {
                if let Some(event_id) = &remote.id {
                    r.set(EVENT_ID, event_id);
                }
                r.set(CALENDAR_ID, remote_calendar_id);
                if let Some(etag) = &remote.etag {
                    r.set(ETAG, etag);
                }
                r.set(SNAPSHOT, &snapshot);
            });
        })
    })?;

    // A changed body has gone to the page with the entity (`App::edit`).
    let doc = app.vault.index().entry(&id).map(|e| &e.doc);
    let changed =
        doc.is_some_and(|d| d.content() != body || d.roles().event != event || d.title() != title);
    Ok(changed)
}

// ----------------------------------------------------------------------------
/// Mark the event `id` cancelled (deleted in Google), recording `etag`.
/// Whether it wasn't already.
fn cancel_event(app: &mut App, account: Id, id: Id, etag: Option<&str>) -> Result<bool> {
    let was_cancelled = app
        .vault
        .index()
        .entry(&id)
        .and_then(|e| e.doc.roles().event.as_ref()?.status.clone())
        .is_some_and(|s| s == CANCELLED);
    app.edit_written(|index, changes| {
        index.edit(&id, changes, |doc| mark_cancelled(doc, account, etag))
    })?;
    Ok(!was_cancelled)
}

// ----------------------------------------------------------------------------
/// Mark the event `doc` cancelled, recording `etag` and the snapshot that
/// makes it count as synced.
fn mark_cancelled(doc: &mut Document, account: Id, etag: Option<&str>) {
    let mut event = doc.roles().event.clone().unwrap_or_default();
    event.status = Some(CANCELLED.to_string());
    doc.set_event(Some(event));
    let snapshot = calendar_sync::snapshot(&payload_of(doc));
    set_bookkeeping(doc, account, |r| {
        if let Some(etag) = etag {
            r.set(ETAG, etag);
        }
        r.set(SNAPSHOT, &snapshot);
    });
}

// ----------------------------------------------------------------------------
/// Take Google's `instances` of the recurring event `id` (`take_exceptions`).
/// Whether its file changed.
fn set_exceptions(app: &mut App, id: Id, instances: &[GoogleEvent]) -> Result<bool> {
    app.edit_written(|index, changes| {
        index.edit(&id, changes, |doc| take_exceptions(doc, instances))
    })
}

// ----------------------------------------------------------------------------
/// The trashed file of the event `id`, at `path` under `.trash`: mark it
/// cancelled -- it is gone in Google too -- recording `etag`. A file that
/// left the trash meanwhile (restored, or deleted for good) or holds
/// another entity is left alone.
fn cancel_trashed(app: &App, account: Id, id: Id, path: &Path, etag: Option<&str>) -> Result<()> {
    let edited = app.vault.index().edit_trashed(in_trash(path), |doc| {
        if doc.id() == id {
            mark_cancelled(doc, account, etag);
        }
    });
    match edited {
        Ok(_) => Ok(()),
        Err(Error::FileNotFound { .. }) => {
            log::info!("{}: no longer in the trash", path.display());
            Ok(())
        }
        Err(e) => Err(e),
    }
}

// ----------------------------------------------------------------------------
/// `path` (under `.trash`) relative to the trash.
fn in_trash(path: &Path) -> &Path {
    path.strip_prefix(layout::TRASH).unwrap_or(path)
}

// ----------------------------------------------------------------------------
/// What comes back of an event deleted in the vault (`restore_event`).
struct Back<'a> {
    pub(super) remote: &'a GoogleEvent,
    pub(super) instances: &'a [GoogleEvent],
    pub(super) remote_calendar_id: &'a str,
}

// ----------------------------------------------------------------------------
/// The event `id`, deleted in the vault, changed in Google since: its file
/// at `path` (under `.trash`) is restored and takes Google's version, so it
/// keeps its id, labels, projects and links. If the restore is refused (a
/// file in its place, its id in the vault, the file gone), it comes back as
/// a new event, and its trashed copy is marked cancelled. Whether it was
/// restored.
fn restore_event(
    app: &mut App,
    sync: &CalendarSync,
    id: Id,
    path: &Path,
    back: Back,
) -> Result<bool> {
    let local = in_trash(path).to_path_buf();
    match app.edit(|index, changes| index.restore(&local, changes)) {
        Ok(restored) => {
            log::info!("{}: restored, changed in Google", path.display());
            take_remote(
                app,
                sync.account,
                restored,
                back.remote,
                back.remote_calendar_id,
            )?;
            if !back.instances.is_empty() {
                set_exceptions(app, restored, back.instances)?;
            }
            Ok(true)
        }
        Err(e) => {
            log::info!(
                "{}: not restored ({e}), back as a new event",
                path.display()
            );
            create_event(
                app,
                sync,
                back.remote,
                back.instances,
                back.remote_calendar_id,
            )?;
            cancel_trashed(app, sync.account, id, path, None)?;
            Ok(false)
        }
    }
}

// ----------------------------------------------------------------------------
/// Before a conflict lets the remote win: keep the local version of `id`
/// as a new entity beside it, made here and labelled `conflict`, without the
/// recurring event's exceptions. It belongs to the same calendar, so a two-way sync
/// pushes it as a new event.
pub(super) fn keep_local_version(app: &mut App, id: Id) -> Result<()> {
    // Exceptions belong to Google's recurring event, not to the new event.
    let roles = |roles: &mut Roles| {
        if let Some(event) = &mut roles.event {
            event.exceptions.clear();
        }
    };
    documents::keep_conflict_copy(app, id, None, roles)?;
    Ok(())
}

// ----------------------------------------------------------------------------
/// The sync is done: finish the account's sync (`google::finish_sync`), and
/// end the job with its summary.
fn finish(app: &mut App, sync: CalendarSync, counts: &Counts) {
    google::finish_sync(app, sync.account, |_| {});
    app.end_job_with(sync.job_id, counts.summary(), false);
}
