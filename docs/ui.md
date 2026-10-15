# Avelor — User interface

> The main window: its layout, the status bar and the preferences.

## Layout

Three panels:

- the **vault panel** on the left;
- the **main view**;
- the **side panel** on the right.

Both side panels have grips to resize them; the widths are remembered
across restarts (`ui.md#preferences`).

## Status bar <a id="status-bar"></a>

The status bar is a snackbar (`ui/statusbar.js`): it floats over the
bottom of the main view and slides in only while there is something to
tell (the `notice` event, a failed call), the newest message with a count
of the others.

- An error (a failed save) stays until dismissed with ×.

## Preferences <a id="preferences"></a>

The page keeps its preferences as one JSON object of strings
(`savePrefs`, `model/state.js` → `set_prefs`): `sidebar_width`,
`panel_width`. The host stores them as given
(`config.yaml` `prefs`) and never reads them; it fills them into the page
it loads on the next start, as JSON in
`<script type="application/json" id="prefs">` (`page::html`), which
becomes `prefs` in the store, so the layout is right from the first frame.

## Flows

| # | Flow | Trigger → page | Host → events | Result |
|---|---|---|---|---|
| A5 | Panel widths | drag a grip → `saveWidth` → `savePrefs` | `set_prefs {prefs}` → `config.yaml` `prefs` | the width comes back on the next start, inside the page (`#prefs`), without easing |
