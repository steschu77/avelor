# Avelor — User interface

> The main window: its layout, the vault panel with its filters, the
> lenses, the status bar, the jobs panel, page keys and the preferences.

## Layout

Three panels:

- the **vault panel** on the left, for filtering
  (`ui.md#vault-panel`); its lower area holds the jobs panel
  (`ui.md#jobs-panel`);
- the **main view** with the lenses (`ui.md#lenses`);
- the **side panel** on the right.

Both side panels have grips to resize them; the widths are remembered
across restarts (`ui.md#preferences`). Menus are popups (`showMenu`,
`ui/menu.js`), which mark their anchor `aria-expanded`.

## Vault panel <a id="vault-panel"></a>

The vault panel defines the view on the vault: a project filters
what the lens shows. Filtering is entirely on the page: all metadata
arrives with the scan (`renderSidebar`, `features/sidebar.js`;
`matchesFilter`, `model/entities.js`). The counts per project are derived on
the page, once per frame.

- **Projects**: "All Projects" clears the filter; "No project" shows the
  entities in no project of the vault (`NO_PROJECT` as
  `selected_project`); starred projects come first; archived projects are
  folded under an "Archived" row at the end, still selectable
  (`projectsInOrder`).

## Lenses <a id="lenses"></a>

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
  before the first fold, the archived projects are folded. Threads fold while
  the app runs: folded at first, a thread unfolded shows unfolded in every
  group.
- **Keyed rows** (by group, thread and entity) stay built while filtered
  out or folded, so a scan only adds rows.

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

- **Esc** closes an open menu.

## Preferences <a id="preferences"></a>

The page keeps its preferences as one JSON object of strings
(`savePrefs`, `model/state.js` → `set_prefs`): `sidebar_width`,
`panel_width` and `context_width`; the Documents lens's folded projects
(`doc_folded`, project ids joined by commas; project ids are unique
across vaults, so one list serves every vault). The host stores them as given
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
| B1 | Filter by project | pill or No project → `selectProject` → `selected_project` (`NO_PROJECT` for No project) | — | lens, counts re-render |
| F1 | Sort in Documents | header click (`onSortClick`, `features/lenses/documents.js`) | — | multi-key sort in `sort_state` |
| F6 | Fold in Documents | a group row or a thread's fold (`toggleGroup`, `toggleThread`); Open → `selected_project` | `set_prefs {prefs}` (`doc_folded`) | rows folded or shown; the projects' folds kept across restarts |
