# Avelor — User interface

> The main window: its layout, the vault panel with its filters, the
> lenses, the status bar, the jobs panel, page keys, New and the
> preferences. The side panel is `panel.md`; the Workspace pages belong to
> their features (`accounts.md`, `vault.md`).

## Layout

Three panels:

- the **vault panel** on the left, for filtering
  (`ui.md#vault-panel`); its lower area holds the jobs panel
  (`ui.md#jobs-panel`);
- the **main view** with the lenses (`ui.md#lenses`), and above them the vault
  header (the selected project's name, description and toolbar,
  `panel.md#projects`);
- the **side panel** on the right, for the selected entity, thread or day
  (`panel.md`).

Both side panels can be extended to full width for focused work and have
grips to resize them; the widths are remembered across restarts
(`ui.md#preferences`). Opening a panel never moves the main view's
layout. The expanded vault area hosts the
pages of the vault panel: the Workspace pages (Accounts, Trash) and New. It is
expanded only while one of them is shown: a page opens it (`openVaultOp`,
`ui/page.js`); its ×, Esc on a page that registers it (`registerVaultPage`)
or a click on a filter closes it (`closeVaultArea`). Menus are popups
(`showMenu`, `ui/menu.js`), which mark their anchor `aria-expanded`.

## Vault panel <a id="vault-panel"></a>

The vault panel defines the view on the vault: a project filters
what the lens shows, and so do the labels picked in the lens head.
Filtering is entirely on the page: all metadata
arrives with the scan (`renderSidebar`, `features/sidebar.js`;
`matchesFilter`, `model/entities.js`). The counts per project are derived on
the page, once per frame (`panel.md#host-pushes`).

- **Projects**: "All Projects" clears the filter; "No project" shows the
  entities in no project of the vault (`NO_PROJECT` as
  `selected_project`); starred projects come first; archived projects are
  folded under an "Archived" row at the end, still selectable
  (`projectsInOrder`). Creating and editing projects, and dropping rows on
  them, is `panel.md#projects`.
- **Workspace**: Accounts, Trash.

**Labels** filter from the lens head, for every lens: after the project's
name the active labels, each a label chip whose × takes it out of the
filter (`tagChips`, `features/panel/panel.js`), then a "+" while any label
is in use (`#label-add-btn`, `features/header.js`). Its menu lists the
labels in use but the active ones, with a filter box, each with the
entities in the filter carrying it; one with none is dim. Selecting
several shows the entities carrying all of them.

## Lenses <a id="lenses"></a>

The main view's tabs are Dashboard, Documents, Timeline and Calendar; each
lens registers itself (`registerLens`, `features/lenses/registry.js`), a
tab selects it (`selectLens`), and the visible lens is rendered again if
it is stale. The lens shown last comes back on the next start
(`restoreLens`, the `lens` preference); one that no longer exists leaves
the Dashboard. There is no task kanban of its own: the Dashboard's Tasks
card is one.

### Dashboard <a id="dashboard"></a>

`features/dashboard/`: cards over the data the page has, each following
the vault panel's filter (`matchesFilter`), in a grid of six columns that
stacks as the lens narrows (container queries): paper lifted off the
shaded canvas, their days and tasks on white. A row, an event or a task
selects its entity (`selectDocument`), by click or Enter, and the
selected one is marked, every occurrence of a series (`markSelection`).

- **Week** (full width): seven days from today, or the calendar week from
  Monday (the `week_start` preference, from the card's gear), with
  previous, next and Today; each day its open tasks due ("due") and its
  events with their time (`eventsBetween`, series expanded); past days,
  today and weekends marked.
- **Tasks** (four of six columns): the entities with a task block, in the
  columns of `TASK_STATUSES` (To do, In progress, Waiting, Done; an
  unknown status under To do). The open ones by due date, an "Overdue"
  chip once it has passed, the projects named while none is selected;
  Done the last done first ("Done 11/10", `task.done`; one without a done
  date after them, the newest first), three of them, the rest behind "N
  more". The head counts the open and the overdue ones. A task the host
  can write drags to another column, which sets its status
  (`edit_entities`, as the card's Status buttons do,
  `panel.md#field-edits`): the card dims while dragged, the column it
  would go to is marked, and its own column takes no drop. The drag
  carries the task as a type only the columns take, and as text too, so
  a project pill in the vault panel takes it like an entity's row in
  the Documents lens.
- **Upcoming**: the next six events and open tasks due after the Week's
  seven days (from today while the Week card is closed), within a year.
- **Recent files**: the six newest dated entities that aren't events,
  tasks, calendars or chats with an AI.
- **Recent chats**: the four newest chats with an AI, with their
  projects.

The cards (`features/dashboard/cards.js`) are drawn by the lens
(`features/dashboard/dashboard.js`), again when the entities, the filter,
a project's name, the layout or the day change (`everyMidnight`). A
card's head drags it before or after another (its own drag type, which
nothing else takes); its × closes it. The lens header shows the date and
the Cards menu: each card, checked while shown, and Reset layout; the
button counts the closed cards. With every card closed, the lens says so
and offers Show all cards. The order and the closed cards are
preferences (`dashboard_cards`, `dashboard_closed`); an id they don't
know is dropped, a card they don't name joins at the end.

### Documents <a id="documents"></a>

`features/lenses/documents.js`: the entities in
the vault panel's filter as rows, in columns Name (the fold, the type
icon, the title) and Date, with a multi-key sort (`onSortClick`, kept in
`sort_state`; `kind` is its last tiebreak, no column sorts by it).

- **Groups**: under All Projects, a group row per project in the vault
  panel's order (`projectsInOrder`): its fold, name, count of entities
  and, on hover, Open, which makes its project the filter. An entity
  shows under each of its projects; empty groups aren't shown; a group
  row stays under the head while its rows scroll (the groups sit in
  `.docs__projects`). After the last project, a gap and a rule, then the
  entities in no project of the vault, without a group row. With a
  project or No project selected, the list is flat.
- **Threads**: within a group, a thread's members fold under the
  thread's row: its title, "5 of 6 · 14/08 – 23/09" (its members shown
  here of all of them, their days; "2 · …" when all are here), dated and
  sorted by its newest member here. A member in several threads shows
  under each. Unfolded, the members sort like the rows.
- **Folds**: a project's group folded or open is kept across restarts in
  the preferences (`doc_folded`), not in the vault (`vault.md#principles`);
  before the first fold, the archived projects are folded. A reveal
  (below) opens a folded group for the session only. Threads fold while
  the app runs: folded at first, a thread unfolded shows unfolded in every
  group.
- **Selecting**: a row is selected by a click, an entity's in the thread
  it shows under (`selectDocument(id, thread)`), a thread's row selects
  the thread (`selectById`). The selected entity's rows are marked
  (`markSelection`), and the rows of the thread the panel shows.
- **Types**: the bar under the lens head (`#doc-types`; its code shared
  with the Timeline lens, `typeBar`, `features/lenses/types.js`), the
  lens's own filter — the vault panel's counts and the other lenses don't
  follow it: Documents (a `document` but a note, a `message`),
  Mail, Notes (the genre `note`), Chats, Bank (bank accounts and
  transactions), Lists, Records, Contacts, Tasks and Events (with
  calendars), each a toggle with its count in the vault panel's filter,
  faint at none. Documents, Mail and Notes are shown at first. The lens
  head says how many entities are shown and hidden ("62 documents · 34
  hidden"), and its Types button folds the bar; the rows end with what is
  hidden and Show all types. The types shown and the bar's fold are
  preferences.
- **Dragging**: an entity's row drags (with a drag preview chip) onto a
  project pill, a standalone card or the thread panel
  (`panel.md#structural-edits`), or onto a project's or a thread's row
  here, which it joins (`joinProjectLocal`, `joinThreadLocal`; the row
  highlighted while it is over it). The lens takes a card dragged out of
  a thread: on such a row it joins as well, anywhere else it leaves its
  thread. Thread and group rows don't drag.
- **Keyed rows** (by group, thread and entity) stay built while filtered
  out or folded, so a scan only adds rows.
- **Shown on request** (the selection restored on start,
  `revealInDocuments`): the group and thread holding the entity's first
  row unfold and it is scrolled to.

### Timeline <a id="timeline"></a>

`features/lenses/timeline.js`:
the entities in the vault panel's filter with a date, each once on its
day — a series on its first start, a task on its own date — but calendars
(dated when they were connected) and cancelled events.

- **Types**: the Documents lens's bar (`#tl-types`, `typeBar`), with the
  lens's own selection, every type shown at first; each toggle counts its
  entries in the range. The feed ends with what the types hide and Show
  all types; the lens head's Types button folds this lens's bar.
- **The scrubber**: "Time · 12 entries" (the feed's), a bar per week from
  Monday over last year and this one, as high as its entries of the types
  shown against the busiest week's; the weeks to come lighter, a dashed
  line at today, the two years under it. A drag across it picks the weeks
  it passes as the range, a click one week; the range is named ("5–18 Oct
  2026") with Clear, the feed and the toggles keep to it, and it lasts
  while the app runs. Today scrolls the feed to today, dropping a range
  without it. Older and later entries are in the feed, not on the
  scrubber.
- **The feed**: newest first, a heading per month, a block per day with
  its number and weekday beside its entries (the date stays while they
  scroll), the latest first; the days to come dimmed, "Today · Tuesday 13
  October" between them and the past, while today is in the range. Once a
  vault is open, the feed starts at that line; at midnight, today moves
  on.
- **A row**: its type's icon, its title, what its kind tells
  (`summaryOf`) — an event's place, times and rule, a mail's sender and
  time, a task's status and due day, a chat's model and messages, a
  list's items, a contact's phone or mail, a bank account's bank and
  number, transactions' bookings, a document's author or sender and genre
  — then its projects while no project is selected (`projectNames`), its
  labels at the right. A click or Enter selects it, the selected one
  marked (`markSelection`). It drags as a Documents row does; the lens
  takes a card dragged out of a thread, which leaves it
  (`takeMemberDrops`, `panel.md#structural-edits`).
- **Kept parts**: rows by entity, day blocks by day, month headings by
  month, so a scan only adds.

### Calendar <a id="calendar"></a>

`features/lenses/calendar.js`: a month grid with previous, next and
today; series
expanded on the page from their RRULE and EXDATE lines, with Google's
exceptions (`occurrencesOf`, matched by the occurrence's day); cancelled
events hidden.

- Open tasks with a due date show on their due day, tagged "Due"; once that
  day has passed and a task is still open, its tag reads "Overdue",
  filled, and today's cell counts the overdue tasks (a click on the count
  shows them). Done tasks aren't shown.
- A click on an event or a task selects its day with its card focused; a
  click on a day selects the day. At midnight, today moves on.
- The selected entity's chips are marked, every occurrence of a series
  (`markSelection`, `features/selection.js`, as on the Dashboard's Week
  card).
- The side panel shows the selected day (`panel.md#the-day-view`).

## Status bar <a id="status-bar"></a>

The status bar is a snackbar (`ui/statusbar.js`): it floats over the
bottom of the main view and slides in only while there is something to
tell (the `notice` event, a failed call), the newest message with a count
of the others.

- An error (a failed save) stays until dismissed with ×.

## Jobs panel <a id="jobs-panel"></a>

A card per job from the `job` event (`features/jobs.js`, `renderJob`): its label,
its progress, and its note when it ends. A card is removed 1.5 s after its job is done, 8 s
when it has a note. How jobs run on the host: `architecture.md#jobs`.

## Keys and actions <a id="keys-and-actions"></a>

What a key, a menu or a button starts is a registered action
(`architecture.md#actions`). The page keys:

- **Esc** closes an open menu, else the page in the expanded vault area
  (New); in an editor it leaves edit mode.
- **Ctrl+Enter** runs the open page's main action (Create).
- **Ctrl+N** opens New.
- A key the editor already handled (Ctrl+I as italic) is left alone.

The host switches off the browser's accelerator keys so they reach the page
(`sys/win32/webview.rs`).

## New

The New item and Ctrl+N open a menu of types
(`showNewMenu`, `features/new.js`): a document, task, event, contact, list or
record. The choice opens the New page in the expanded vault area
(`openNew`, op `new`); its projects start with the selected one, and its
projects and labels share one row with a card's "+" menu
(`panel.md#labels`). Create or
Ctrl+Enter (`createNew`) sends `create_entity`, which writes the entity
into its type's folder (`vault.md#vault-layout`); the area closes and
the new entity is selected. A failure stays on the page; Esc cancels. The
Task tab of the context column uses the same form (`panel.md#context-column`).

## Preferences <a id="preferences"></a>

The page keeps its preferences as one JSON object of strings
(`savePrefs`, `model/state.js` → `set_prefs`): `sidebar_width`,
`panel_width` and `context_width`; the lens shown last (`lens`); the
dashboard's layout (`dashboard_cards`, `dashboard_closed`, card ids
joined by commas) and the Week card's start (`week_start`: `today` or
`monday`); the Documents lens's types shown
(`doc_types`, type ids joined by commas; an id the page doesn't know is
dropped), its bar (`doc_types_bar`: `open` or `closed`) and its folded
projects (`doc_folded`, project ids joined by commas; project ids are
unique across vaults, so one list serves every vault); the Timeline lens's types
shown and its bar (`tl_types`, `tl_types_bar`, as the Documents lens's);
the selection
(`entity`, `thread`: ids, empty for none), saved once the vault's scan
has ended and selected again after the next one (`restoreSelection`,
`features/selection.js`), if it is still in the vault and nothing else
was selected meanwhile; a vault switch clears it. The host stores them as given
(`config.yaml` `prefs`) and never reads them; it fills them into the page
it loads on the next start, as JSON in
`<script type="application/json" id="prefs">` (`page::html`), which
becomes `prefs` in the store, so the layout is right from the first frame.
The panel's mode (`panel_size`: closed, normal, expanded) isn't kept.

## Flows

| # | Flow | Trigger → page | Host → events | Result |
|---|---|---|---|---|
| A5 | Panel widths | drag a grip → `saveWidth` → `savePrefs` | `set_prefs {prefs}` → `config.yaml` `prefs` | the width comes back on the next start, inside the page (`#prefs`), without easing |
| A7 | Job progress | the `job` event → `renderJob` (`features/jobs.js`) | — | a card per job, removed 1.5 s after done (8 s with a note) |
| B1 | Filter by project | pill or No project → `selectProject` → `selected_project` (`NO_PROJECT` for No project) | — | lens, header, counts re-render |
| B3 | Filter by labels | the lens head's "+" → its menu, a label picked; the × of a label in the lens head → `selectFilter` | — | entities carrying all selected labels; the head names them |
| B4 | Switch lens | topbar tab → `selectLens` (`features/selection.js`) | `set_prefs {prefs}` (`lens`) | visible lens rendered if stale; the calendar lens shows the day view in the panel; the next start shows it again (`restoreLens`) |
| B5 | The selection on start | `vault` (open) → `restoreSelection` → `selectDocument` or `selectById`, `revealInDocuments`; a selection changed → `savePrefs` | `set_prefs {prefs}` (`entity`, `thread`) | the entity or thread shown last selected again, its row unfolded |
| D7 | New | New item or Ctrl+N → `showNewMenu` → the New page (`openNew`, op `new`); Create or Ctrl+Enter → `createNew` | `create_entity {roles, title, …}` → `Index::create_new` (type, folder, format, `created: {}`) → `entity`, `ok {id}` | the area closes and the new entity is selected; a failure stays on the page; Esc cancels |
| F1 | Sort / drag in Documents | header click (`onSortClick`, `features/lenses/documents.js`); row drag | — | multi-key sort in `sort_state`; drag preview chip |
| F3 | Dashboard item | a Week event or task, a task card, a row of Upcoming, Recent files or Recent chats → `selectDocument` | — | the entity in the panel, its rows marked on the dashboard |
| F4 | Dashboard layout | a card's ×, the Cards menu, Show all cards, a card's head dropped on another, Reset layout; the Week card's gear | `set_prefs {prefs}` (`dashboard_cards`, `dashboard_closed`, `week_start`) | the cards in their order, the closed ones hidden and counted on the Cards button; kept across restarts |
| F5 | Task dropped on a column | a task card of the Tasks card dragged onto another column (`dragTask`, `dropColumn`, `features/dashboard/cards.js`) → `editEntityLocal(id, {task: {status}})` | `edit_entities` → `entity` (with `task.done` for Done), `ok {changed, skipped}` | the task in its new column, Done first; the panel's card follows |
| F6 | Fold, select, types in Documents | a group row or a thread's fold (`toggleGroup`, `toggleThread`); Open → `selectFilter`; an entity's row → `selectDocument(id, thread)`, a thread's row → `selectById`; a type toggle, Show all types, Types | `set_prefs {prefs}` (`doc_folded`, `doc_types`, `doc_types_bar`) | rows folded or shown; the entity in its thread, or the thread, in the panel; the projects' folds and the types kept across restarts |
| F7 | Drop on a row in Documents | an entity's row or a card's head dropped on a project's row → `joinProjectLocal`, on a thread's row → `joinThreadLocal` | `join_project` / `join_thread` → `entity` | its rows under the project or the thread; joined to a thread, it is shown there |
| F8 | Timeline | a drag or click across the scrubber's track (`pointerdown` … `pointerup`, `features/lenses/timeline.js`); Clear; Today; a type toggle, Show all types, Types; a row → `selectDocument`; a card's head dropped on the lens → `leaveThreadLocal` | `set_prefs {prefs}` (`tl_types`, `tl_types_bar`); `leave_thread` | the feed kept to the weeks picked, or to all; scrolled to today; the types and the bar kept across restarts; the entity in the panel; the card out of its thread |
| F2 | Calendar month and day | prev/next/today (`features/lenses/calendar.js`); day click → `selectDay`; event or task click → `selectDay(day, id)`; today's overdue count → `selectDay(today)`, `scrollToOverdue` | — | the day's events as cards, then its tasks due (today: also the overdue ones) as rows; the clicked one focused, a task opened into its card |
