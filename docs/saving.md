# Avelor — Saving and closing

> When an edited document is saved, how the page knows a document has
> unsaved changes, what a delayed reply may and may not do, how a file
> changed outside the app is handled, and how the app closes without losing
> edits. The messages are in `api.md#documents`; how structural edits
> are saved, `panel.md#saving-structure`.

## Overview

```mermaid
sequenceDiagram
    participant U as user
    participant E as editor.js (DOM)
    participant A as model/documents.js
    participant H as host (Rust)

    A->>H: load_doc_content {id}
    H-->>A: ok {source, html}
    A->>E: setHTML(html)
    A->>A: baseline[id] = serializeMarkdown(root)

    U->>E: edits
    E->>A: onChange
    A->>A: editSeq[id]++ ; dirty = serialize(root) != baseline[id]

    Note over A: 1 s after typing pauses (at the latest 4 s), and on leaving edit mode
    A->>A: markdown = serializeMarkdown(root)
    alt markdown == baseline[id]
        A->>A: dirty = false (nothing to send)
    else
        A->>A: seq = editSeq[id]
        A->>H: save_doc_content {id, markdown}
        H->>H: write file if it differs; re-render
        H-->>A: ok {source, html}
        A->>A: baseline[id] = source ; recompute dirty
        A->>E: setHTML(html) — only if seq is current and not editing
    end
```

Everything document-shaped happens in `app/static/model/documents.js`;
the editor (`editor.js`) knows nothing about documents, markdown, or the
host, and the serializer (`markdown.js`) knows nothing about saving.
Edits are written straight to the vault file: there are no drafts
(`vault.md#versioning`).

## Triggers <a id="triggers"></a>

A save is attempted:

- when the card's Edit button, or the project description's pen in the
  vault header, is toggled off, or Escape is pressed;
- when a different document or project is selected while the current one
  is in edit mode — the outgoing one is saved first; also when a card is
  closed;
- for a document in edit mode, once its typing has paused for a second,
  and at the latest 4 seconds after its first unsaved edit while the typing
  goes on. It is a flat poll every 250 ms over the documents edited since
  their last autosave (`editedAt`, `unsavedSince` in
  `model/documents.js`): simple, and cheap, since a document nobody edits
  isn't looked at. The file is at most about a second behind the typing,
  which is all Windows ending the session can count on
  (`saving.md#session-end`).

"Attempted" because `trySave` first serializes the DOM and compares it to
the document's baseline (`saving.md#dirty-state`); only a difference produces a
message.

## Messages <a id="messages"></a>

`load_doc_content` and `save_doc_content` carry a document's content
(`api.md#documents`); a body the host sends unasked (a file read
again) is the `content` event, handled the same way (`onDocContent`).
`check_files` and the `activated` event are the look at the files when the
window becomes active again (`saving.md#conflicts`); `reload_entity` is the conflict dialog's Use theirs; `close` and
`close_ready` are `saving.md#closing-the-app`.

A failed save is answered with an error, shown in the status bar
(in the host's words, "Could not save the document.", `showError` in
`model/state.js`); the document stays dirty and the next poll retries.

On the host, `Vault::save_content` hands the markdown to
`vault::Index::save_content`, which writes only if the document's content
or frontmatter differ from what is on disk (`Document::is_dirty`), then
re-reads and re-renders (`Vault::content`) for the reply.

## Dirty state <a id="dirty-state"></a>

A document is **dirty** when its current content differs from what the
file holds. The webview decides this itself, by content, because it has
the serializer:

- **`baseline[doc_id]`** is the markdown the editor's content last
  matched on disk. It is set in two places: to `serializeMarkdown(root)`
  right after `setHTML()` loads a document into the editor, and to
  `source` from every save's answer.
- **`dirty[doc_id]`** is `serializeMarkdown(root) !== baseline[doc_id]`,
  recomputed once per animation frame after any edit (`onChange` from the
  editor), and again whenever the baseline changes (`dirty_docs`). The dot on the
  document's row and on its card's title follow it.
- Only a document currently loaded into *some* editor has a DOM to
  compare (`editorRoots[doc_id]`): every open document or thread member is
  its own card with its own editor (`panel.md#cards`), and the selected
  project's description under the vault header is one more editor root,
  keyed by the project's id. Any other document keeps the state its last
  save reply left it in: a reply covering its latest edit clears it; a
  failed save leaves it dirty.

Two consequences are the point of doing it this way:

- **Typing something and undoing it leaves the document clean**, and the
  next poll sends nothing. A counter-based scheme ("any edit happened")
  can't tell.
- **A file whose markdown is not in the serializer's canonical form is not
  dirty for being opened.** The baseline is the serialization of the *loaded
  DOM*, not the host's `source`: a file with `* item` bullets renders to the
  same DOM as `- item` and would compare unequal to its own source forever.
  Measured against the loaded DOM, it is clean until the user actually
  changes something — and only then is it rewritten, in canonical form
  (`markdown.md#round-trip`). "Unchanged documents are never rewritten"
  holds without the host having to know about canonical forms.

## The staleness guard <a id="staleness"></a>

`editSeq[doc_id]` is a per-document generation counter, bumped on every
edit. A save remembers the generation it covers with its call
(`saveDocContent`); the host never sees it. On the save's answer
(`onDocSaved`), the webview compares that value with the document's
*current* `editSeq`:

- **Equal** — the reply describes the latest edit. Its `html` may be shown
  (subject to `saving.md#live-document`).
- **Lower** — a newer edit or a newer save exists locally that this reply
  knows nothing about. Its `html` is *not* shown; its `source` still
  updates the baseline (it is what the file held as of that save, and the
  newer save's reply will update it again).

The race this closes: a save issued while an earlier save's reply is still
in flight would otherwise let that stale reply overwrite the editor's live
view with older content the moment the user returns to read-only preview.
Checking "is the document read-only right now" is not enough, because it
is read-only again by the time the stale reply lands.

Dropping a stale reply's `html` is safe because **the file on disk is
always correct regardless**: each save writes exactly the serialization
of the real last edit. A dropped reply only means the preview might not
show a cosmetic correction for a moment — never a mismatch between the
file and what was typed.

## The live document is never replaced while editing <a id="live-document"></a>

A save answer's `html` is applied via `editor.setHTML()`, which replaces
the whole DOM and therefore resets the caret. So while the document it
concerns is the one in the editor *and* edit mode is on, the reply updates
the baseline and the dirty state only; neither the editor nor the content
cache (`docContent`, whose subscription would also call `setHTML()`) is
touched. Once editing stops, the next save brings its own, then-safe
re-render if one is still needed.

Even when not editing, the re-render is applied only if it differs from
the editor's current HTML — usually it doesn't (`markdown.md#round-trip`).

## Conflicts <a id="conflicts"></a>

Another program may change a vault file while the app has it open. The app
never writes over such a change, and takes it in on its own only when that
loses nothing of the app's.

### When the app looks <a id="file-checks"></a>

Before every write; when a document's content is loaded
(`load_doc_content`, opening a card); and when the window becomes active
again (the `activated` event): then unsaved edits are saved at once, and
the host looks at the files of the other documents in an editor
(`check_files`).

What it does:

- Unsaved edits are a conflict (the save is refused).
- No unsaved edits, and the app had only *read* the file: the change is
  taken in silently (`Index::refresh`) and logged.
- No unsaved edits, but the app had *written* its own edits to the file
  (`Entry::written`), which the change replaced — another editor saving
  its older copy: a conflict, so Use ours can write the app's version back.

### The stamp <a id="the-stamp"></a>

Every document keeps the `FileStamp` (modified time, size) of the file as
it last read or wrote it, with what the file held then (`Document`'s
`OnDisk`) — taken before reading, so a change during the read shows up as
a conflict, never as a missed one. Before a write, the host compares it
with the file; a mismatch, or a missing file, is `SaveOutcome::Conflict`,
and nothing is written. A conflict is an expected outcome, not an error:
saves return `SaveOutcome::{Saved, Unchanged, Conflict}`, and the reply is
`{"ok": {"conflict": …}}`.

### The dialog <a id="the-dialog"></a>

A `conflict` in the answer of `load_doc_content`, `save_doc_content` or
`check_files` (and of a structural edit) opens the conflict dialog
(`features/conflict.js`): modal, over the shaded app,
and it has to be answered (Escape doesn't close it). It names the entity,
its file and when the file changed, or that it was deleted.

- **Use theirs** sends `reload_entity`: the edits made here are dropped
  and the file is taken in (or the entity leaves, if the file is gone).
- **Use ours** — only for a document's content — sends `save_doc_content`
  with `overwrite`: the version edited here is written over the file, or as
  the deleted file again, even when the host's copy didn't change.
- A refused structural edit only offers Use theirs; the user makes the edit
  again afterwards.
- Several conflicts wait in a queue and are shown one at a time.

### Held saves <a id="held-saves"></a>

- **On the page.** While a document's conflict waits, `trySave` sends
  nothing and keeps its latest markdown for Use ours — also once its card
  is gone. After Use theirs they stay held back until the file's content
  has arrived, so none of the old content can follow the reload and
  overwrite the file after all; the reload bumps `editSeq`, making any
  in-flight reply stale.
- **On the host.** The markdown of a save refused for a conflict, or of the
  app's saved edits another program wrote over (found by `check_files` or a
  load), is held in `DocumentState` (`documents.rs`) until a save of the document goes
  through or it is reloaded. If no one is left to answer the dialog —
  Windows ends the session, another vault opens — it is kept as a new
  entity beside the file, made here and tagged `conflict`
  (`documents::keep_conflict_copy`), so the edits survive and the file keeps
  the other program's version.

### Atomic writes <a id="atomic-writes"></a>

Every overwriting write — vault files and the config folder — goes through
`av_utils::fs::write_atomic`: a temporary `.{name}.tmp` beside the file,
flushed, then renamed over it; on Linux it takes the mode of the file it
replaces. A file another program holds open without
delete sharing can't be replaced; that save fails visibly (its error in
the status bar).

### Limits <a id="limits"></a>

- A change that keeps both the size and the modified time (FAT's 2-second
  resolution, tools that restore the time) isn't found.
- A file changed outside the app is found before a write, on load and when
  the window becomes active — not while the app stays in front, and not
  for files no editor shows.
- Typing in the very instant a taken-in file arrives (one round trip)
  loses those keystrokes to the file's version.

## Closing the app <a id="closing-the-app"></a>

The window's Close doesn't destroy the window at once: the host asks
the page first (`close::on_close`; `close.rs`, `features/close.js`).

- **Refused** while a dialog or a handler runs.
- **The request.** The host sends the event `close {id}` on every Close,
  also with nothing in edit mode — an edit held for the conflict dialog or
  a save on its way is the page's to know. A second Close while it is asked
  asks nothing more.
- **What the page does.** Every editor leaves edit mode — the card being
  edited, the project description — which saves it as the Edit button does
  (`saving.md#triggers`). Once every save sent is answered (`savesInFlight`,
  `model/documents.js`), the page answers `close_ready {id, ready}`.
- **Not ready.** A save answered as a conflict, or a conflict already
  waiting, keeps the window: the conflict dialog is on screen, and closing
  again once it is answered goes on. A save that failed otherwise doesn't
  hold the close; its error was shown in the status bar.
- **No answer.** A page that doesn't answer within 4 s is hung or broken
  and can't save anything: the window closes anyway.
- The host closes the window through the UI queue
  (`UiQueue::close_window`, the window message `WM_CLOSE_NOW`; on Linux
  an idle callback), never from a worker.

### Windows ending the session <a id="session-end"></a>

When Windows ends the session (shutdown, sign-out), the page isn't asked.
Windows wants `WM_QUERYENDSESSION` answered at once, and WebView2 may have
ended its processes by then; the UI thread never waits for the page
(`architecture.md#principles`, principle 8). So the host answers yes
and relies on what it knows: what the page saved is on
disk, at most about a second behind the typing (`saving.md#triggers`), and
at `WM_ENDSESSION` the app's version of a document still in conflict becomes
a conflict copy beside its file (`close::on_session_end`,
`saving.md#held-saves`). What was typed in the last second is lost. On
Linux the session manager's `query-end`, SIGTERM and SIGHUP do the same.
Both messages are logged as they arrive. How to test it:
`development.md#session-end`.

## Flows <a id="flows"></a>

| # | Flow | Trigger → page | Host → events | Result |
|---|---|---|---|---|
| D2 | Load content | `createMessageController` → `bindDocContent` (`model/documents.js`) → `docContent` cache | `load_doc_content {id}` → the file looked at (`Index::refresh`) → `ok {source, html}`, `ok {gone}` or with `conflict` | `setHTML`; baseline = serialization of the loaded DOM |
| D4 | Save | a pause of 1 s (at the latest 4 s), leaving edit mode, switching, closing a card → `trySave` | `save_doc_content {id, markdown}` → `Index::save_content` (writes only if changed, never over a file changed outside the app) → `ok {source, html}`, `ok {conflict}` (the dialog) or an error (the status bar) | baseline updated; HTML applied only if current and not editing |
| A6 | Close the app | the window's Close → `close::on_close` (`shell.rs`) | refused while a dialog or a handler runs; otherwise `close {id}`: every editor leaves edit mode, saving, and once the saves are answered the page answers `close_ready`; ready, the host closes the window (`WM_CLOSE_NOW`; on Linux an idle callback); a conflict keeps it; no answer in 4 s closes it anyway | nothing is synced before the window goes |
