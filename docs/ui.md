# Avelor — User interface

> The main window: its layout, the vault panel, the lenses, the status
> bar, the jobs panel, page keys and the preferences.

## Layout

Three panels:

- the **vault panel** on the left; its lower area holds the jobs panel
  (`ui.md#jobs-panel`);
- the **main view** with the lenses (`ui.md#lenses`);
- the **side panel** on the right.

Both side panels have grips to resize them; the widths are remembered
across restarts (`ui.md#preferences`). Menus are popups (`showMenu`,
`ui/menu.js`), which mark their anchor `aria-expanded`.

## Lenses <a id="lenses"></a>

### Documents <a id="documents"></a>

`features/lenses/documents.js`: the entities as rows, in columns Name (the type
icon, the title) and Date, with a multi-key sort (`onSortClick`, kept in
`sort_state`; `kind` is its last tiebreak, no column sorts by it).

- **Keyed rows** (by entity) stay built, so a scan only adds rows.

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
`panel_width` and `context_width`. The host stores them as given
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
| F1 | Sort in Documents | header click (`onSortClick`, `features/lenses/documents.js`) | — | multi-key sort in `sort_state` |
