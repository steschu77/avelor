# Avelor — Host/WebView API

> The protocol between the Rust host and the WebView page: the transport,
> its rules, the page's side, and every command. A command's row
> says what it takes, answers and refuses.

## Transport <a id="transport"></a>

One string per message in each direction (`PostWebMessageAsString`,
`chrome.webview.postMessage`; on Linux `evaluate_javascript` and the script
message handler `host`, both behind the page's `window.host`,
`architecture.md#platforms`), each a JSON object: a request from the page,
a reply from the host (`app/src/protocol.rs`,
`app/static/core/rpc.js`).

The host decodes a message in `protocol::on_message` into a `Command` (one
enum variant per command) and routes it to its feature module's function,
which answers through a `Req`. 2 commands.

## Rules <a id="rules"></a>

- **Exactly one reply per request**: `ok` with the command's value, or
  `err` with a `code` and a `message`.
- **Two-step decode** (envelope first, then the command), so a bad payload
  still has an `id` to answer: a request whose command or `args` don't
  decode is answered `bad_request`. A message that isn't JSON, or has no
  numeric `id`, can't be answered; the host only logs it.
- **`Req` is a must-answer token.** It is consumed by `ok`/`err`/`fail`; if it is
  dropped unanswered (a forgotten branch) its `Drop` sends `{"err":
  {"code": "cancelled"}}` through the UI queue, so every promise on the
  page settles. `Req` is `Send` and holds only the `UiQueue`, never the
  outbox or the webview.
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
- The preferences reach the page inside it, not as a message
  (`ui.md#preferences`).

## The page side <a id="the-page-side"></a>

`rpc.js` is the page's only door to the host.

- `rpcCall(cmd, args)` sends a command and returns a promise of
  its one reply, which rejects with an `RpcError {cmd, code, message}`.

## Commands

### Vault commands

Host modules: `api.rs`.

| Command | Args | Reply and pushes |
|---|---|---|
| `ready` | — | The page is loaded and listens. `null`. |
| `set_prefs` | `{ prefs }` | Store the page's preferences, `{ key: value }` with string values, replacing the ones stored (`ui.md#preferences`). `null`, or the failed save ("Could not save the settings"); a value that isn't a string is `bad_request`. |
