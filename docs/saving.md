# Avelor — Saving and closing

> How a file changed outside the app is handled, and how the app closes
> without losing edits.

## Conflicts <a id="conflicts"></a>

Another program may change a vault file while the app has it open. The app
never writes over such a change.

### The stamp <a id="the-stamp"></a>

Every document keeps the `FileStamp` (modified time, size) of the file as
it last read or wrote it, with what the file held then (`Document`'s
`OnDisk`) — taken before reading, so a change during the read shows up as
a conflict, never as a missed one. Before a write, the host compares it
with the file; a mismatch, or a missing file, is `SaveOutcome::Conflict`,
and nothing is written. A conflict is an expected outcome, not an error:
saves return `SaveOutcome::{Saved, Unchanged, Conflict}`.

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
