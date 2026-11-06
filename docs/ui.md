# Avelor — User interface

> The main window: its layout, the vault panel with its filters, the
> lenses, the status bar, the jobs panel, page keys, Go to, New and the
> preferences. The side panel is `panel.md`; the Workspace pages belong to
> their features (`accounts.md`, `import.md`, `vault.md`).

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
(`ui.md#preferences`). The right panel can also be half-open: a card's chat
button puts the assistant beside the card, over the main view, as the full
width does (`assistant.md#conversations`). Opening a panel never moves the
main view's layout. The expanded vault area hosts the pages of the vault
panel: the Workspace pages (Accounts, Folders, Vault, Trash), Settings
(`ui.md#settings`), Fetch email, Import scans and New. It is expanded only
while one of them is shown: a page opens it (`openVaultOp`, `ui/page.js`);
its ×, Esc on a page that registers it (`registerVaultPage`) or a click on a
filter closes it (`closeVaultArea`). Menus are popups (`showMenu`,
`ui/menu.js`), which mark their anchor `aria-expanded`.

## Vault panel <a id="vault-panel"></a>

The vault panel defines the view on the vault: words, a project and a
source filter what the lens shows, and so do the labels picked in the lens
head. Filtering is on the page, where all metadata arrives with the indexing
(`renderSidebar`, `features/sidebar.js`; `matchesFilter`,
`model/entities.js`); only the words are found by the host
(`ui.md#words`). The counts per project and source are derived on the
page, once per frame (`panel.md#host-pushes`), and follow no filter:
each the vault's entries of its project or source, All Projects and All
Sources every entry, as the lenses count them; the files are counted on
the Vault page alone.

- **Filter by words**: the box at the top (`ui.md#words`).

- **Go to**: under New, places, commands and keys by name (`ui.md#go-to`).

- **Marked** is a row while it narrows the list (`aria-current`): a
  project, No project, a source. "All Projects" and "All Sources", the
  default, never are, so a filter stands out.
- **Projects**: "All Projects" clears the project; "No project" shows the
  entities in no project of the vault (`NO_PROJECT` as
  `selected_project`); starred projects come first; archived projects are
  folded under an "Archived" row at the end, still selectable
  (`projectsInOrder`). Creating and editing projects, and dropping rows on
  them, is `panel.md#projects`.
- **Sources**: "Created Here" (with a "+" that opens New); each enabled
  folder watch under its name, a statements watch under its account's
  (its account's statements, whichever watch imported them), with its
  Import button (`import.md#folder-watches`); per
  connected Google account an Email source (unless its access is off), one
  source per checked calendar. A source
  counts and filters the entities that came through it (`sourceKeyOf`);
  its sync button syncs it: a calendar right away (`calendar.md`), Email
  opens Fetch email (`email.md`). While auto-sync
  is on, the sync button stays shown, pressed, in the count's place
  (`.nav-item__action[aria-pressed]`).
- **Workspace**: Accounts, Folders, Vault, Trash.
- **Help and settings**: at the foot, under what scrolls, a menu: the
  keyboard shortcuts (`ui.md#go-to`) and Settings (`ui.md#settings`).

**Labels** filter from the lens head, for every lens: after the project's
name the active labels, each a label chip whose × takes it out of the
filter (`labelChips`, `features/panel/panel.js`), then a "+" while any label
is in use (`#label-add-btn`, `features/header.js`). Its menu lists the
labels in use but the active ones, with a filter box, each with the
entities in the filter carrying it; one with none is dim. Selecting
several shows the entities carrying all of them. The words filtering come
first, as a chip with the words and a × that empties the box.

### Filter by words <a id="words"></a>

`features/words.js`: the box at the top of the vault panel, "Filter by
words", is a filter like a project or a source, which every lens follows.
It filters, it doesn't search: what holds every word stays in the lens,
in its own groups and order; the rest drops out. A type switched off
hides no hit from sight: the type bars show while the words filter, each
type with its hits counted, and Documents lists the hits of the types
switched off at its rows' end (`ui.md#documents`).

- **The words**: every one, in any order and case, also inside a longer
  word, in the title, a label or the text; the host finds them (`search`,
  `vault.md#search`). The box asks at most every 120 ms with the words as
  they are then, a newer question superseding the one before (the channel
  `search`); until the answer comes, the lenses keep the last one
  (`word_filter`). It asks again while the entities change, and when an
  editor saves a body (`saved_rev`), so an edit's new words count.
- **The box** is marked while it filters, with a × that empties it; Esc
  empties it, a second Esc leaves it; Ctrl+F goes to it, unless an editor
  or a field has the focus.
- **The rows** mark the words (`<mark>`, `ui/marks.js`); a row whose title
  lacks some says where they are: the line of the text or the label
  (`ui.md#documents`, `ui.md#timeline`).
- **The lens head** names them as a chip before the labels, its × empties
  the box (`features/header.js`).
- **The panel** marks them in its cards, and its find strip steps through
  them (`panel.md#find`).
- A vault switch empties the box; showing an entity in Documents
  (`showInDocuments`) does too if it doesn't hold the words.

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
shaded canvas, their days and tasks on card. A row, an event or a task
selects its entity (`selectDocument`), by click or Enter, and the
selected one is marked, every occurrence of a recurring event (`markSelection`).

- **Week** (full width): seven days from today, or the calendar week from
  Monday (the `week_start` preference, from the card's gear), with
  previous, next and Today; each day its open tasks due ("due") and its
  events with their time (`eventsBetween`, recurring events expanded); past days,
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
  tasks, calendars or conversations with an AI.
- **Recent conversations**: the four newest conversations with an AI, with their
  projects; + New conversation in the head starts one linked to nothing
  (`assistant.md#conversations`).

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
`sort_state`; `kind` is its last tiebreak, no column sorts by it). A
title is tagged read-only for an entity whose file can't be written and
for an event of a calendar read-only in Google (`calendar.md#two-way-sync`),
its tooltip saying which (`readOnlyTag`, `features/lenses/read-only.js`);
so is the Timeline's.

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
- **The words**: while the vault panel's words filter (`ui.md#words`),
  the titles mark them, and an entity's row whose title lacks some shows
  where they are after the title (`.docs__hit`): the line of its text, or
  the label marked "Label". The hits of the types switched off stand at
  the rows' end, each in no group or thread, under a head of their own
  ("In hidden types: 2 conversations and 1 contact", `.docs__found`) in
  place of what is hidden and Show all types.
- **Selecting**: a row is selected by a click, an entity's in the thread
  it shows under (`selectDocument(id, thread)`), a thread's row selects
  the thread (`selectById`). The selected entity's rows are marked
  (`markSelection`), and the rows of the thread the panel shows. A
  conversation with an AI selected opens in the assistant, beside what
  it is about (`assistant.md#conversations`).
- **Types**: the bar under the lens head (`#doc-types`; its code shared
  with the Timeline lens, `typeBar`, `features/lenses/types.js`), the
  lens's own filter — the vault panel's counts and the other lenses don't
  follow it: Documents (a `document` but a note, a `message`),
  Email, Notes (the type `note`), Conversations, Bank (bank accounts and
  statements), Lists, Records, Contacts, Tasks and Events (with
  calendars), each a toggle with its count in the vault panel's filter,
  faint at none. Every type but Events, which have a lens of
  their own, is shown at first. The lens head says how many entities are
  shown and hidden ("62 entries · 34 hidden"), and its Types button
  folds the bar, which shows whatever its fold while the words filter,
  Types gone meanwhile; the rows end with what is hidden and Show all
  types. The types switched off and the bar's fold are preferences.
- **Dragging**: an entity's row drags (with a drag preview chip) onto a
  project pill, a standalone card or the thread panel
  (`panel.md#structural-edits`), or onto a project's or a thread's row
  here, which it joins (`joinProjectLocal`, `joinThreadLocal`; the row
  highlighted while it is over it). The lens takes a card dragged out of
  a thread: on such a row it joins as well, anywhere else it leaves its
  thread. Thread and group rows don't drag.
- **Keyed rows** (by group, thread and entity) stay built while filtered
  out or folded, so an indexing only adds rows.
- **Shown on request** (`showInDocuments`, the `documents.show` action;
  the selection restored on start, `revealInDocuments`): the group and
  thread holding the entity's first row unfold and it is scrolled to.

### Timeline <a id="timeline"></a>

`features/lenses/timeline.js`:
the entities in the vault panel's filter with a date, each once on its
day — a recurring event on its first start, a task on its own date — but calendars
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
  (`summaryOf`) — an event's place, times and rule, an email's sender and
  time, a task's status and due day, a conversation's model and messages, a
  list's items, a contact's phone or email, a bank account's bank and
  number, statements' transactions, a document's author or sender and type
  — then its projects while no project is selected (`projectNames`), its
  labels at the right. While the words filter (`ui.md#words`), the title
  and the labels mark them, and a row whose title lacks some shows where
  they are instead of what its kind tells (`.tl__hit`). A click or Enter
  selects it, the selected one
  marked (`markSelection`). It drags as a Documents row does; the lens
  takes a card dragged out of a thread, which leaves it
  (`takeMemberDrops`, `panel.md#structural-edits`).
- **Kept parts**: rows by entity, day blocks by day, month headings by
  month, so an indexing only adds.

### Calendar <a id="calendar"></a>

`features/lenses/calendar.js`: a month grid with previous, next and
today; recurring events
expanded on the page from their RRULE and EXDATE lines, with Google's
exceptions (`occurrencesOf`, matched by the occurrence's day); cancelled
events hidden.

- Open tasks with a due date show on their due day, tagged "Due"; once that
  day has passed and a task is still open, its tag reads "Overdue",
  filled, and today's cell counts the overdue tasks (a click on the count
  shows them). Done tasks aren't shown.
- A click on an event or a task selects its day with its card focused; a
  click on a day selects the day. At midnight, today moves on.
- The selected entity's chips are marked, every occurrence of a recurring event
  (`markSelection`, `features/selection.js`, as on the Dashboard's Week
  card).
- The side panel shows the selected day (`panel.md#the-day-view`).

## Status bar <a id="status-bar"></a>

The status bar is a snackbar (`ui/statusbar.js`): it floats over the
bottom of the main view and slides in only while there is something to
tell (the `notice` event, a failed call), the newest message with a count
of the others.

- An error (a failed save, an email a fetch couldn't write) stays until
  dismissed with ×.
- A message about what an undo or a redo did, or what a job brought, goes
  on its own after 6 seconds, but not while the mouse is over it:
  "Undone: Renamed The Money Pit"; after a fetch or an auto-sync wrote new
  email, "3 new emails · jan@gmail.com", or for a single email "New email:
  <subject>", which a click shows in the Documents lens
  (`showInDocuments`, `features/lenses/documents.js`): the email selected,
  its thread unfolded and scrolled to, a filter that would hide it
  cleared and its type shown.

A new message is read out by a screen reader: a live region beside the
bar holds the newest one's text, there while the bar is hidden too.

The vault's state and the running jobs are on the Vault page and in the
jobs panel, not in the status bar.

## Jobs panel <a id="jobs-panel"></a>

A card per job from the `job` event (`features/jobs.js`, `renderJob`): its label,
its progress, a Stop button while it is `stoppable` (an email fetch, a calendar sync, the reading of scans or attachments, an email analysis; an indexing can't be stopped), and its summary when it ends. A card is removed 1.5 s after its job is done, 8 s
when it has a summary. How jobs run on the host: `architecture.md#jobs`.

## Keys and actions <a id="keys-and-actions"></a>

What a key, a menu, a button or Go to starts is a registered action
(`architecture.md#actions`). Go to lists the keys by where they work
(`ui.md#go-to`). The page keys, none of which acts behind an open dialog:

- **Ctrl+Shift+P** opens Go to (`ui.md#go-to`).
- **Ctrl+,** opens Settings (`ui.md#settings`).
- **Esc** closes an open menu, else the page in the expanded vault area
  (Settings, Fetch email, Import scans, New); in an editor it leaves edit
  mode.
- **Ctrl+Enter** runs the open page's main action (Fetch, Import all,
  Create).
- **Ctrl+N** opens New.
- **Ctrl+Z** undoes the last edit, **Ctrl+Y** or **Ctrl+Shift+Z** does it
  again, outside a field or an editor (`saving.md#undo`).
- **Ctrl+F** goes to the words box, unless an editor or a field has the
  focus; in the box, Esc empties it, then leaves it (`ui.md#words`).
- **F3** and **Shift+F3** step through the words marked in the panel
  (`panel.md#find`).
- **F6** and **Shift+F6** move between the areas: the lens tabs, the vault
  panel, the main view and the panel, onto what is current or selected
  there, else its first control.
- **Ctrl+1** to **Ctrl+5** show the lenses, in the tabs' order.
- A key the editor already handled (Ctrl+I as italic) is left alone.

Every control is in the keyboard's reach (`architecture.md#css`): **Tab**
goes from one to the next, a row or a day included, and **Enter** or
**Space** presses it, as a click would. A control shown on hover shows
while its row has the focus too. In a menu **Up** and **Down** move the
highlight and **Enter** runs it; a focused grip moves with **Left** and
**Right**. In an editor Tab indents where it can, in a list, a quote, a
table or code, and goes on elsewhere (`editor.md#keys`); Esc leaves edit
mode, back on the Edit button. Left to the mouse: joining or making a
thread by drag, the Dashboard's cards reordered, a scan's pages in any
order (a first or last page still moves to the next document by its
button), the scrubber's range of weeks (Clear and Today remain), the
editor's toolbar, and a chat message edited by clicking it.

The host switches off the browser's accelerator keys so they reach the page
(`sys/win32/webview.rs`).

**F11** enters and leaves fullscreen, as in a browser: the host takes it
before the page, and a held key's repeats are left out. Fullscreen isn't
kept: the next start opens the window as it was before. Go to has it too.

**Ctrl+Plus**, **Ctrl+Minus** and **Ctrl+0** zoom the page in, out and back
to 100%, in a browser's steps; so do Ctrl+wheel, a pinch and Go to. The host
takes the keys before the page: the browser's own went with its accelerator
keys.

## Go to <a id="go-to"></a>

Ctrl+Shift+P, or the Go to item under New, opens a dialog over the app that
finds places, commands and keys by name (`features/palette.js`, the blocks
of rows `features/palette-found.js`). With nothing typed it lists the
commands: the actions that have a group and apply now
(`architecture.md#actions`), under their groups: New, Assistant, Get things
in, Go to (the lenses, the Workspace pages, Settings, the next and the
previous area), Filter, Window, Vault, Settings (a row of the page each,
which it opens with the focus there) and Help. What the vault has brings its
own: an enabled folder watch its import, an account's Email its Fetch email,
a checked calendar its sync.

- **What is typed** is looked for in the names and in the other words one is
  found by: every word, in any order, case and accents aside (`fit`,
  `core/match.js`). The words are marked, and a row found by another word
  says which ("rescan", Rebuild index). What has a name is collected as the
  dialog opens (`collect`), each name folded once.
- **What is found** stands in blocks, the one with the best fit first, and
  of two alike in this order; within a block the best fit first
  (`foundBlocks`):
  - **Go to**: the places: the lenses, the Workspace pages and Settings,
    All Projects, No project, each project (an archived one after the
    others), All Sources and each source, with its entries counted. A
    project or a source is the filter then, as a press on its row in the
    vault panel, and the lens stays.
  - **Commands**: those that are no place.
  - **Keys**: those that are no command, at most two, by a word of theirs
    begun.
- **Narrowed** to the commands, only those are listed, by their groups: by a
  press on Commands beside the box, by `>` typed first, or with **Tab** and
  **Shift+Tab**, which go round what it can be narrowed to. **Backspace** in
  the empty box is back at All, and **Ctrl+Shift+P** too, the box emptied.
- **Narrowed to Keys**, by a press, Tab or its command Keyboard shortcuts,
  it lists the keys by where they work: anywhere, in the side panel, in a
  text being edited, in a menu or at a divider, in Go to itself
  (`KEY_GROUPS`, `features/keys.js`, with the commands'). One is found by
  what it does or by its keys, in the run's language or in English ("ctrl
  e"). Enter runs a command that applies now; of any other key the foot
  says where it works.
- **Its keys** stand at a command's end, each drawn as a key and named in
  the run's language (`shortcutKeys`, `core/actions.js`).
- **Up** and **Down** move the highlight, round the ends of the list, and
  **PageUp** and **PageDown** go over several rows; **Enter**, or a press
  on a row, acts on it, once the dialog is closed and the focus is
  back where it was. The foot says what Enter does. **Esc** or a
  press beside the dialog closes it.

## New

The New item, the "+" of "Created Here" and Ctrl+N open a menu of types
(`showNewMenu`, `features/new.js`): a document, task, event, contact, list or
record. The choice opens the New page in the expanded vault area
(`openNew`, op `new`); its projects start with the selected one, and its
projects and labels share one row with a card's "+" menu
(`panel.md#labels`). Create or
Ctrl+Enter (`createNew`) sends `create_entity`, which writes the entity
into its type's folder (`vault.md#vault-layout`); the area closes and
the new entity is selected. A failure stays on the page; Esc cancels. The
Task tab of the context column uses the same form (`panel.md#context-column`).

## Settings <a id="settings"></a>

How Avelor works for the user on this PC, not for the vault: the page of Help
and settings, of Ctrl+, and of Go to (`features/workspace/settings.js`). Its
list names the page's parts with what they are set to now, a press goes to
the part, and what is set on other pages, a press goes there; it stays while
the parts scroll. A row is a setting's name over what it does, its controls
at the end. A change takes effect at once, and one that waits for the next
start says so in the row. Another vault opening leaves the page open.

- **Look and language**, in every vault (`config.yaml`,
  `architecture.md#storage`):
  - **Language**: System, the language the system's take, or one of the
    catalogs', each by its own name (`localization.md#language`); beside it
    the form of address ("du" or "Sie"), where the language has both. From
    the next start.
  - **Theme**: System, Light or Dark, the page (`architecture.md#css`) and on
    Windows the title bar (`architecture.md#platforms`).

## Preferences <a id="preferences"></a>

The page keeps its preferences as one JSON object of strings
(`savePrefs`, `model/state.js` → `set_prefs`): `sidebar_width`,
`panel_width` and `context_width`; the lens shown last (`lens`); the
dashboard's layout (`dashboard_cards`, `dashboard_closed`, card ids
joined by commas) and the Week card's start (`week_start`: `today` or
`monday`); the Documents lens's types switched off
(`doc_hidden`, type ids joined by commas, so a type added since is
shown), its bar (`doc_types_bar`: `open` or `closed`) and its folded
projects (`doc_folded`, project ids joined by commas; project ids are
unique across vaults, so one list serves every vault); the Timeline lens's types
switched off and its bar (`tl_hidden`, `tl_types_bar`, as the Documents lens's);
the selection
(`entity`, `thread`: ids, empty for none), saved once the vault's indexing
has ended and selected again after the next one (`restoreSelection`,
`features/selection.js`), if it is still in the vault and nothing else
was selected meanwhile; a vault switch clears it. The host stores them as given
(`config.yaml` `prefs`) and never reads them; it fills them into the page
it loads on the next start, as JSON in
`<script type="application/json" id="prefs">` (`page::html`), which
becomes `prefs` in the store, so the layout is right from the first frame.
The half-open panel is `panel_width` plus `context_width`. The panel's mode
(`panel_size`: closed, normal, chat, expanded) isn't kept.

## Flows

| # | Flow | Trigger → page | Host → events | Result |
|---|---|---|---|---|
| A5 | Panel widths | drag a grip → `saveWidth` → `savePrefs` | `set_prefs {prefs}` → `config.yaml` `prefs` | the width comes back on the next start, inside the page (`#prefs`), without easing |
| A7 | Job progress | the `job` event → `renderJob` (`features/jobs.js`) | — | a card per job, removed 1.5 s after done (8 s with a summary) |
| A8 | Stop a job | the card's Stop | `cancel_job` → the job's requests cancelled; it ends `stopped · …` | email fetch, calendar sync |
| A9 | Language | Settings → the language or its form of address → `setLanguage` (`features/workspace/settings.js`) | `set_language {language, formal}` → `config.yaml` `language` | the row says what the next start shows (`localization.md#language`) |
| A10 | Theme | Settings → System, Light, Dark → `setTheme` | `set_theme {theme}` → `config.yaml` `theme`, the title bar (`UiSignal::theme`) | the page in the theme (`data-theme`) |
| B1 | Filter by project | pill or No project → `selectProject` → `selected_project` (`NO_PROJECT` for No project) | — | lens, header, counts re-render |
| B2 | Filter by source | source row → `selectSource(key)` | — | keys: `created`, `watch:<id>`, `<account>\nemail`, `<account>\ncalendar\n<id>` |
| B3 | Filter by labels | the lens head's "+" → its menu, a label picked; the × of a label in the lens head → `selectFilter` | — | entities carrying all selected labels; the head names them |
| B4 | Switch lens | topbar tab → `selectLens` (`features/selection.js`) | `set_prefs {prefs}` (`lens`) | visible lens rendered if stale; the calendar lens shows the day view in the panel; the next start shows it again (`restoreLens`) |
| B5 | The selection on start | `vault` (open) → `restoreSelection` → `selectDocument` or `selectById`, `revealInDocuments`; a selection changed → `savePrefs` | `set_prefs {prefs}` (`entity`, `thread`) | the entity or thread shown last selected again, its row unfolded |
| B6 | Filter by words | typing in the box (`features/words.js`) → `selectFilter({words})`, at most every 120 ms `search` on the channel `search`; the chip's ×, the box's × or Esc → `NO_WORDS` | `search {text}` → `ok {terms, hits}` | the lenses keep what holds every word, its words marked, an excerpt where the title lacks some; the chip in the lens head; the find strip in the panel |
| D7 | New | New item, the "+" of "Created Here" or Ctrl+N → `showNewMenu` → the New page (`openNew`, op `new`); Create or Ctrl+Enter → `createNew` | `create_entity {roles, title, …}` → `Index::create_new` (type, folder, format, `created: {}`) → `entity`, `ok {id}` | the area closes and the new entity is selected; a failure stays on the page; Esc cancels |
| F1 | Sort / drag in Documents | header click (`onSortClick`, `features/lenses/documents.js`); row drag | — | multi-key sort in `sort_state`; drag preview chip |
| F3 | Dashboard item | a Week event or task, a task card, a row of Upcoming, Recent files or Recent conversations → `selectDocument` | — | the entity in the panel, its rows marked on the dashboard |
| F4 | Dashboard layout | a card's ×, the Cards menu, Show all cards, a card's head dropped on another, Reset layout; the Week card's gear | `set_prefs {prefs}` (`dashboard_cards`, `dashboard_closed`, `week_start`) | the cards in their order, the closed ones hidden and counted on the Cards button; kept across restarts |
| F5 | Task dropped on a column | a task card of the Tasks card dragged onto another column (`dragTask`, `dropColumn`, `features/dashboard/cards.js`) → `editEntityLocal(id, {task: {status}})` | `edit_entities` → `entity` (with `task.done` for Done), `ok {changed, skipped}` | the task in its new column, Done first; the panel's card follows |
| F6 | Fold, select, types in Documents | a group row or a thread's fold (`toggleGroup`, `toggleThread`); Open → `selectFilter`; an entity's row → `selectDocument(id, thread)`, a thread's row → `selectById`; a type toggle, Show all types, Types | `set_prefs {prefs}` (`doc_folded`, `doc_hidden`, `doc_types_bar`) | rows folded or shown; the entity in its thread, or the thread, in the panel; the projects' folds and the types kept across restarts |
| F7 | Drop on a row in Documents | an entity's row or a card's head dropped on a project's row → `joinProjectLocal`, on a thread's row → `joinThreadLocal` | `join_project` / `join_thread` → `entity` | its rows under the project or the thread; joined to a thread, it is shown there |
| F8 | Timeline | a drag or click across the scrubber's track (`pointerdown` … `pointerup`, `features/lenses/timeline.js`); Clear; Today; a type toggle, Show all types, Types; a row → `selectDocument`; a card's head dropped on the lens → `leaveThreadLocal` | `set_prefs {prefs}` (`tl_hidden`, `tl_types_bar`); `leave_thread` | the feed kept to the weeks picked, or to all; scrolled to today; the types and the bar kept across restarts; the entity in the panel; the card out of its thread |
| F2 | Calendar month and day | prev/next/today (`features/lenses/calendar.js`); day click → `selectDay`; event or task click → `selectDay(day, id)`; today's overdue count → `selectDay(today)`, `scrollToOverdue` | — | the day's events as cards, then its tasks due (today: also the overdue ones) as rows; the clicked one focused, a task opened into its card |
