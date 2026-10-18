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

```text
page → host  {"id": 17, "cmd": "set_title", "args": {"id": "01a0…", "title": "Taxes"}}
host → page  {"re": 17, "ok": {…}}                          exactly one reply per request
             {"re": 17, "err": {"code": "not_writable", "message": "…"}}
             {"re": 17, "ok": {"job": "01a1…"}}             a long operation started
             {"ev": "entity", "data": {…}}                  a push
```

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
- **Long operations answer once they have started**: a job (mail fetch,
  calendar sync) with `{job}`, then reports through `job`
  events; `chat_send` once the user's message is written, its reply
  following as events.
- **Error codes** are the names of `av_utils::Error`'s variants in
  snake_case (`Error::code()`): `bad_request`, `invalid_argument`,
  `cancelled`, `not_found` (no entity, account, job or message with that id), `wrong_kind` (the id
  names something of another kind than the command takes), `not_writable`,
  `changed_outside`, `file_not_found`, `file_write`, `file_exists`, `spend_limit`, `win32`, `gtk`, `http`, …. `Req::fail` answers in the host's own
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
  reply is ignored. Channels in use: `connect_ai`, `mail-preview`,
  `chat-open`.
- `rpcOn(event, fn)` listens for the pushes.
- A failed call nobody handles is shown in the status bar as an error that
  stays until dismissed, unless it is `superseded` or `cancelled`, so no
  failure is only a console line.

The structural commands go through `structureCall`
(`panel.md#structural-edits`).

## Commands

### Vault commands

Host modules: `api.rs`, `vault_scan.rs`, `vault_settings.rs`.

| Command | Args | Reply and pushes |
|---|---|---|
| `ready` | — | The page is loaded and listens. A `config.yaml` that couldn't be read at start is told first (`notice`). The host opens the vault of the last session (or the one the command line names), if there is one (`vault.md#flows`, A1): its accounts and settings (`accounts`, `vault_settings`), then the scan (`vault`, an `entity` per file, `vault_error`, the scan's `job` events; `vault.md#opening`). `{google}` once the scan has started: whether this build has Google (`accounts.md#design`). |
| `switch_vault` | `{ path? }` | Open the vault at `path`, or (without one, as the page sends it) the folder picked in a dialog owned by the main window. Remembered for the next start (`config.yaml` `vault`), opened once the work bound to the open vault has stopped (`architecture.md#jobs`), and scanned as on `ready`. `ok {path}`, the vault being opened, once the switch is under way; `path` is null when the dialog was cancelled, which keeps the open vault. The page sends its unsaved edits first. |
| `set_vault_settings` | `{ chat_model? }` | Change the open vault's settings (`.avelor/vault.yaml`, `vault.md#vault-config`); only the fields given change. `chat_model` is `{account, model}`, the model the assistant answers with, checked when it is used (`accounts.md#model-choice`). Pushes `vault_settings`, then `null`; `invalid_path` (no vault is open), or the failed save ("Could not save the vault's settings."). |
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
| `set_editing` | `{ ids }` | The documents in edit mode now, all of them, sent whenever the list changes and after the save that ends an edit (`saving.md#edit-mode`). `null`. |

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
| `edit_entities` | `{ ids, change }` | One change to every entity named, each written as its own vault edit: `change` is `{genre}` (a document's), or `{task: {status}}`, `{task: {due}}` or `{task: {assignee}}` (a task's); a null or empty value removes the key, but a status can't be empty (`invalid_argument`, nothing written). A status becoming `done` writes `task.done` (now), leaving it removes it. The answer is `ok {changed: [id], skipped: [{id, reason, changed?}]}` instead: what was written, and what wasn't, with the error code a single command would answer — `changed_outside` (with `changed`, as in a conflict), `not_writable`, `not_found`, or `invalid_argument` for a change the entity can't take (a task's field on a non-task, a genre on a non-document). An entity the change leaves as it was is in neither list. | — |
| `delete_entity` | `{ id }` | Move `id`'s file to `.trash`; every thread it leaves with fewer than two members dissolves. A project's members (threads included) leave it first; if one of them is read-only or changed outside the app, nothing is written. Threads can't be deleted this way. | — |
| `reload_entity` | `{ id }` | Read the file again and take in what it holds (the conflict dialog's Use theirs): pushes `entity` and `content`, or `entity_deleted` if the file is gone. | the id the file has now; none if it is gone |

### Jobs and closing

Host module: `jobs.rs`, `close.rs`.

| Command | Args | Reply and pushes |
|---|---|---|
| `cancel_job` | `{ id }` | Stop the job `id`: its requests are cancelled and it ends through its `job` events — a mail fetch at once, writing nothing more; a calendar sync once it has recorded what it already did. `null` once the stop is under way; `not_found` for a job that isn't running, `unsupported` for a vault scan. |
| `close_ready` | `{ id, ready }` | The page's answer to the `close` event `id`, sent once every save it sent is answered (`saving.md#closing-the-app`): `ready` closes the window; not ready keeps it. An answer to an earlier close changes nothing. `null`. |

### Trash commands <a id="trash"></a>

Host module: `trash.rs`. `vault.md#trash`.

| Command | Args | Reply and pushes |
|---|---|---|
| `list_trash` | — | The files in `.trash/`, newest first: `ok {items: [{path, id?, kind, title, genre?, source?, date?, error?}]}`. `path` is relative to `.trash/` and names the file in the commands below; `id` is missing for a file without one, `error` says why a file couldn't be read, or what is wrong with its frontmatter. |
| `restore_entity` | `{ path }` | Move the trashed file back to where it was: pushes `entity` (a document, thread or project alike), then `ok {id}`. `file_exists` if a file is in that place or an entity with its `id` is in the vault, `invalid_path` for a path that leaves `.trash/`, `file_not_found` if it isn't there; each in the host's words ("Could not restore …"). |
| `delete_trashed` | `{ path }` | Delete the trashed file for good: `ok {}`; the errors as for `restore_entity` ("Could not delete … for good."). |
| `empty_trash` | — | Delete every file `list_trash` lists for good, and the folders this leaves empty: `ok {}`. |

### Google account commands <a id="google"></a>

Host module: `google.rs`, `accounts.rs` (both kinds). `accounts.md`.

| Command | Args | Reply and pushes |
|---|---|---|
| `connect_google` | `{ account?, levels }` | Connect a Google account (no `account`) or reconnect one, asking for `levels` (`{"email": "read", "calendar": "full", …}`, each `off`/`read`/`full`). The browser opens for the sign-in (loopback redirect, 5-minute limit); a new connect replaces the pending one, which answers `cancelled`, and so do `cancel_connect` and opening another vault. Reconnecting keeps the account's id and must sign in as the same email (`wrong_account`); a new connect of an email the vault has updates that account. Pushes `account`, then `ok {id}`. Else `timeout` or `denied` (the sign-in), `not_found` (no such account, or it was removed meanwhile), `invalid_path` (no vault is open to keep it in), `unsupported` (a build without the OAuth client, `accounts.md#design`). |
| `cancel_connect` | — | Abandon the pending sign-in, if there is one: `null`; the connect answers `cancelled`. |
| `list_calendars` | `{ account }` | The account's calendars: `ok {calendars: [{id, name, color, primary}]}`, the primary calendar first, then by name. Checked calendars take their current names and colors; if that changed any, `account` is pushed first. `not_found`, `not_connected`, or the request's error. |
| `set_calendar_source` | `{ account, calendar: {id, name, color}, selected }` | Check or uncheck a calendar as a source, stored in the account file. Pushes `account` (also when nothing changed), then `null`. |
| `disconnect_account` | `{ account }` | Either kind: a Google account's token is revoked at Google (best effort) and deleted, an AI account's key deleted; the account file stays, marked `disconnected` (`accounts.md#connection`). Pushes `account`, then `null`. |
| `remove_account` | `{ account }` | Forget an account that isn't connected (`disconnected` or `reconnect`; an Ollama account in any state): its file and secret are deleted. Pushes `account_removed`, then `null`; `unsupported` for a connected account. |

### AI account commands <a id="ai"></a>

Host module: `ai.rs`. `accounts.md`.

| Command | Args | Reply and pushes |
|---|---|---|
| `connect_ai` | `{ account?, provider?, key?, address? }` | Check and store a key or an address: for a new account of `provider` (`anthropic`, `openai`, `mistral`, `gemini`, `ollama`; no `account`), or for `account`; what is left out stays. The check lists the provider's models with the new values; only if it works is the account created or updated. Pushes `account`, then `ok {id, models}`, the models as for `list_ai_models`. A failed check answers its error (`http`, `network`, `timeout`, …) with a `message` worded for the page ("The key was rejected: …", "Ollama isn't reachable at … Is it running?"), with nothing stored. A newer `connect_ai` supersedes a running one, which answers `cancelled` (channel `connect_ai`), and so does opening another vault. Else `invalid_argument` (no provider for a new account, no key for a key provider), `not_found`, `invalid_path` (no vault is open to keep it in). |
| `list_ai_models` | `{ account }` | `ok {models: [{id, name, context_window, price: {input, output} \| null}]}`, the price in USD per million tokens (0 for a local Ollama, null if unknown). Pushes `account` first: an Ollama account's status follows the listing. A failed listing answers as a failed `connect_ai` check; `not_connected` for a key provider without its key, `not_found`. |
| `set_ai_settings` | `{ account, offered?, spend_limit? }` | Only the fields given change: `offered` replaces the models offered in the pickers, `spend_limit: null` removes the limit. Pushes `account`, then `null`; `invalid_argument` for a limit that isn't above 0, `not_found`. |

### Mail commands <a id="mail"></a>

Host module: `mail.rs`. `mail.md`. The errors of every `mail_*`:
`not_found` (no such account), `not_connected`, and Gmail's (`http`,
`network`, …) in the host's words ("Searching failed."), the error in the log.

| Command | Args | Reply and pushes |
|---|---|---|
| `mail_labels` | `{ account }` | The account's own Gmail labels: `ok {labels: [{ id, name }]}`, by name. |
| `mail_search` | `{ account, filter }` | `ok {matches: [{ id, in_vault }], truncated, headers}`: every match's Gmail id, newest first (at most 2000; `truncated` if there are more), `in_vault` if an entity from this account names it as `gmail-id`, and the first page's `headers` (as `mail_headers`). A newer `mail_search` or `mail_headers` supersedes a running one: its work is aborted and it answers `cancelled` (channel `mail-preview`). |
| `mail_headers` | `{ account, ids }` | The preview headers of up to 50 mails, a page: `ok {headers: [{ id, date, from, subject, in_vault }]}`, `in_vault` also by Message-ID from any source. Supersedes like `mail_search`. |
| `mail_fetch` | `{ account, ids, replace?, filter?, auto_update?, analysis? }` | Fetch the mails `ids` into `Mail/` as a job, `ok {job}` once it started; `filter`, `auto_update` and `analysis` (`{level: "off"\|"lite"\|"medium", model?: {account, model}}`) are remembered in the account first. With the analysis on, its pick is checked before anything is fetched (`accounts.md#model-choice`): refused, the fetch answers the refusal (`invalid_argument`, `not_connected`, `spend_limit`) and nothing is fetched; else the mails it created are analysed after it (`mail.md#analysis`). A mail already in the vault (by Gmail id from this account, else by Message-ID) is skipped — without a download if its Gmail id is known — or has its body replaced if its id is in `replace`. Every mail written arrives as `entity`, a replaced body also as `content`; each failed mail as a `notice`; at the end `account` (`last_sync`, and `mail.last_fetch` if all were fetched and none failed), then the job's end with the summary ("3 new · 1 skipped"). One fetch at a time: `unsupported` ("Another fetch is still running."). |
| `set_mail_sync` | `{ account, enabled?, filter?, analysis? }` | Change the account's mail auto-sync (`mail.md#mail-auto-sync`): `filter` replaces its filter, without dates (`since`, `after`, `before` are dropped); `analysis` replaces its analysis, as given (its pick is checked when a run has new mail); `enabled: true` starts it from now; `enabled: false` keeps the filter. Pushes `account` (its `mail.auto_sync`), then `null`; `not_found`, or the failed save ("Could not save the account."). |

### Calendar commands <a id="calendar"></a>

Host module: `calendar.rs`. `calendar.md`.

| Command | Args | Reply and pushes |
|---|---|---|
| `calendar_sync` | `{ account, calendar }` | Sync the checked calendar `calendar` (Google's id) as a job (`calendar.md#syncing`), `ok {job}` once it started. Every entity written arrives as `entity`, a changed body also as `content`; each failed event as a `notice`; at the end `account` (`last_sync`), then the job's end with the summary ("3 new · 1 updated") or why it failed. Stopped (`cancel_job`), it still records what it did. Refused before any job: `unsupported` while another sync runs ("Another calendar sync is still running.") or for an account without calendar access, `not_found` (no such account), `not_connected`, `invalid_argument` for a calendar that isn't one of the account's sources. |
| `set_calendar_sync` | `{ account, calendar, enabled }` | Switch the auto-sync of the account's checked calendar on or off (`calendar.md#auto-sync`), stored in the account file (`auto-sync: true`); switched on, the calendar is due at the clock's next tick. Pushes `account` (its `calendars[].auto_sync`), then `null`; `not_found`, `invalid_argument` for a calendar that isn't one of the account's sources, or the failed save ("Could not save the account."). |

### Chat commands <a id="chat"></a>

Host module: `chat.rs`. `assistant.md`.

| Command | Args | Reply and pushes |
|---|---|---|
| `chat_open` | `{ id }` | The conversation `id` as the assistant shows it: `ok {conversation}`, the `conversation` event's data; `not_found` if there is no such entity, `wrong_kind` if it is no conversation. The page calls it on the channel `chat-open`. |
| `chat_send` | `{ conversation?, text?, parent?, linked_to?, retry? }` | A user turn: `text` after the message `parent` (none: a first message, or an edited first message beside it), or a new reply to the user message `retry`. Without `conversation` it starts one in `Conversations/`, titled after `text` and linked to `linked_to`. The host checks the model picked for chats (the vault's settings, `accounts.md#model-choice`) and builds the context (`assistant.md#context`). The user message is written first (`entity`), then `ok {id, conversation}` shows the reply being written; `ai_delta` streams it, `conversation` ends it, and `account` (its spending) follows. Refused in the host's words, with nothing written: `not_connected` ("For chats, add an AI provider under Accounts.", or the picked model's key is missing), `invalid_argument` ("For chats, pick a model.", its account gone, the model no longer offered; "The message is empty."), `spend_limit` ("OpenAI has reached its spend limit of $10.00 this month; raise it under Accounts." — no other model is tried), `not_found` (the conversation, the message it follows, or the one to answer again is gone), `wrong_kind` (the one to answer again is a reply), `invalid_argument` ("The message it follows is gone." for a new conversation's first message that follows another; `retry` without `conversation`), `unsupported` ("A reply is still being written."), or the failed write ("Could not save the conversation."). |
| `chat_stop` | `{ conversation }` | Stop the reply being written; what it wrote so far is written as the reply (`status: "stopped"` until the app closes). Pushes `conversation`, then `null`; also `null` when no reply is being written. Opening another vault stops every reply the same way first. |
| `chat_select` | `{ conversation, message }` | Show another branch: the leaf below `message` (following the newest child; a failed reply stands for its question) is written as the conversation's `active` (`vault.md#conversation`). Pushes `conversation`, then `null`; `not_found`, `wrong_kind` (`conversation` is no conversation), or the failed write ("Could not save the branch."). |

## Events <a id="events"></a>

What the host pushes; the page listens with `rpcOn`.

| Event | Data | Pushed when |
|---|---|---|
| `vault` | `{ state, name, path, total }` | A vault scan started, `state` `scanning` (on `ready`, after `switch_vault`), or read every file, `state` `open`: the vault's folder name and path, and the number of files found. Every entity comes in between as an `entity` event, with the scan's `job` events. A new scan starts the page's vault over. |
| `vault_settings` | `{ chat_model }` | A vault opened (before its scan), or its settings changed (`set_vault_settings`): `chat_model` is `{account, model}` or null. |
| `vault_error` | `{ path, error }` | A file of the scan couldn't be read; it stays out of the index. The scan's job ends with the count as its note. |
| `activated` | `null` | The window is active again: the page saves its unsaved edits and looks at the files of the others (`check_files`). |
| `close` | `{ id }` | The window's Close (`saving.md#closing-the-app`). |
| `job` | `{ id, label, state: "running"\|"stopping"\|"done", done, total, note, stoppable }` | A job started, made progress, is stopping, or ended: a vault scan, a mail fetch, a calendar sync, a mail analysis ("Analysing mail · <email>": one step per mail; the note "18 analysed · 4 chats · 1 unchanged · 1 failed", "… · spend limit reached"; its failures in one `notice` at the end, a refused pick in one `notice` and no job). `note` is an ended job's summary (what it did, what failed, or why it failed); a job that ended without a stop did all its steps (`done` = `total`). |
| `entity` | `{ id, kind, title, path, source, genre, date, projects, tags, linked_to, starred, archived, threads, editable, writable, deletable }` plus the role blocks, flattened | An entity as it is on disk now, one shape for every kind: once per entity during a vault scan, and for every entity an edit created or rewrote, whatever the edit (a structural command, a mail fetch, a calendar sync, the assistant, a file read again). `kind` is the derived type (kebab-case, e.g. `bank-account`); `threads` is `message.threads`; a mail's `email` block carries `subject` and `kind` once the mail analysis wrote them (`vault.md#email`). A thread carries no member list: members are derived from their `threads`. |
| `entity_deleted` | `{ id }` | The entity left the vault: its file was moved to `.trash` (a delete, a dissolved thread) or is gone. |
| `content` | `{ id, source, html }` | The entity's body as it is now, sent unasked: an edit rewrote it (a sync, a mail fetched again), or its file was read again. `load_doc_content` answers with the same `source` and `html`. |
| `accounts` | `{ items: [account…] }` | A vault opened: its accounts, from `.avelor/accounts/`, replacing the ones the page has. Each item is an `account`'s data. |
| `account` | Google: `{ id, kind: "google", email, scope: { email, calendar, contacts, tasks }, connected, last_sync, status, calendars: [{ id, name, color, auto_sync }], mail: { filter, auto_update, last_fetch, analysis, auto_sync: { enabled, filter, since, analysis } \| null } \| null }`; AI: `{ id, kind: "anthropic"\|"openai"\|"mistral"\|"gemini"\|"ollama", key, address, status, connected, offered, spend_limit, spent: { cost, calls, unpriced } }` | An account was connected, changed, used or disconnected. Each `scope` level is `off`/`read`/`full`; `status` is `connected`, `disconnected` or `reconnect`, an Ollama account's `running`/`not running`/`unknown` (`accounts.md#connection`). `key` is only the key's ends (`sk-…4f2a`) or null; `spent` is what the account's calls cost this month (UTC): USD of the priced calls, all calls, and the calls whose price is unknown. A mail `analysis` is `{level, model?}` or null (off). |
| `account_removed` | `{ id }` | The account was removed (`remove_account`); its files are gone. |
| `conversation` | `{ id, title, linked_to, active, messages: [{ id, parent, role: "user"\|"assistant", model, date, content, html, status: "done"\|"writing"\|"stopped"\|"failed", error?, suggestions? }] }` | A conversation changed without the page asking: a reply done, failed or stopped, a branch selected. `chat_open` and `chat_send` answer with the same shape. `suggestions` are questions offered to ask next (the mail analysis's chat, `vault.md#conversation`). `messages` are the file's plus the reply being written and the failed ones, which are never written. The page shows the path from the first message to `active`, then on down the newest children, and derives the siblings from `parent`. |
| `ai_delta` | `{ conversation, message, html }` | The reply being written, all of it so far, rendered from its markdown. Coalesced: at most one per run of the host's queue, however fast the model writes. The page ignores it for a conversation it doesn't show. |
| `notice` | `{ level: "error"\|"info", message, entity? }` | `error`: one item of a job failed (a mail, an event; the job's summary counts it too), or the settings couldn't be saved where no command asked for it (the window's size, the vault remembered on a switch), or `config.yaml` couldn't be read at start (on `ready`). `info`: what a job brought — new mail, `"New mail: <title>"` with `entity`, the mail, or `"<n> new mails · <email>"`. A notice without `level` counts as an error. The page shows them in the status bar (`ui.md#status-bar`). |
