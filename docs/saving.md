# Avelor — Saving and closing

> How a file changed outside the app is handled, and how the app closes
> without losing edits. The messages are in `api.md#documents`; how
> structural edits are saved, `panel.md#saving-structure`.

## Messages <a id="messages"></a>

`load_doc_content` carries a document's content
(`api.md#documents`); a body the host sends unasked (a file read
again) is the `content` event, handled the same way (`onDocContent`).
`reload_entity` is the conflict dialog's Use theirs; `close` and
`close_ready` are `saving.md#closing-the-app`.

## Conflicts <a id="conflicts"></a>

Another program may change a vault file while the app has it open. The app
never writes over such a change, and takes it in on its own only when that
loses nothing of the app's.

### When the app looks <a id="file-checks"></a>

Before every write; and when a document's content is loaded
(`load_doc_content`, opening a card).

What it does:

- No unsaved edits, and the app had only *read* the file: the change is
  taken in silently (`Index::refresh`) and logged.
- No unsaved edits, but the app had *written* its own edits to the file
  (`Entry::written`), which the change replaced — another editor saving
  its older copy: a conflict.

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

A `conflict` in the answer of `load_doc_content` (and of a structural
edit) opens the conflict dialog
(`features/conflict.js`): modal, over the shaded app,
and it has to be answered (Escape doesn't close it). It names the entity,
its file and when the file changed, or that it was deleted.

- **Use theirs** sends `reload_entity`: the file is taken in (or the
  entity leaves, if the file is gone).
- A refused structural edit only offers Use theirs; the user makes the edit
  again afterwards.
- Several conflicts wait in a queue and are shown one at a time.

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

## Closing the app <a id="closing-the-app"></a>

The window's Close doesn't destroy the window at once: the host asks
the page first (`close::on_close`; `close.rs`, `features/close.js`).

- **Refused** while a dialog or a handler runs.
- **The request.** The host sends the event `close {id}` on every Close.
  A second Close while it is asked asks nothing more.
- **What the page does.** The page answers `close_ready {id, ready}`.
- **No answer.** A page that doesn't answer within 4 s is hung or broken
  and can't save anything: the window closes anyway.
- The host closes the window through the UI queue
  (`UiQueue::close_window`, the window message `WM_CLOSE_NOW`; on Linux
  an idle callback), never from a worker.

### Windows ending the session <a id="session-end"></a>

When Windows ends the session (shutdown, sign-out), the page isn't asked.
Windows wants `WM_QUERYENDSESSION` answered at once, and WebView2 may have
ended its processes by then; the UI thread never waits for the page
(`architecture.md#principles`, principle 8). So the host answers yes.
Both messages are logged as they arrive. How to test it:
`development.md#session-end`.
