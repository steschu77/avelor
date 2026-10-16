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
  snake_case (`Error::code()`): `bad_request`, `invalid_argument`,
  `cancelled`, `not_found` (no entity with that id), `wrong_kind` (the id
  names something of another kind than the command takes), `not_writable`,
  `file_not_found`, `file_write`, `file_exists`, `win32`, `gtk`, …. `Req::fail` answers in the host's own
  words where the page shows the message.
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

- `rpcCall(cmd, args, {channel})` sends a command and returns a promise of
  its one reply, which rejects with an `RpcError {cmd, code, message}`.
- Of the calls on one channel only the latest counts: the one before is
  rejected as `superseded` at once (a code the page adds), and its late
  reply is ignored.
- `rpcOn(event, fn)` listens for the pushes.
- A failed call nobody handles is shown in the status bar as an error that
  stays until dismissed, unless it is `superseded` or `cancelled`, so no
  failure is only a console line.

The structural commands go through `structureCall`
(`panel.md#structural-edits`).

## Commands

### Vault commands

Host modules: `api.rs`, `vault_scan.rs`.

| Command | Args | Reply and pushes |
|---|---|---|
| `ready` | — | The page is loaded and listens. A `config.yaml` that couldn't be read at start is told first (`notice`). The host opens the vault of the last session (or the one the command line names), if there is one: the scan (`vault`, an `entity` per file, `vault_error`, the scan's `job` events; `vault.md#opening`). `null` once the scan has started. |
| `switch_vault` | `{ path? }` | Open the vault at `path`, or (without one, as the page sends it) the folder picked in a dialog owned by the main window. Remembered for the next start (`config.yaml` `vault`), and scanned as on `ready`. `ok {path}`, the vault being opened, once the switch is under way; `path` is null when the dialog was cancelled, which keeps the open vault. The page sends its unsaved edits first. |
| `set_prefs` | `{ prefs }` | Store the page's preferences, `{ key: value }` with string values, replacing the ones stored (`ui.md#preferences`). `null`, or the failed save ("Could not save the settings"); a value that isn't a string is `bad_request`. |
| `reveal_vault` | — | Open the vault's folder in Explorer (on Linux the file manager) once the handler has returned. `null`; `invalid_path` ("No vault is open."), or the shell's error ("Could not show the vault's folder."). |
| `open_url` | `{ url }` | Open a link in its default app (the browser, the mail app). Only `http://`, `https://`, `mailto:` and `tel:` links, else `invalid_argument`: anything else could start a program or open a file. `null` once the shell has taken the link, or the shell's error ("Could not open …"). An `id:` link never reaches the host (`editor.md#links`). |

### Document commands <a id="documents"></a>

Host module: `documents.rs`. The page side is `saving.md`.

| Command | Args | Reply and pushes |
|---|---|---|
| `load_doc_content` | `{ id }` | A document's content for an editor, once its file was looked at (`saving.md#conflicts`). What was taken in is pushed first (`entity`, `content`, `entity_deleted`). `ok {source, html}`: the markdown body and its `to_html` rendering, `html` empty for a YAML record; with `conflict: {id, changed}` too if the app's own saved edits were written over; or `ok {gone: true}` if the entity left the vault (its file gone, or its `id` changed by hand), after `entity_deleted`. |
| `save_doc_content` | `{ id, markdown, overwrite? }` | Save an editor's markdown (`serializeMarkdown(root)`), written only if the content or the frontmatter differs from the file. `ok {source, html}` after every successful save, also when nothing changed: `source` is the body now on disk, `html` its rendering. `ok {conflict: {id, changed}}` if the file changed outside the app since it was last read or written, nothing written: `changed` is its modified time now (ISO 8601), empty if it was deleted. `overwrite: true` (Use ours) writes over the changed file, or as the deleted file again. The host holds a refused save (`saving.md#held-saves`). A failure in the host's words ("Could not save the document."). |
| `check_files` | `{ ids }` | The documents in an editor without unsaved edits, once the window is active again: changes made outside the app are taken in and pushed (`entity`, `content`, `entity_deleted`), then `ok {taken_in, conflicts: [{id, changed}]}`. `taken_in`: entities were taken in or removed, so the panel is rebuilt; each conflict replaced the app's own saved edits. |

### Structural commands <a id="structure"></a>

Host module: `structure.rs`. Each pushes the `entity` / `entity_deleted`
events of what it wrote, then answers `ok {id}` with the id below, or
`ok {conflict: {id, changed}}` when a file changed outside the app and
nothing was written, or an error: `not_writable` when an entity to rewrite
has no file, is read-only or has broken frontmatter, `not_found`,
`wrong_kind` (a project or thread that isn't one), …. The rules are
`panel.md#saving-structure`.

| Command | Args | Does | Answer `id` |
|---|---|---|---|
| `create_entity` | `{ roles, title, date?, projects?, tags?, linked_to?, content? }` | Make an entity: `roles` are its frontmatter blocks, e.g. `{"task": {"status": "todo", "due": "2026-10-09"}}`, and decide its type — a document, task, event, contact, list or record; anything else is `invalid_argument`, as are an empty title, an event without `start`, a non-project in `projects` and a `linked_to` that isn't in the vault. Written with `created: {}` into its type's folder (`vault.md#vault-layout`), a list or record as YAML without `content`; `date` is an event's start, else the one given, else now. | the new entity |
| `create_thread` | `{ title, members }` | A thread with these members, without projects of its own (`vault.md#thread`); its file goes beside its first member. | the new thread |
| `join_thread` | `{ id, thread }` | Make `id` a member of `thread`. | the thread |
| `leave_thread` | `{ id, thread }` | Remove `id` from `thread`; a thread left with fewer than two members dissolves: the other member leaves too, and the thread's file goes to `.trash`. | the thread |
| `create_project` | `{ title }` | A project titled `title`, as a markdown file in `Projects/`. | the new project |
| `join_project` | `{ id, project }` | Make `id` a member of the project; nothing if it already is. | the project |
| `leave_project` | `{ id, project }` | Remove `id` from the project. | the project |
| `set_title` | `{ id, title }` | Set the title of any entity; an empty title removes the `title` key. The file keeps its name. | the entity |
| `set_starred` | `{ id, starred }` | Star or unstar any entity; unstarring removes the key. | the entity |
| `set_archived` | `{ id, archived }` | Archive any entity, or take it out of the archive; unarchiving removes the key. | the entity |
| `set_tags` | `{ id, tags }` | Set the labels of any entity: each trimmed, empty ones dropped, a repeated one kept once; none removes the key. | the entity |
| `set_link` | `{ id, to }` | Set the `linked-to` of any entity to `to`, another entity in the vault (not `id`); `null` removes the key. | the entity |
| `edit_entities` | `{ ids, change }` | One change to every entity named, each written as its own vault edit: `change` is `{task: {status}}` (a task's); a status can't be empty (`invalid_argument`, nothing written). A status becoming `done` writes `task.done` (now), leaving it removes it. The answer is `ok {changed: [id], skipped: [{id, reason, changed?}]}` instead: what was written, and what wasn't, with the error code a single command would answer — `changed_outside` (with `changed`, as in a conflict), `not_writable`, `not_found`, or `invalid_argument` for a change the entity can't take (a task's field on a non-task). An entity the change leaves as it was is in neither list. | — |
| `delete_entity` | `{ id }` | Move `id`'s file to `.trash`; every thread it leaves with fewer than two members dissolves. A project's members (threads included) leave it first; if one of them is read-only or changed outside the app, nothing is written. Threads can't be deleted this way. | — |
| `reload_entity` | `{ id }` | Read the file again and take in what it holds (the conflict dialog's Use theirs): pushes `entity` and `content`, or `entity_deleted` if the file is gone. | the id the file has now; none if it is gone |

### Jobs and closing

Host module: `close.rs`.

| Command | Args | Reply and pushes |
|---|---|---|
| `close_ready` | `{ id, ready }` | The page's answer to the `close` event `id`, sent once every save it sent is answered (`saving.md#closing-the-app`): `ready` closes the window; not ready keeps it. An answer to an earlier close changes nothing. `null`. |

### Trash commands <a id="trash"></a>

Host module: `trash.rs`. `vault.md#trash`.

| Command | Args | Reply and pushes |
|---|---|---|
| `list_trash` | — | The files in `.trash/`, newest first: `ok {items: [{path, id?, kind, title, genre?, source?, date?, error?}]}`. `path` is relative to `.trash/` and names the file in the commands below; `id` is missing for a file without one, `error` says why a file couldn't be read, or what is wrong with its frontmatter. |
| `restore_entity` | `{ path }` | Move the trashed file back to where it was: pushes `entity` (a document, thread or project alike), then `ok {id}`. `file_exists` if a file is in that place or an entity with its `id` is in the vault, `invalid_path` for a path that leaves `.trash/`, `file_not_found` if it isn't there; each in the host's words ("Could not restore …"). |
| `delete_trashed` | `{ path }` | Delete the trashed file for good: `ok {}`; the errors as for `restore_entity` ("Could not delete … for good."). |
| `empty_trash` | — | Delete every file `list_trash` lists for good, and the folders this leaves empty: `ok {}`. |

## Events <a id="events"></a>

What the host pushes; the page listens with `rpcOn`.

| Event | Data | Pushed when |
|---|---|---|
| `vault` | `{ state, name, path, total }` | A vault scan started, `state` `scanning` (on `ready`, after `switch_vault`), or read every file, `state` `open`: the vault's folder name and path, and the number of files found. Every entity comes in between as an `entity` event, with the scan's `job` events. A new scan starts the page's vault over. |
| `vault_error` | `{ path, error }` | A file of the scan couldn't be read; it stays out of the index. The scan's job ends with the count as its note. |
| `activated` | `null` | The window is active again: the page saves its unsaved edits and looks at the files of the others (`check_files`). |
| `close` | `{ id }` | The window's Close (`saving.md#closing-the-app`). |
| `job` | `{ id, label, state: "running"\|"done", done, total, note, stoppable }` | A job started, made progress, or ended: a vault scan. `note` is an ended job's summary (what it did, what failed, or why it failed); a job that ended did all its steps (`done` = `total`). |
| `entity` | `{ id, kind, title, path, source, genre, date, projects, tags, linked_to, starred, archived, threads, editable, writable, deletable }` plus the role blocks, flattened | An entity as it is on disk now, one shape for every kind: once per entity during a vault scan, and for every entity an edit created or rewrote, whatever the edit (a structural command, a file read again). `kind` is the derived type (kebab-case, e.g. `bank-account`); `threads` is `message.threads`. A thread carries no member list: members are derived from their `threads`. |
| `entity_deleted` | `{ id }` | The entity left the vault: its file was moved to `.trash` (a delete, a dissolved thread) or is gone. |
| `content` | `{ id, source, html }` | The entity's body as it is now, sent unasked: its file was read again. `load_doc_content` answers with the same `source` and `html`. |
| `notice` | `{ level: "error", message }` | `error`: the settings couldn't be saved where no command asked for it (the window's size, the vault remembered on a switch), or `config.yaml` couldn't be read at start (on `ready`). A notice without `level` counts as an error. The page shows them in the status bar (`ui.md#status-bar`). |
