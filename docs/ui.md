# Avelor — User interface

> The main window: the preferences.

## Preferences <a id="preferences"></a>

The page keeps its preferences as one JSON object of strings
(`set_prefs`). The host stores them as given
(`config.yaml` `prefs`) and never reads them; it fills them into the page
it loads on the next start, as JSON in
`<script type="application/json" id="prefs">` (`page::html`).
