//! The Google Calendar sync engine.
//!
//! The caller (the app shell) collects a snapshot of the linked calendars'
//! events on its UI thread and hands over a `SyncJob`; `run_sync` does all the
//! HTTP and returns *effects* plus a report — it never touches vault files
//! itself (they are only ever written on the caller's UI thread). Effects are
//! applied in order; each calendar's `SetSyncToken` comes last and must only
//! be applied if everything before it was.
//!
//! A caller that syncs on its own (an auto-sync) can list a calendar's changes
//! first, writing nothing, hand them to the job and run it only if it has
//! something to do.
//!
//! The decision core is `plan()`, a pure function over (local snapshot, remote
//! deltas) — unit-tested with no HTTP anywhere near it. Change detection: an
//! event is locally changed iff the snapshot of its mapped payload differs
//! from the one stored at the last sync (state lives in the files, never in a
//! cache); remotely changed iff the snapshot of the delta, as the local side
//! would store it, differs from that stored snapshot.
//!
//! A series' instances that Google reports (occurrences moved, retitled or
//! cancelled) are pulled only, outside change detection: the local side keeps
//! them on the series, and only an instance that changes what it keeps is
//! pulled.
//!
//! A two-way sync also deletes in Google what was deleted locally (the
//! caller's trash) -- unless Google changed it since: then it comes back.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use av_utils::cancel::Cancel;
use av_utils::error::{Error, Result};
use av_utils::fnv::fnv1a_64;
use serde::Serialize;

use crate::calendar::{self, DeleteOutcome, Event, WriteOutcome};
use crate::client::Client;

// ----------------------------------------------------------------------------
/// Change-detection snapshot: FNV-1a 64 over the pushable payload's JSON.
/// Struct-field serialization order is fixed, and this same payload is
/// what gets pushed — snapshot and push can never disagree about what
/// "changed" means.
pub fn snapshot(payload: &impl Serialize) -> String {
    let json = serde_json::to_vec(payload).unwrap_or_default();
    format!("{:016x}", fnv1a_64(&json))
}

// ----------------------------------------------------------------------------
/// Our UUIDv7 as a Google event id: hex with the dashes stripped is valid
/// base32hex alphabet, making inserts idempotent (a retry finds the id
/// taken and adopts instead of duplicating).
pub fn google_id_for(local_id: &str) -> String {
    local_id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_lowercase()
}

// ----------------------------------------------------------------------------
// job input (built on the caller's UI thread)

pub struct LocalEvent {
    pub local_id: String,
    pub path: PathBuf,
    /// The current file mapped to Google's shape — what a push would send.
    pub payload: Event,
    /// `snapshot(&payload)`.
    pub snapshot: String,
    /// The snapshot recorded at the last sync (`remote.snapshot`).
    pub stored_snapshot: Option<String>,
    pub remote_event_id: Option<String>,
    pub etag: Option<String>,
    /// The Google calendar the event was last synced with — differing from
    /// the job's calendar means the event was moved locally and needs
    /// `events.move` first.
    pub remote_calendar_id: Option<String>,
    pub cancelled: bool,
    /// The instances of the series the local side keeps, as
    /// `SyncJob::exception` gives them; empty for a single event.
    pub exceptions: Vec<Event>,
    /// Deleted locally: its file is in the trash (`path`). A two-way sync
    /// deletes it in Google.
    pub deleted: bool,
}

impl LocalEvent {
    // ------------------------------------------------------------------------
    fn changed(&self) -> bool {
        self.stored_snapshot.as_deref() != Some(self.snapshot.as_str())
    }

    // ------------------------------------------------------------------------
    /// Whether a two-way sync pushes the event: one never synced (but a
    /// tombstone, which stays local), one changed since its last sync, or
    /// one deleted locally that Google still has (a delete).
    pub fn wants_push(&self) -> bool {
        match &self.remote_event_id {
            None => !self.cancelled && !self.deleted,
            Some(_) if self.deleted => !self.cancelled,
            Some(_) => self.changed(),
        }
    }

    // ------------------------------------------------------------------------
    /// Whether the remote `delta` -- `remote`, its normalized snapshot --
    /// changed nothing the local side keeps since the last sync: its etag
    /// is the one recorded (e.g. the echo of our own push), or it hashes
    /// to the stored snapshot (e.g. only an attendee answered).
    fn untouched_by(&self, delta: &Event, remote: &str) -> bool {
        (delta.etag.is_some() && delta.etag == self.etag)
            || self.stored_snapshot.as_deref() == Some(remote)
    }

    // ------------------------------------------------------------------------
    /// How the event is named in the report.
    fn label(&self) -> String {
        self.payload
            .summary
            .clone()
            .unwrap_or_else(|| self.local_id.clone())
    }

    // ------------------------------------------------------------------------
    /// Whether the series' instance `delta` changes what the local side
    /// keeps of it: it differs from the kept one with the same original
    /// start, or there is none and it keeps something.
    fn exception_changed(&self, delta: &Event, exception: fn(&Event, &Event) -> Event) -> bool {
        let kept = exception(delta, &self.payload);
        let original = &kept.original_start_time;
        match self
            .exceptions
            .iter()
            .find(|x| x.original_start_time == *original)
        {
            Some(stored) => *stored != kept,
            None => {
                let nothing = Event {
                    original_start_time: original.clone(),
                    ..Event::default()
                };
                kept != nothing
            }
        }
    }
}

// ----------------------------------------------------------------------------
pub struct CalendarJob {
    pub calendar_path: PathBuf,
    pub local_calendar_id: String,
    /// The linked Google calendar.
    pub remote_calendar_id: String,
    pub sync_token: Option<String>,
    /// The timeMin the original full sync used (an expired sync token's
    /// full re-list reuses it so the token's query stays consistent).
    pub stored_time_min: Option<String>,
    pub events: Vec<LocalEvent>,
    /// The changes listed already (`list_changes`); none: the sync lists
    /// them itself.
    pub listed: Option<Changes>,
}

// ----------------------------------------------------------------------------
/// What a calendar's events list found (`list_changes`).
pub struct Changes {
    /// Google's deltas since the sync token; after a full list, every event
    /// from the time-min on.
    pub deltas: Vec<Event>,
    pub next_sync_token: Option<String>,
    /// The time-min a full list starts at, kept with the token.
    pub time_min: String,
    /// Whether it was a full list: there was no sync token, or Google no
    /// longer took it. Its token is new either way.
    pub full: bool,
}

// ----------------------------------------------------------------------------
pub struct SyncJob {
    /// The RFC 3339 lower bound of a calendar's first full sync (the
    /// caller's choice, e.g. a year back).
    pub time_min: String,
    /// Push local changes (the account may write the calendars); pull
    /// only otherwise: local changes then stay pending.
    pub push: bool,
    /// A remote event as the local side would store it, in Google's shape
    /// (the caller's mapping to its format and back), as a pushable
    /// payload. Change detection compares only this, so fields or formats
    /// the local side doesn't keep never count as changes.
    pub normalize: fn(&Event) -> Event,
    /// An instance of a series as the local side would keep it, given the
    /// series' payload (`LocalEvent::payload`): in Google's shape, with its
    /// `original_start_time`, and nothing else if the local side keeps
    /// nothing of it. Whether an instance is pulled compares only this.
    pub exception: fn(&Event, &Event) -> Event,
    pub calendars: Vec<CalendarJob>,
}

impl SyncJob {
    // ------------------------------------------------------------------------
    /// Whether running the job would change anything, given its calendars'
    /// listed changes (`CalendarJob::listed`): a full list (its new token
    /// is kept), a delta to pull or record, or a local event to push. A
    /// calendar not listed yet counts as work.
    pub fn has_work(&self) -> bool {
        self.calendars.iter().any(|cal| {
            let Some(changes) = &cal.listed else {
                return true;
            };
            let mut out = SyncOutput::default();
            let pushes = plan(cal, &changes.deltas, self, &mut out);
            changes.full || !pushes.is_empty() || !out.effects.is_empty()
        })
    }
}

// ----------------------------------------------------------------------------
// output (applied on the caller's UI thread)

pub enum Effect {
    /// A new event, with the instances Google reports of it if it is a
    /// series.
    CreateLocal {
        calendar_path: PathBuf,
        local_calendar_id: String,
        remote: Event,
        remote_calendar_id: String,
        exceptions: Vec<Event>,
    },
    OverwriteLocal {
        path: PathBuf,
        remote: Event,
        remote_calendar_id: String,
    },
    /// Remote tombstone for an unchanged local event: set status only.
    CancelLocal { path: PathBuf, etag: Option<String> },
    /// Both sides changed: preserve the local version as a sibling, then
    /// let the remote version win the file (None = remote cancelled it).
    Conflict {
        path: PathBuf,
        remote: Option<Event>,
        remote_calendar_id: String,
    },
    /// Refresh sync bookkeeping (after a push, or a delta that changes
    /// nothing the local side syncs). `remote` is the server's current
    /// version -- the delta, or the write's response -- for the fields the
    /// caller only pulls (and never syncs back).
    SetRemote {
        path: PathBuf,
        remote_event_id: String,
        etag: Option<String>,
        snapshot: String,
        remote_calendar_id: String,
        remote: Box<Event>,
    },
    /// Instances of the series at `path` that change what the local side
    /// keeps of them (`SyncJob::exception`): pulled only, applied even if
    /// the event changed meanwhile.
    SetExceptions {
        path: PathBuf,
        exceptions: Vec<Event>,
    },
    /// An event deleted locally (its file in the trash, `path`) is gone in
    /// Google too -- deleted by this sync, or there: its trashed file says
    /// so (`cancelled`, and the `etag` if known), so no sync deletes it
    /// again.
    CancelTrashed { path: PathBuf, etag: Option<String> },
    /// An event deleted locally changed in Google since: it comes back from
    /// the trash (`path`) with the remote version, and the instances Google
    /// reports of it if it is a series.
    RestoreLocal {
        path: PathBuf,
        remote: Event,
        remote_calendar_id: String,
        exceptions: Vec<Event>,
    },
    SetSyncToken {
        calendar_path: PathBuf,
        token: String,
        time_min: String,
    },
}

// ----------------------------------------------------------------------------
/// Something that failed and is retried on the next run.
#[derive(Debug)]
pub struct Failure {
    /// The event (its summary) or calendar (its Google id) concerned.
    pub subject: String,
    pub error: Error,
}

// ----------------------------------------------------------------------------
#[derive(Default)]
pub struct Report {
    pub pulled: usize,
    pub pushed: usize,
    /// Events deleted in Google because they were deleted locally.
    pub deleted: usize,
    pub conflicts: Vec<String>,
    /// Instances of a series the local side doesn't have.
    pub skipped_exceptions: usize,
    pub failures: Vec<Failure>,
}

// ----------------------------------------------------------------------------
/// Everything a sync produced.
#[derive(Default)]
pub struct SyncOutput {
    pub effects: Vec<Effect>,
    pub report: Report,
}

impl SyncOutput {
    // ------------------------------------------------------------------------
    fn pulled(&mut self, effect: Effect) {
        self.report.pulled += 1;
        self.effects.push(effect);
    }

    // ------------------------------------------------------------------------
    fn conflict(&mut self, cal: &CalendarJob, l: &LocalEvent, remote: Option<Event>) {
        self.report.conflicts.push(l.label());
        self.effects.push(Effect::Conflict {
            path: l.path.clone(),
            remote,
            remote_calendar_id: cal.remote_calendar_id.clone(),
        });
    }

    // ------------------------------------------------------------------------
    /// `l`, deleted locally, is gone in Google too.
    fn cancel_trashed(&mut self, l: &LocalEvent, etag: Option<String>) {
        self.effects.push(Effect::CancelTrashed {
            path: l.path.clone(),
            etag,
        });
    }

    // ------------------------------------------------------------------------
    /// `l`, deleted locally, comes back as `remote`.
    fn restore(
        &mut self,
        cal: &CalendarJob,
        l: &LocalEvent,
        remote: Event,
        exceptions: Vec<Event>,
    ) {
        self.pulled(Effect::RestoreLocal {
            path: l.path.clone(),
            remote,
            remote_calendar_id: cal.remote_calendar_id.clone(),
            exceptions,
        });
    }

    // ------------------------------------------------------------------------
    /// Record `remote` (the server's current version) as `l`'s remote.
    fn set_remote(
        &mut self,
        cal: &CalendarJob,
        l: &LocalEvent,
        remote_event_id: String,
        snapshot: String,
        remote: Event,
    ) {
        self.effects.push(Effect::SetRemote {
            path: l.path.clone(),
            remote_event_id,
            etag: remote.etag.clone(),
            snapshot,
            remote_calendar_id: cal.remote_calendar_id.clone(),
            remote: Box::new(remote),
        });
    }
}

// ----------------------------------------------------------------------------
/// A local event to push (`plan`): its index into `CalendarJob::events`,
/// and the etag a patch may assume of the remote (If-Match).
#[derive(Debug, PartialEq)]
pub struct Push {
    pub index: usize,
    pub etag: Option<String>,
    /// The remote id of the event's own earlier insert, found by the list
    /// though never recorded: patched, not inserted again.
    pub found: Option<String>,
}

// ----------------------------------------------------------------------------
/// Decide what to do with `cal`, given the remote deltas: pull effects go
/// straight into `out`; the returned locals need a push (insert or patch),
/// none unless `job.push`. Deltas repeated across pages resolve last-wins;
/// a local matched by any delta is pushed only if that delta changed
/// nothing it syncs.
///
/// A series' instances go with it: a new one's in `CreateLocal`, a known
/// one's that change what it keeps in `SetExceptions` -- unless the series
/// is cancelled. Those of a series the local side doesn't have are counted.
///
/// Locally deleted events only count with `job.push`, and a live event with
/// the same remote id wins over them (one that came back while pulling
/// only). One is deleted in Google (a push) unless the remote changed since
/// the last sync: cancelled there too, its trashed file says so
/// (`CancelTrashed`); changed, it comes back (`RestoreLocal`).
///
/// A live local event never synced is also matched under the id its insert
/// used (`google_id_for`), unless a synced event holds that id: listed, it
/// is its own earlier insert, which a run stopped before recording (a
/// stop, a crash). It counts as untouched -- the remote was born from a
/// local version, so local wins, as `push_one`'s adopt does -- and is never
/// created a second time.
///
/// For a matched delta, "untouched" means nothing the local side keeps
/// changed remotely since the last sync: the etag is the one recorded
/// (e.g. the echo of our own push), or the normalized delta hashes to the
/// stored snapshot (e.g. only an attendee answered). A refresh that would
/// record what is recorded already (the same etag, snapshot and calendar)
/// is left out: Google changes the etag with any change, attendees too.
///
/// | remote               | local unchanged  | local changed           |
/// |----------------------|------------------|-------------------------|
/// | cancelled            | cancel (refresh) | conflict, remote gone   |
/// | equal to local       | refresh          | refresh                 |
/// | untouched            | refresh          | push with delta's etag  |
/// | otherwise            | overwrite        | conflict                |
pub fn plan(cal: &CalendarJob, deltas: &[Event], job: &SyncJob, out: &mut SyncOutput) -> Vec<Push> {
    let (push, normalize) = (job.push, job.normalize);
    let locals = &cal.events;
    let mut by_remote: HashMap<&str, usize> = HashMap::new();
    for deleted in [true, false] {
        let counted = locals
            .iter()
            .enumerate()
            .filter(|(_, l)| l.deleted == deleted && (push || !deleted));
        for (i, l) in counted {
            if let Some(rid) = l.remote_event_id.as_deref() {
                by_remote.insert(rid, i);
            }
        }
    }
    let own_inserts: HashMap<String, usize> = locals
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.deleted && l.remote_event_id.is_none())
        .map(|(i, l)| (google_id_for(&l.local_id), i))
        .filter(|(gid, _)| !by_remote.contains_key(gid.as_str()))
        .collect();
    let find = |rid: &str| by_remote.get(rid).or_else(|| own_inserts.get(rid)).copied();

    // The last delta per remote id, ordered by id so effects are stable;
    // the instances by their series' id, the last per instance.
    let mut last: BTreeMap<&str, &Event> = BTreeMap::new();
    let mut instances: BTreeMap<&str, BTreeMap<&str, &Event>> = BTreeMap::new();
    for d in deltas {
        let Some(rid) = d.id.as_deref() else {
            continue;
        };
        match d.recurring_event_id.as_deref() {
            Some(series) => {
                instances.entry(series).or_default().insert(rid, d);
            }
            None => {
                last.insert(rid, d);
            }
        }
    }

    let mut matched = HashSet::new();
    let mut pushes = Vec::new();
    for (&rid, &d) in &last {
        let remote_cancelled = is_cancelled(d);
        let Some(idx) = find(rid) else {
            // New remotely, with its instances; created-then-deleted within
            // the window is a no-op.
            let exceptions = instances.remove(rid).unwrap_or_default();
            if !remote_cancelled {
                out.pulled(Effect::CreateLocal {
                    calendar_path: cal.calendar_path.clone(),
                    local_calendar_id: cal.local_calendar_id.clone(),
                    remote: d.clone(),
                    remote_calendar_id: cal.remote_calendar_id.clone(),
                    exceptions: exceptions.into_values().cloned().collect(),
                });
            }
            continue;
        };
        matched.insert(idx);
        let l = &locals[idx];
        if l.deleted {
            // Deleted locally: gone in Google too, deleted there now, or
            // back if Google changed it since.
            let remote = snapshot(&normalize(d));
            if remote_cancelled {
                out.cancel_trashed(l, d.etag.clone());
            } else if l.untouched_by(d, &remote) || remote == l.snapshot {
                pushes.push(Push {
                    index: idx,
                    etag: d.etag.clone(),
                    found: None,
                });
            } else {
                let exceptions = instances.remove(rid).unwrap_or_default();
                out.restore(
                    cal,
                    l,
                    d.clone(),
                    exceptions.into_values().cloned().collect(),
                );
            }
            continue;
        }
        // Only the bookkeeping moves; `snapshot` is what counts as synced.
        let refresh = |out: &mut SyncOutput, snapshot: String| {
            let recorded = d.etag.is_some()
                && d.etag == l.etag
                && l.stored_snapshot.as_ref() == Some(&snapshot)
                && l.remote_calendar_id.as_ref() == Some(&cal.remote_calendar_id);
            if !recorded {
                out.set_remote(cal, l, rid.to_string(), snapshot, d.clone());
            }
        };
        if remote_cancelled {
            if l.cancelled {
                refresh(out, l.snapshot.clone());
            } else if !l.changed() {
                out.pulled(Effect::CancelLocal {
                    path: l.path.clone(),
                    etag: d.etag.clone(),
                });
            } else {
                out.conflict(cal, l, None);
            }
            continue;
        }
        let remote = snapshot(&normalize(d));
        let own_insert = l.remote_event_id.is_none();
        let untouched = own_insert || l.untouched_by(d, &remote);
        if remote == l.snapshot || (untouched && !l.changed()) {
            refresh(out, l.snapshot.clone());
        } else if untouched && push {
            pushes.push(Push {
                index: idx,
                etag: d.etag.clone(),
                found: own_insert.then(|| rid.to_string()),
            });
        } else if untouched {
            // Pull only: the local change stays pending.
            refresh(out, l.stored_snapshot.clone().unwrap_or_default());
        } else if !l.changed() {
            out.pulled(Effect::OverwriteLocal {
                path: l.path.clone(),
                remote: d.clone(),
                remote_calendar_id: cal.remote_calendar_id.clone(),
            });
        } else {
            out.conflict(cal, l, Some(d.clone()));
        }
    }

    for (series, deltas) in instances {
        let Some(idx) = find(series) else {
            out.report.skipped_exceptions += deltas.len();
            continue;
        };
        let l = &locals[idx];
        let cancelled = l.deleted || last.get(series).map_or(l.cancelled, |d| is_cancelled(d));
        let exceptions: Vec<Event> = deltas
            .into_values()
            .filter(|d| !cancelled && l.exception_changed(d, job.exception))
            .cloned()
            .collect();
        if !exceptions.is_empty() {
            out.pulled(Effect::SetExceptions {
                path: l.path.clone(),
                exceptions,
            });
        }
    }

    if !push {
        return Vec::new();
    }
    for (index, l) in locals.iter().enumerate() {
        // A deleted event only if no live one holds its remote id.
        let held = |rid: &str| by_remote.get(rid) == Some(&index);
        let counted = !l.deleted || l.remote_event_id.as_deref().is_some_and(held);
        if counted && l.wants_push() && !matched.contains(&index) {
            pushes.push(Push {
                index,
                etag: l.etag.clone(),
                found: None,
            });
        }
    }
    pushes.sort_by_key(|p| p.index);
    pushes
}

// ----------------------------------------------------------------------------
/// Whether `event` is deleted in Google.
fn is_cancelled(event: &Event) -> bool {
    event.status.as_deref() == Some("cancelled")
}

// ----------------------------------------------------------------------------
// execution (blocking — the caller picks the thread)

/// Run the whole job: pull every linked calendar (listing it, unless its
/// changes were listed already), and push it unless the job is pull-only
/// (`SyncJob::push`). `Err` only if the account can't authorize at all
/// (`Error::Expired`: reconnect needed); anything narrower lands in
/// `report.failures`.
pub fn run_sync(client: &Client, cancel: &Cancel, mut job: SyncJob) -> Result<SyncOutput> {
    client.access_token(cancel)?;

    let mut out = SyncOutput::default();
    for mut cal in std::mem::take(&mut job.calendars) {
        let listed = cal.listed.take();
        if let Err(error) = sync_calendar(client, cancel, &job, &cal, listed, &mut out) {
            out.report.failures.push(Failure {
                subject: cal.remote_calendar_id.clone(),
                error,
            });
        }
    }
    Ok(out)
}

// ----------------------------------------------------------------------------
/// List `cal`'s changes in Google, writing nothing: the deltas since its
/// sync token, or every event from its stored time-min (else `time_min`)
/// on.
pub fn list_changes(
    client: &Client,
    cancel: &Cancel,
    cal: &CalendarJob,
    time_min: &str,
) -> Result<Changes> {
    let time_min = cal.stored_time_min.as_deref().unwrap_or(time_min);
    let (listed, full) = calendar::list_events(
        client,
        cancel,
        &cal.remote_calendar_id,
        cal.sync_token.as_deref(),
        time_min,
    )?;
    Ok(Changes {
        deltas: listed.items,
        next_sync_token: listed.next_sync_token,
        time_min: time_min.to_string(),
        full,
    })
}

// ----------------------------------------------------------------------------
fn sync_calendar(
    client: &Client,
    cancel: &Cancel,
    job: &SyncJob,
    cal: &CalendarJob,
    listed: Option<Changes>,
    out: &mut SyncOutput,
) -> Result<()> {
    // ---- pull ----
    let changes = match listed {
        Some(changes) => changes,
        None => list_changes(client, cancel, cal, &job.time_min)?,
    };
    let pushes = plan(cal, &changes.deltas, job, out);

    // ---- push ----
    for push in pushes {
        let l = &cal.events[push.index];
        match push_one(client, cancel, cal, l, &push, job.normalize) {
            Ok(PushOutcome::Synced {
                remote_event_id,
                snapshot,
                remote,
            }) => {
                out.report.pushed += 1;
                out.set_remote(cal, l, remote_event_id, snapshot, *remote);
            }
            Ok(PushOutcome::Conflict(remote)) => out.conflict(cal, l, Some(*remote)),
            Ok(PushOutcome::Deleted) => {
                out.report.deleted += 1;
                out.cancel_trashed(l, None);
            }
            Ok(PushOutcome::Gone) => out.cancel_trashed(l, None),
            Ok(PushOutcome::Back(remote)) => out.restore(cal, l, *remote, Vec::new()),
            Err(error) => out.report.failures.push(Failure {
                subject: l.label(),
                error,
            }),
        }
    }

    if let Some(token) = changes.next_sync_token {
        out.effects.push(Effect::SetSyncToken {
            calendar_path: cal.calendar_path.clone(),
            token,
            time_min: changes.time_min,
        });
    }
    Ok(())
}

// ----------------------------------------------------------------------------
/// What a push achieved (failures are `Err`, retried next run).
enum PushOutcome {
    /// Inserted, adopted, or patched: record the new bookkeeping, and the
    /// server's version the write left (`Effect::SetRemote::remote`).
    Synced {
        remote_event_id: String,
        snapshot: String,
        remote: Box<Event>,
    },
    /// The remote changed since our snapshot: the fetched remote version
    /// wins the file, the local one becomes the conflict sibling.
    Conflict(Box<Event>),
    /// Deleted in Google, as it was locally.
    Deleted,
    /// Already gone in Google.
    Gone,
    /// Changed in Google since it was deleted locally: it comes back as the
    /// fetched remote version.
    Back(Box<Event>),
}

// ----------------------------------------------------------------------------
/// Push `l`: insert it if it was never synced (nor its insert found),
/// delete it if it was deleted locally, else patch it -- assuming the
/// remote's etag (`Push`).
fn push_one(
    client: &Client,
    cancel: &Cancel,
    cal: &CalendarJob,
    l: &LocalEvent,
    push: &Push,
    normalize: fn(&Event) -> Event,
) -> Result<PushOutcome> {
    let calendar_id = cal.remote_calendar_id.as_str();
    let etag = push.etag.as_deref();
    let Some(rid) = l.remote_event_id.as_ref().or(push.found.as_ref()) else {
        let gid = google_id_for(&l.local_id);
        let mut body = l.payload.clone();
        body.id = Some(gid.clone());
        return match calendar::insert_event(client, cancel, calendar_id, &body)? {
            WriteOutcome::Written(created) => Ok(PushOutcome::Synced {
                remote_event_id: created.id.clone().unwrap_or(gid),
                snapshot: l.snapshot.clone(),
                remote: created,
            }),
            // Our id is taken: we created it in an earlier run and crashed
            // before the bookkeeping write. Adopt: fetch and record; if the
            // remote content drifted from the current local state, the next
            // run's change detection patches it (local wins — the remote
            // was born from an earlier local version).
            WriteOutcome::Conflict => {
                let got = calendar::get_event(client, cancel, calendar_id, &gid)?;
                Ok(PushOutcome::Synced {
                    snapshot: snapshot(&normalize(&got)),
                    remote_event_id: got.id.clone().unwrap_or(gid),
                    remote: Box::new(got),
                })
            }
        };
    };

    if l.deleted {
        return delete_one(client, cancel, cal, l, rid, etag);
    }

    // Moved locally to another linked calendar → move remotely first.
    let mut etag = etag.map(str::to_string);
    if let Some(old_calendar_id) = l
        .remote_calendar_id
        .as_deref()
        .filter(|&c| c != calendar_id)
    {
        etag = calendar::move_event(client, cancel, old_calendar_id, rid, calendar_id)?.etag;
    }
    let etag = etag.as_deref();
    match calendar::patch_event(client, cancel, calendar_id, rid, etag, &l.payload)? {
        WriteOutcome::Written(updated) => Ok(PushOutcome::Synced {
            remote_event_id: rid.clone(),
            snapshot: l.snapshot.clone(),
            remote: updated,
        }),
        WriteOutcome::Conflict => {
            // Remote changed since our snapshot: fetch the winner.
            let remote = calendar::get_event(client, cancel, calendar_id, rid)?;
            Ok(PushOutcome::Conflict(Box::new(remote)))
        }
    }
}

// ----------------------------------------------------------------------------
/// Delete `l`, deleted locally, in Google (in the calendar it was last
/// synced with), assuming the remote's `etag`. If the remote changed since,
/// it comes back as it is now -- unless it is gone there too.
fn delete_one(
    client: &Client,
    cancel: &Cancel,
    cal: &CalendarJob,
    l: &LocalEvent,
    rid: &str,
    etag: Option<&str>,
) -> Result<PushOutcome> {
    let calendar_id = l
        .remote_calendar_id
        .as_deref()
        .unwrap_or(&cal.remote_calendar_id);
    match calendar::delete_event(client, cancel, calendar_id, rid, etag)? {
        DeleteOutcome::Deleted => Ok(PushOutcome::Deleted),
        DeleteOutcome::Gone => Ok(PushOutcome::Gone),
        DeleteOutcome::Conflict => {
            let remote = calendar::get_event(client, cancel, calendar_id, rid)?;
            Ok(match is_cancelled(&remote) {
                true => PushOutcome::Gone,
                false => PushOutcome::Back(Box::new(remote)),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn local(
        id: &str,
        remote: Option<&str>,
        snapshot: &str,
        stored: Option<&str>,
        cancelled: bool,
    ) -> LocalEvent {
        LocalEvent {
            local_id: id.into(),
            path: PathBuf::from(format!("{id}.yaml")),
            payload: Event {
                summary: Some(id.into()),
                ..Event::default()
            },
            snapshot: snapshot.into(),
            stored_snapshot: stored.map(str::to_string),
            remote_event_id: remote.map(str::to_string),
            etag: Some("\"e1\"".into()),
            remote_calendar_id: Some("cal@remote".into()),
            cancelled,
            exceptions: Vec::new(),
            deleted: false,
        }
    }

    /// An event synced as `remote` (with snapshot `h1`), deleted locally.
    fn trashed(id: &str, remote: &str) -> LocalEvent {
        LocalEvent {
            path: PathBuf::from(format!(".trash/{id}.yaml")),
            deleted: true,
            ..local(id, Some(remote), "h1", Some("h1"), false)
        }
    }

    /// An instance of the series `series`, starting at `original` there.
    fn instance(series: &str, original: &str, summary: &str) -> Event {
        Event {
            id: Some(format!("{series}_{original}")),
            summary: Some(summary.into()),
            recurring_event_id: Some(series.into()),
            original_start_time: Some(calendar::EventTime {
                date: Some(original.into()),
                ..Default::default()
            }),
            ..delta(series, "confirmed")
        }
    }

    /// A local side that keeps of an instance whether it is cancelled, and
    /// its summary if it differs from the series'.
    fn kept(instance: &Event, series: &Event) -> Event {
        Event {
            original_start_time: instance.original_start_time.clone(),
            status: instance.status.clone().filter(|s| s == "cancelled"),
            summary: instance
                .summary
                .clone()
                .filter(|s| Some(s) != series.summary.as_ref()),
            ..Event::default()
        }
    }

    fn delta(rid: &str, status: &str) -> Event {
        Event {
            id: Some(rid.into()),
            etag: Some("\"e2\"".into()),
            status: Some(status.into()),
            summary: Some(format!("remote {rid}")),
            ..Event::default()
        }
    }

    fn job(events: Vec<LocalEvent>) -> CalendarJob {
        CalendarJob {
            calendar_path: PathBuf::from("Calendar"),
            local_calendar_id: "local-cal".into(),
            remote_calendar_id: "cal@remote".into(),
            sync_token: None,
            stored_time_min: None,
            events,
            listed: None,
        }
    }

    fn sync_job(cal: CalendarJob, push: bool) -> SyncJob {
        SyncJob {
            time_min: "2026-01-01T00:00:00Z".into(),
            push,
            normalize: Event::payload,
            exception: kept,
            calendars: vec![cal],
        }
    }

    fn listed(deltas: Vec<Event>, full: bool) -> Option<Changes> {
        Some(Changes {
            deltas,
            next_sync_token: Some("token".into()),
            time_min: "2026-01-01T00:00:00Z".into(),
            full,
        })
    }

    fn run_plan(cal: &CalendarJob, deltas: &[Event]) -> (SyncOutput, Vec<usize>) {
        let (out, pushes) = plan_with(cal, deltas, true, Event::payload);
        (out, pushes.into_iter().map(|p| p.index).collect())
    }

    fn plan_with(
        cal: &CalendarJob,
        deltas: &[Event],
        push: bool,
        normalize: fn(&Event) -> Event,
    ) -> (SyncOutput, Vec<Push>) {
        let sync = SyncJob {
            normalize,
            calendars: Vec::new(),
            ..sync_job(job(Vec::new()), push)
        };
        let mut out = SyncOutput::default();
        let pushes = plan(cal, deltas, &sync, &mut out);
        (out, pushes)
    }

    /// A local side that doesn't keep attendees.
    fn without_attendees(event: &Event) -> Event {
        Event {
            attendees: None,
            ..event.payload()
        }
    }

    fn attendee(email: &str) -> calendar::Attendee {
        calendar::Attendee {
            email: Some(email.into()),
            ..Default::default()
        }
    }

    #[test]
    fn plans_the_four_pull_cases_and_pushes() {
        let cal = job(vec![
            local("a", Some("ra"), "h1", Some("h1"), false), // unchanged + remote delta → overwrite
            local("b", Some("rb"), "h2", Some("old"), false), // changed + remote delta → conflict
            local("c", Some("rc"), "h3", Some("old"), false), // changed, no delta → push patch
            local("d", None, "h4", None, false),             // never synced → push insert
            local("e", None, "h5", None, true),              // local-only tombstone → nothing
            local("f", Some("rf"), "h6", Some("h6"), false), // unchanged, no delta → nothing
        ]);
        let deltas = vec![
            delta("ra", "confirmed"),
            delta("rb", "confirmed"),
            delta("rnew", "confirmed"),
        ];
        let (out, pushes) = run_plan(&cal, &deltas);

        assert_eq!(out.effects.len(), 3);
        for e in &out.effects {
            match e {
                Effect::CreateLocal { remote, .. } => {
                    assert_eq!(remote.id.as_deref(), Some("rnew"))
                }
                Effect::OverwriteLocal { path, .. } => assert_eq!(path, Path::new("a.yaml")),
                Effect::Conflict { path, remote, .. } => {
                    assert_eq!(path, Path::new("b.yaml"));
                    assert!(remote.is_some());
                }
                _ => panic!("unexpected effect"),
            }
        }
        assert_eq!(out.report.pulled, 2);
        assert_eq!(out.report.conflicts, vec!["b".to_string()]);
        assert_eq!(
            pushes,
            vec![2, 3],
            "changed-with-remote and never-synced push; tombstone and clean don't"
        );
    }

    #[test]
    fn remote_tombstones_cancel_or_conflict() {
        let cal = job(vec![
            local("a", Some("ra"), "h1", Some("h1"), false), // unchanged → cancel
            local("b", Some("rb"), "h2", Some("old"), false), // changed → conflict with remote=None
            local("c", Some("rc"), "h3", Some("h3"), true),  // already cancelled → refresh only
        ]);
        let deltas = vec![
            delta("ra", "cancelled"),
            delta("rb", "cancelled"),
            delta("rc", "cancelled"),
        ];
        let (out, pushes) = run_plan(&cal, &deltas);
        assert!(pushes.is_empty());

        assert_eq!(out.effects.len(), 3);
        for e in &out.effects {
            match e {
                Effect::CancelLocal { path, .. } => assert_eq!(path, Path::new("a.yaml")),
                Effect::Conflict { path, remote, .. } => {
                    assert_eq!(path, Path::new("b.yaml"));
                    assert!(remote.is_none(), "remote deleted → sibling + cancel");
                }
                Effect::SetRemote { path, etag, .. } => {
                    assert_eq!(path, Path::new("c.yaml"));
                    assert_eq!(etag.as_deref(), Some("\"e2\""));
                }
                _ => panic!("unexpected effect"),
            }
        }
    }

    #[test]
    fn equal_content_both_sides_refreshes_bookkeeping_only() {
        // The delta's payload hashes identically to the local content
        // (e.g. the echo of our own previous push).
        let remote = Event {
            id: Some("ra".into()),
            etag: Some("\"e9\"".into()),
            status: Some("confirmed".into()),
            summary: Some("same".into()),
            ..Event::default()
        };
        let mut l = local("a", Some("ra"), "", Some("stale"), false);
        l.snapshot = snapshot(&remote.payload());
        let (out, pushes) = run_plan(&job(vec![l]), &[remote]);
        assert!(
            pushes.is_empty(),
            "refresh consumes the local; no echo push"
        );
        let [Effect::SetRemote { remote, etag, .. }] = &out.effects[..] else {
            panic!("expected one SetRemote");
        };
        assert_eq!(etag.as_deref(), Some("\"e9\""));
        assert_eq!(
            remote.summary.as_deref(),
            Some("same"),
            "the delta is handed over"
        );
        assert_eq!(out.report.pulled, 0);
    }

    #[test]
    fn the_echo_of_our_push_never_conflicts_with_a_later_edit() {
        // Pushed with etag e2, edited locally since; the delta is our push.
        let mut l = local("a", Some("ra"), "edited", Some("pushed"), false);
        l.etag = Some("\"e2\"".into());
        let (out, pushes) = plan_with(
            &job(vec![l]),
            &[delta("ra", "confirmed")],
            true,
            Event::payload,
        );
        assert!(out.effects.is_empty() && out.report.conflicts.is_empty());
        assert_eq!(
            pushes,
            [Push {
                index: 0,
                etag: Some("\"e2\"".into()),
                found: None,
            }]
        );
    }

    #[test]
    fn a_refresh_that_records_nothing_new_is_left_out() {
        // Synced with etag e2; the delta is that version again (the echo
        // of our push, listed again while nothing changed).
        let synced = snapshot(&delta("ra", "confirmed").payload());
        let mut l = local("a", Some("ra"), &synced, Some(&synced), false);
        l.etag = Some("\"e2\"".into());
        let cal = job(vec![l]);
        let (out, pushes) = run_plan(&cal, &[delta("ra", "confirmed")]);
        assert!(out.effects.is_empty() && pushes.is_empty());

        // The same content under another etag (e.g. an attendee answered):
        // recorded.
        let mut answered = delta("ra", "confirmed");
        answered.etag = Some("\"e3\"".into());
        let (out, _) = run_plan(&cal, &[answered]);
        assert!(matches!(&out.effects[..], [Effect::SetRemote { .. }]));
    }

    #[test]
    fn work_is_a_full_list_a_delta_to_apply_or_a_push() {
        let synced = || {
            let mut l = local("a", Some("ra"), "h1", Some("h1"), false);
            l.etag = Some("\"e2\"".into());
            l
        };
        let with = |events: Vec<LocalEvent>, changes: Option<Changes>, push: bool| {
            let mut cal = job(events);
            cal.listed = changes;
            sync_job(cal, push).has_work()
        };
        assert!(with(vec![synced()], None, true), "not listed yet");
        assert!(!with(vec![synced()], listed(vec![], false), true));
        assert!(
            !with(
                vec![synced()],
                listed(vec![delta("ra", "confirmed")], false),
                true
            ),
            "only the echo"
        );
        assert!(with(vec![synced()], listed(vec![], true), true), "full");
        let new = listed(vec![delta("rnew", "confirmed")], false);
        assert!(with(vec![synced()], new, true), "a new event");

        let unsynced = local("d", None, "h4", None, false);
        assert!(with(vec![unsynced], listed(vec![], false), true), "a push");
        let unsynced = local("d", None, "h4", None, false);
        assert!(
            !with(vec![unsynced], listed(vec![], false), false),
            "pull only: the change waits"
        );
    }

    #[test]
    fn remote_changes_the_local_side_drops_are_not_conflicts() {
        let base = delta("ra", "confirmed");
        let mut answered = base.clone();
        answered.etag = Some("\"e3\"".into());
        answered.attendees = Some(vec![attendee("kim@example.com")]);
        let stored = snapshot(&without_attendees(&base));
        let l = local("a", Some("ra"), "edited", Some(&stored), false);

        // Two-way: the edit is pushed over the answered version.
        let cal = job(vec![l]);
        let (out, pushes) = plan_with(&cal, &[answered.clone()], true, without_attendees);
        assert!(out.effects.is_empty() && out.report.conflicts.is_empty());
        assert_eq!(
            pushes,
            [Push {
                index: 0,
                etag: Some("\"e3\"".into()),
                found: None,
            }]
        );

        // Pull only: the edit stays pending, the attendees still arrive.
        let (out, pushes) = plan_with(&cal, &[answered], false, without_attendees);
        assert!(pushes.is_empty());
        let [
            Effect::SetRemote {
                etag,
                snapshot,
                remote,
                ..
            },
        ] = &out.effects[..]
        else {
            panic!("expected one SetRemote");
        };
        assert_eq!(etag.as_deref(), Some("\"e3\""));
        assert_eq!(*snapshot, stored, "still changed locally");
        assert_eq!(remote.attendees.as_ref().map(Vec::len), Some(1));
    }

    #[test]
    fn pull_only_pushes_nothing() {
        let cal = job(vec![
            local("c", Some("rc"), "h3", Some("old"), false), // changed
            local("d", None, "h4", None, false),              // never synced
        ]);
        let (out, pushes) = plan_with(&cal, &[], false, Event::payload);
        assert!(pushes.is_empty() && out.effects.is_empty());
    }

    #[test]
    fn exceptions_skip_and_created_then_deleted_collapses() {
        // An instance of a series the vault doesn't have.
        let mut exception = delta("rx", "confirmed");
        exception.recurring_event_id = Some("master".into());
        let deltas = vec![
            exception,
            delta("rnew", "confirmed"),
            delta("rnew", "cancelled"), // later page cancels it again
        ];
        let (out, _) = run_plan(&job(vec![]), &deltas);
        assert_eq!(out.report.skipped_exceptions, 1);
        assert!(
            out.effects.is_empty(),
            "created-then-deleted within one window is a no-op"
        );
    }

    #[test]
    fn a_new_series_comes_with_its_instances() {
        let deltas = vec![
            delta("rs", "confirmed"),
            instance("rs", "2026-11-17", "moved"),
            delta("rc", "cancelled"),
            instance("rc", "2026-11-17", "of a deleted series"),
        ];
        let (out, _) = run_plan(&job(vec![]), &deltas);
        let [
            Effect::CreateLocal {
                remote, exceptions, ..
            },
        ] = &out.effects[..]
        else {
            panic!("expected one CreateLocal");
        };
        assert_eq!(remote.id.as_deref(), Some("rs"));
        assert_eq!(exceptions.len(), 1);
        assert_eq!(exceptions[0].summary.as_deref(), Some("moved"));
        assert_eq!(out.report.skipped_exceptions, 0);
    }

    #[test]
    fn instances_are_pulled_only_when_they_change_what_is_kept() {
        let mut series = local("a", Some("ra"), "h1", Some("h1"), false);
        let stored = |original: &str, summary: &str| {
            kept(&instance("ra", original, summary), &series.payload)
        };
        let (renamed, cancelled) = (stored("d1", "renamed"), {
            let mut x = stored("d2", "a");
            x.status = Some("cancelled".into());
            x
        });
        series.exceptions = vec![renamed, cancelled];
        let cal = job(vec![series]);

        let deltas = vec![
            instance("ra", "d1", "renamed"), // as kept
            instance("ra", "d2", "a"),       // restored: kept no more
            instance("ra", "d3", "a"),       // keeps nothing, none kept
            instance("ra", "d4", "x"),
            instance("ra", "d4", "a"), // a later page undoes it
            instance("ra", "d5", "new"),
        ];
        let (out, pushes) = run_plan(&cal, &deltas);
        assert!(pushes.is_empty());
        let [Effect::SetExceptions { path, exceptions }] = &out.effects[..] else {
            panic!("expected one SetExceptions");
        };
        assert_eq!(path, Path::new("a.yaml"));
        let ids: Vec<_> = exceptions
            .iter()
            .map(|x| x.id.as_deref().unwrap())
            .collect();
        assert_eq!(ids, ["ra_d2", "ra_d5"]);

        // Only what is kept already: no work.
        let mut cal = cal;
        cal.listed = listed(vec![deltas[0].clone(), deltas[2].clone()], false);
        assert!(!sync_job(cal, true).has_work());
    }

    #[test]
    fn instances_of_a_cancelled_series_are_left_out() {
        let deltas = [instance("ra", "d1", "x"), instance("rb", "d1", "x")];
        let cal = job(vec![
            local("a", Some("ra"), "h1", Some("h1"), true),
            local("b", Some("rb"), "h2", Some("h2"), false),
        ]);
        let mut with_b_cancelled = deltas.to_vec();
        with_b_cancelled.push(delta("rb", "cancelled"));
        let (out, _) = run_plan(&cal, &with_b_cancelled);
        assert!(
            out.effects
                .iter()
                .all(|e| !matches!(e, Effect::SetExceptions { .. }))
        );
        assert_eq!(out.report.skipped_exceptions, 0);
    }

    #[test]
    fn a_local_delete_is_pushed_only_two_way() {
        let cal = job(vec![trashed("a", "ra")]);
        let (out, pushes) = plan_with(&cal, &[], true, Event::payload);
        assert!(out.effects.is_empty());
        assert_eq!(
            pushes,
            [Push {
                index: 0,
                etag: Some("\"e1\"".into()),
                found: None,
            }],
            "with the stored etag"
        );
        let (out, pushes) = plan_with(&cal, &[], false, Event::payload);
        assert!(out.effects.is_empty() && pushes.is_empty());

        let with = |push: bool| {
            let mut cal = job(vec![trashed("a", "ra")]);
            cal.listed = listed(vec![], false);
            sync_job(cal, push).has_work()
        };
        assert!(with(true) && !with(false));

        let mut cancelled = trashed("b", "rb");
        cancelled.cancelled = true;
        let (out, pushes) = run_plan(&job(vec![cancelled]), &[]);
        assert!(
            out.effects.is_empty() && pushes.is_empty(),
            "deleted before"
        );
    }

    #[test]
    fn a_local_delete_follows_what_google_did_since() {
        let mut untouched = delta("rb", "confirmed");
        untouched.etag = Some("\"e1\"".into());
        let deltas = vec![
            delta("ra", "cancelled"),
            untouched,
            delta("rc", "confirmed"),
            instance("rc", "d1", "moved"),
        ];
        let cal = job(vec![
            trashed("a", "ra"),
            trashed("b", "rb"),
            trashed("c", "rc"),
        ]);
        let (out, pushes) = run_plan(&cal, &deltas);
        assert_eq!(pushes, [1], "untouched: deleted");
        let [
            Effect::CancelTrashed { path, etag },
            Effect::RestoreLocal {
                path: back,
                remote,
                exceptions,
                ..
            },
        ] = &out.effects[..]
        else {
            panic!("expected CancelTrashed, RestoreLocal");
        };
        assert_eq!(
            (path.as_path(), etag.as_deref()),
            (Path::new(".trash/a.yaml"), Some("\"e2\""))
        );
        assert_eq!(back, Path::new(".trash/c.yaml"));
        assert_eq!(remote.summary.as_deref(), Some("remote rc"));
        assert_eq!(exceptions.len(), 1, "with its instances");
    }

    #[test]
    fn a_live_event_wins_over_a_deleted_one_with_its_remote_id() {
        let live = local("b", Some("ra"), "h1", Some("h1"), false);
        let cal = job(vec![trashed("a", "ra"), live]);
        let (out, pushes) = run_plan(&cal, &[]);
        assert!(out.effects.is_empty() && pushes.is_empty());

        let (out, pushes) = run_plan(&cal, &[delta("ra", "confirmed")]);
        assert!(pushes.is_empty());
        let [Effect::OverwriteLocal { path, .. }] = &out.effects[..] else {
            panic!("expected one OverwriteLocal");
        };
        assert_eq!(path, Path::new("b.yaml"));
    }

    /// A run inserted an event and stopped before recording it: the next
    /// list finds the insert under the id made from the local id, and takes
    /// it as that event's own -- never as a new event.
    #[test]
    fn an_unrecorded_insert_is_matched_not_created_again() {
        const ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b51";
        let gid = google_id_for(ID);
        let listed = delta(&gid, "confirmed");
        let unsynced = || local(ID, None, "h1", None, false);

        // Equal: recorded under its id.
        let equal = LocalEvent {
            snapshot: snapshot(&listed.payload()),
            ..unsynced()
        };
        let (out, pushes) = run_plan(&job(vec![equal]), std::slice::from_ref(&listed));
        assert!(pushes.is_empty());
        let [
            Effect::SetRemote {
                remote_event_id, ..
            },
        ] = &out.effects[..]
        else {
            panic!("expected one SetRemote");
        };
        assert_eq!(*remote_event_id, gid);

        // Changed since: patched there, with the delta's etag.
        let cal = job(vec![unsynced()]);
        let (out, pushes) = plan_with(&cal, std::slice::from_ref(&listed), true, Event::payload);
        assert!(out.effects.is_empty() && out.report.conflicts.is_empty());
        assert_eq!(
            pushes,
            [Push {
                index: 0,
                etag: Some("\"e2\"".into()),
                found: Some(gid.clone()),
            }]
        );

        // Pull only: recorded, the change stays pending.
        let (out, pushes) = plan_with(&cal, std::slice::from_ref(&listed), false, Event::payload);
        assert!(pushes.is_empty());
        let [Effect::SetRemote { snapshot, .. }] = &out.effects[..] else {
            panic!("expected one SetRemote");
        };
        assert_eq!(snapshot, "", "not synced yet");

        // Deleted in Google meanwhile: the local version is kept.
        let (out, pushes) = run_plan(&cal, &[delta(&gid, "cancelled")]);
        assert!(pushes.is_empty());
        assert_eq!(out.report.conflicts.len(), 1);

        // A synced event holding that id keeps it; the unsynced one is new.
        let synced = local("b", Some(&gid), "h1", Some("h1"), false);
        let cal = job(vec![synced, unsynced()]);
        let (out, pushes) = run_plan(&cal, &[listed]);
        assert_eq!(pushes, [1]);
        let [Effect::OverwriteLocal { path, .. }] = &out.effects[..] else {
            panic!("expected one OverwriteLocal");
        };
        assert_eq!(path, Path::new("b.yaml"));
    }

    #[test]
    fn snapshot_is_stable() {
        // Stored in vault files: the value for a given payload must never change.
        let payload = Event {
            summary: Some("x".into()),
            ..Event::default()
        };
        assert_eq!(
            snapshot(&payload),
            format!("{:016x}", fnv1a_64(br#"{"summary":"x"}"#))
        );
    }
}
