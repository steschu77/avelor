# Avelor — Calendar sync

> Google Calendar ⟷ vault: syncing a checked calendar, the rules of the
> two-way sync and how the sync is built. The account and its calendars are
> `accounts.md`; the event format, `vault.md#event` and `vault.md#remote`; the
> calendar lens, `ui.md#calendar`.

## Syncing a calendar <a id="syncing"></a>

Each calendar checked in the account pane is a source in the vault panel.
Its sync button syncs that calendar right away, as a job: the jobs panel
shows it and its summary (new, updated, pushed,
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

## Two-way sync

- With Calendar access Full, the sync is two-way: an event changed in the
  vault (title, body, time, location, status) is written back to Google,
  a field removed in the vault removed there too, and an event made in the
  vault in a synced calendar is created there.
  With Read access it only fetches; changes made in the vault stay in the
  vault.
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
- A recurring event is one entity; the calendar shows its occurrences. A single
  occurrence moved, renamed, given another place or deleted in Google is
  kept on the recurring event (`event.exceptions`) and shown that way; its card
  says where it was moved from. Its own description and guests aren't
  kept, and single occurrences changed in the vault aren't written back.
  Occurrences of a recurring event that isn't in the vault count as skipped.
- The body is written to Google as HTML and read back as markdown. Bold,
  italic, struck-out text, links and lists (also numbered and nested)
  survive the way back; headings, code and tables come back as plain text.

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
- **Resuming**: a sync stopped mid-way (Stop, a close) resumes without
  duplicates. A change pushed but not recorded is matched as equal and
  recorded (`push_one`); an insert is idempotent (`google_id_for`): `plan`
  takes a listed event whose id is `google_id_for` of an unsynced local
  event as that event's own earlier insert — recorded if equal, patched if
  it changed since, never written to the vault a second time.
