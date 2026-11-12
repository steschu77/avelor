# Avelor — Calendar sync

> Google Calendar ⟷ vault: syncing a checked calendar, the rules of the
> two-way sync, the auto-sync and its sync points, how the sync is built. The account and its calendars are
> `accounts.md`; the event format, `vault.md#event` and `vault.md#remote`; the
> calendar lens, `ui.md#calendar`.

## Syncing a calendar <a id="syncing"></a>

Each calendar checked in the account pane is a source in the vault panel.
Its sync button syncs that calendar right away, as a job: the jobs panel
shows it and its summary (new, updated, pushed, deleted, restored,
conflicts, skipped occurrences, failed), and its Stop ends it early — what
it already did is kept, the rest waits for the next sync. One calendar
syncs at a time: a sync asked for while another runs is refused, with a
message in the status bar.

Checking a calendar creates its calendar entity in `Calendar/`
(`calendar::ensure_entities`), so an event can be moved into it before its
first sync; so do the account pane's list, for a checked one that has none,
and a sync. The check and the list create none while the vault is indexed:
one not read yet would be created a second time. Unchecking a calendar
keeps its entity.

The first sync fetches the events from a year back on; later syncs only
fetch what changed since (Google's sync token, `vault.md#remote`). Events
are written to a folder named after the calendar beside it, e.g.
`Calendar/Personal/`.

## Two-way sync <a id="two-way-sync"></a>

- With Calendar access Full, the sync is two-way: an event changed in the
  vault (title, body, time, location, status) is written back to Google,
  a field removed in the vault removed there too, and an event made in the
  vault in a synced calendar is created there.
  With Read access it only fetches. So it does for a calendar Google lets
  the account only see (its `access`, `accounts.md#google-file`), whatever
  the account's access. A change that would stay in the vault isn't made:
  such a calendar's events keep what they sync (title, body, times, zones,
  place, status) as Google has them, and aren't deleted in the vault, as
  Google's next change would bring them back; their labels, projects,
  links, a trip's fields and a work day's off or remote stay the vault's
  to change, and no new event goes into such a calendar. So does an event
  of a kind Google keeps as it is in any calendar, a birthday, an out of
  office, a working location or a focus time (its `event-type`,
  `vault.md#remote`, from Google's `eventType`); one changed in the vault
  anyway takes Google's version back at its push. An event Gmail made from
  a mail (a flight, a hotel stay), which Google lets no app change but
  lets one delete, stays editable, its card saying so: its push deletes
  Gmail's event and makes the vault's version in its place, an ordinary
  event from then on, no longer linked to the mail. A calendar not
  syncing now (unchecked, its account signed out) keeps its events
  editable: a change goes to Google once it syncs two-way again.
- An event changed on both sides since the last sync is a conflict: the
  vault's version is kept as a new event beside it, labelled `conflict`
  (without the recurring event's exceptions), and Google's version takes the file.
  With Full access, the next sync creates the kept version in Google, so
  neither is lost.
- An event deleted in Google is kept with `status: cancelled` and no
  longer shown in the calendar.
- An edit in the vault can be undone after the sync wrote it back
  (`saving.md#undo`); the next sync writes the undone version too. A
  delete it wrote back is not undone: the trashed file holds the
  cancelled event.
- Guests come from Google only and are never written back.
- An arrival in another zone than the start's keeps its own
  (`event.end-tz`), both ways. A trip's fields (`type`, `by`,
  `booking`, …, `vault.md#event`) and a work day's `work` stay in the
  vault; a pull keeps them.
- A recurring event is one entity; the calendar shows its occurrences. A single
  occurrence moved, renamed, given another place or deleted in Google is
  kept on the recurring event (`event.exceptions`) and shown that way; its card
  says where it was moved from. Its own description and guests aren't
  kept, and single occurrences changed in the vault aren't written back.
  Occurrences of a recurring event that isn't in the vault count as skipped.
- The body is written to Google as HTML and read back as markdown. Bold,
  italic, struck-out text, links and lists (also numbered and nested)
  survive the way back; headings, code and tables come back as plain text.
- With Full access, an event deleted in the vault is deleted in Google by
  the next sync (at once if the calendar syncs on its own). Its file in the
  trash is then marked cancelled, so restoring it brings it back hidden. If
  it changed in Google before that sync, it isn't deleted: it comes back
  from the trash with Google's version, keeping its labels, projects and
  links (or as a new event if the restore is refused). With Read access a
  delete stays in the vault, and the event comes back as a new one once it
  changes in Google. Emptying the trash before the sync means the event
  isn't deleted in Google.

## Auto-sync <a id="auto-sync"></a>

Each checked calendar in the account pane has an Auto-sync check. Ticked,
the calendar syncs on its own as an account's email does
(`email.md#email-auto-sync`): one calendar at a time, never beside another
sync; a sync from the sync button counts as a run.

- Each run first asks Google what changed, writing nothing. A run with
  nothing to fetch and nothing to write back leaves no trace; otherwise it
  is a "Syncing calendar · <name>" job like the sync button's.
- The calendar source's sync button is shown pressed while auto-sync is
  on; it still syncs right away.

## Sync points <a id="sync-points"></a>

An auto-synced calendar also syncs sooner:

- With Full access, an event changed in the app syncs it: its description
  once you leave edit mode (the Edit button again, Esc, or closing its
  card), anything else right away — an event made with New, an edit undone
  or redone (`saving.md#undo`), a change made outside the app that the app
  takes in, an edit saved outside edit mode.
  Deleting a synced event makes its calendar due too.
- While one of its events is in edit mode, the calendar's auto-sync waits,
  so a change made in Google never replaces text you are typing; it runs
  once you leave edit mode. An event left in edit mode isn't synced until
  you leave it — there is no cap, and the sync button doesn't wait.
- Switching back to the app syncs the calendars that haven't asked Google
  in the last 2 minutes, so a change just made in Google Calendar shows
  within moments.

## Flows

| # | Flow | Trigger → page | Host → events | Result |
|---|---|---|---|---|
| G1 | Sync a calendar | the source's sync button → `syncCalendar` (`features/sidebar.js`) | `calendar_sync {account, calendar}` → `ok {job}` (refused before any job; the first sync's start, a year back, is the host's) → plan on the UI thread, `run_sync` on a worker, effects applied on the UI thread → job events, `entity`, `content`, a `notice` per failed event, `account` | events in `Calendar/<name>/`, summary on the job card |

## Design <a id="design"></a>

```mermaid
sequenceDiagram
    participant P as page
    participant S as App
    participant W as worker
    participant G as Google
    P->>S: calendar_sync {account, calendar}
    S->>S: build SyncJob from the vault's events
    S-->>P: ok {job}
    S->>W: run_sync(job)
    W->>G: list changes (sync token), push local changes
    W-->>S: effects + report
    S->>S: apply effects (write/conflict/cancel), sync token last
    S-->>P: entity*, content*, notice*, account, job (done, summary)
```

- **Plan → run → apply** (`architecture.md#conventions`): `build_job`
  builds a `SyncJob` from the vault's events on the UI thread (the first
  sync's window from `first_sync_start`); the engine
  (`av-google/calendar_sync.rs`) runs on a worker and talks to Google
  only; its effects are applied on the UI thread through the vault edit,
  the sync token last. The engine's `plan` is pure and tested, and so is
  the mapping between Google's events and entities (`from_google`,
  `to_google`, `event_entity`, `exception_of`). The sync's state lives in
  `CalendarState`.
- **A push is a patch** (`patch_event`), which keeps what it doesn't
  name: it reads Google's version first and names, beside what the vault
  has, what Google has and the vault dropped: a title, a description or a
  place as `null`, a schedule as an empty one, a tentative status as
  confirmed; and where a date takes a time's place or the other way round,
  the other member as `null`, so an all-day event can become a timed one
  and back. A patch Google refuses is logged by its keys alone.
- **A move** (an event's `calendar` set to another calendar of the
  account) is pushed by the calendar it went to: it moves the event in
  Google (`move_event`), then what Google has decides (`after_move`), as a
  pull's delta would: nothing more if it holds the vault's version, a patch
  if only the vault changed it since, Google's version taken if only
  Google did, a conflict if both did. Until then the calendar it left
  leaves its changes alone (`CalendarJob::elsewhere`).
- **Remote versions are compared in one place**: the engine's two hooks,
  `SyncJob::normalize` (an event the way the vault would push it back) and
  `SyncJob::exception` (a recurring event's instance), both given by `calendar.rs`,
  are the only code that compares Google's version with the vault's. The
  bookkeeping keys (`etag`, `snapshot`, `sync-token`, …) are `vault.md#remote`.
- **Exceptions**: Google's instances are kept on the recurring event as
  `event.exceptions` (moved: start and end; cancelled; a title or place of
  their own), Google → vault only, outside change detection; through
  `SyncJob::exception` an instance that changes nothing kept is no work. A
  new recurring event brings its instances (`CreateLocal`), a known one gets
  `SetExceptions`; those of a recurring event not in the vault are skipped and
  counted. Attendees are likewise Google → vault only, outside change
  detection.
- **Descriptions**: Google → vault goes through
  `av_vault::html::to_markdown` (`body_of`), the email converter, so the
  lists and emphasis come back as the editor writes them.
- **Deletes**: with Full access, the calendar's trashed events that Google
  still has are deleted there (`delete_one`, with `If-Match`), and each
  trashed file is rewritten `cancelled` (`cancel_trashed`,
  `Index::edit_trashed`); one changed in Google since is restored from the
  trash with Google's version (`restore_event`).
- **Resuming**: a sync stopped mid-way (Stop, a close) resumes without
  duplicates. A change pushed but not recorded is matched as equal and
  recorded (`push_one`); an insert is idempotent (`google_id_for`): `plan`
  takes a listed event whose id is `google_id_for` of an unsynced local
  event as that event's own earlier insert — recorded if equal, patched if
  it changed since, never written to the vault a second time.
- **Auto-sync**: `calendar::auto_sync` runs from the clock
  (`architecture.md#the-clock`) and after the indexing, waiting for a
  running fetch or sync: a quiet `list_changes` first (`on_auto_listed`),
  and a job only if `SyncJob::has_work`. Its schedule (`SYNC_EVERY`,
  `AutoSyncs`) is shared with the email auto-sync in `google.rs`.
- **Sync points**: an event waiting to be pushed (`LocalEvent::wants_push`)
  makes its auto-synced, two-way calendar due — when it leaves edit mode
  (`DocumentState::is_editing`, `saving.md#edit-mode`), or at once
  when written by the app (`App::edit`), saved outside edit mode or
  taken in (`calendar::changed`); deleting a synced event makes it due
  (`waiting_delete`); the window active again makes due the calendars that
  last looked at Google 2+ minutes ago (`calendar::activated`).
