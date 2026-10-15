# Avelor — Saving and closing

> How the app closes without losing edits.

### Atomic writes <a id="atomic-writes"></a>

Every overwriting write — the config folder — goes through
`av_utils::fs::write_atomic`: a temporary `.{name}.tmp` beside the file,
flushed, then renamed over it. A file another program holds open without
delete sharing can't be replaced; that save fails visibly (its error in
the status bar).

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
