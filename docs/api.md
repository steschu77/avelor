# Avelor — Host/WebView API

> The protocol between the Rust host and the WebView page: the transport,
> its rules, the page's side, and every command and event. A command's row
> says what it takes, answers, pushes and refuses.

## Transport <a id="transport"></a>

One string per message in each direction (`PostWebMessageAsString`,
`chrome.webview.postMessage`; on Linux `evaluate_javascript` and the script
message handler `host`, both behind the page's `window.host`,
`architecture.md#platforms`), each a JSON object: a request from the page,
a reply or an event from the host (`app/src/protocol.rs`,
`app/static/core/rpc.js`).

The host decodes a message in `protocol::on_message` into a `Command` (one
enum variant per command) and routes it to its feature module's function,
which answers through a `Req`.

## Rules <a id="rules"></a>

- **Exactly one reply per request**: `ok` with the command's value, or
  `err` with a `code` and a `message`.
- **Two-step decode** (envelope first, then the command), so a bad payload
  still has an `id` to answer: a request whose command or `args` don't
  decode is answered `bad_request`. A message that isn't JSON, or has no
  numeric `id`, can't be answered; the host only logs it.
- **`Req` is a must-answer token.** It is consumed by `ok`/`err`/`fail`, or
  moved into a continuation; if it is dropped unanswered (an aborted task, a
  forgotten branch, the app closing) its `Drop` sends `{"err": {"code":
  "cancelled"}}` through the UI queue, so every promise on the page settles.
  `Req` is `Send` and holds only the `UiQueue`, never the outbox or the
  webview: an aborted task drops its `apply`, and the `Req` inside it, on
  the worker thread.
- **Events before replies**: the events a command causes are posted before
  its reply (`PostWebMessageAsString` keeps order), so the page's data is
  current when the reply arrives.
- **Error codes** are the names of `av_utils::Error`'s variants in
  snake_case (`Error::code()`): `bad_request`, `cancelled`, `file_write`,
  `win32`, `gtk`, …. `Req::fail` answers in the host's own words where
  the page shows the message.
- The page sends **strings** (`JSON.stringify`), since the host reads with
  `TryGetWebMessageAsString`.
- **Decoding is written by hand**: `Command::decode` matches on `cmd` and
  reads each command's own small args struct (`serde_json::from_value`).
  serde's derive for an adjacently tagged enum cost more, built once
  through its `Content` buffer and once through `Value`.
- **DTOs stay typed**: `App::post_event` writes the `{ev, data}`
  envelope as a struct with `serde_json::to_string`, and a DTO such as
  `EntityMeta` goes in as itself. Through `json!` it would pass a `Value`
  serializer, which cost more; `json!` is for small
  payloads (`{id}`, `{job}`).
- The preferences reach the page inside it, not as a message
  (`ui.md#preferences`).

## The page side <a id="the-page-side"></a>

`rpc.js` is the page's only door to the host.

- `rpcCall(cmd, args)` sends a command and returns a promise of
  its one reply, which rejects with an `RpcError {cmd, code, message}`.
- `rpcOn(event, fn)` listens for the pushes.
- A failed call nobody handles is shown in the status bar as an error that
  stays until dismissed, unless it is `cancelled`, so no
  failure is only a console line.

## Commands

### Vault commands

Host modules: `api.rs`, `vault_scan.rs`.

| Command | Args | Reply and pushes |
|---|---|---|
| `ready` | — | The page is loaded and listens. A `config.yaml` that couldn't be read at start is told first (`notice`). The host opens the vault of the last session (or the one the command line names), if there is one: the scan (`vault`, an `entity` per file, `vault_error`, the scan's `job` events; `vault.md#opening`). `null` once the scan has started. |
| `switch_vault` | `{ path? }` | Open the vault at `path`, or (without one, as the page sends it) the folder picked in a dialog owned by the main window. Remembered for the next start (`config.yaml` `vault`), and scanned as on `ready`. `ok {path}`, the vault being opened, once the switch is under way; `path` is null when the dialog was cancelled, which keeps the open vault. |
| `set_prefs` | `{ prefs }` | Store the page's preferences, `{ key: value }` with string values, replacing the ones stored (`ui.md#preferences`). `null`, or the failed save ("Could not save the settings"); a value that isn't a string is `bad_request`. |
| `reveal_vault` | — | Open the vault's folder in Explorer (on Linux the file manager) once the handler has returned. `null`; `invalid_path` ("No vault is open."), or the shell's error ("Could not show the vault's folder."). |

### Jobs and closing

Host module: `close.rs`.

| Command | Args | Reply and pushes |
|---|---|---|
| `close_ready` | `{ id, ready }` | The page's answer to the `close` event `id` (`saving.md#closing-the-app`): `ready` closes the window; not ready keeps it. An answer to an earlier close changes nothing. `null`. |

## Events <a id="events"></a>

What the host pushes; the page listens with `rpcOn`.

| Event | Data | Pushed when |
|---|---|---|
| `vault` | `{ state, name, path, total }` | A vault scan started, `state` `scanning` (on `ready`, after `switch_vault`), or read every file, `state` `open`: the vault's folder name and path, and the number of files found. Every entity comes in between as an `entity` event, with the scan's `job` events. A new scan starts the page's vault over. |
| `vault_error` | `{ path, error }` | A file of the scan couldn't be read; it stays out of the index. The scan's job ends with the count as its note. |
| `close` | `{ id }` | The window's Close (`saving.md#closing-the-app`). |
| `job` | `{ id, label, state: "running"\|"done", done, total, note, stoppable }` | A job started, made progress, or ended: a vault scan. `note` is an ended job's summary (what it did, what failed, or why it failed); a job that ended did all its steps (`done` = `total`). |
| `entity` | `{ id, kind, title, path, source, genre, date, projects, tags, linked_to, starred, archived, threads, editable, writable, deletable }` plus the role blocks, flattened | An entity as it is on disk now, one shape for every kind: once per entity during a vault scan. `kind` is the derived type (kebab-case, e.g. `bank-account`); `threads` is `message.threads`. A thread carries no member list: members are derived from their `threads`. |
| `notice` | `{ level: "error", message }` | `error`: the settings couldn't be saved where no command asked for it (the window's size, the vault remembered on a switch), or `config.yaml` couldn't be read at start (on `ready`). A notice without `level` counts as an error. The page shows them in the status bar (`ui.md#status-bar`). |
