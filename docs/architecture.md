# Avelor — Architecture

> The entry point to the documentation: what the app is, the constraints
> and principles it is built on, how host, page and crates are structured,
> the runtime, storage, build and conventions — and the map of the other
> documents.

## What the app is

Avelor manages personal documents in a **vault**: a folder of plain
markdown and YAML files that the user owns (`vault.md#principles`). It
holds:

* chat messages, emails and letters,
* notes and documents,
* data records and lists,
* tasks, contacts and calendar events,
* bank accounts and their statements.

The app lets the user:

* import, browse and edit documents — a WYSIWYG editor saves straight to
  the file;
* assign documents to projects and group them into threads;
* cross-link documents with each other (bills to the transactions that paid them, a
  task to its letter, …);
* have AI-powered conversations about documents, threads or projects;
* present documents in their native UI (a calendar for events, a board
  for tasks, …).

Imports happen by hand, from a folder watch or a Google account, whose
email and calendars also sync on their own, and add the initial metadata
(an identifier, title, creation date, …): email from a Google account,
events from Google Calendar, scanned pictures read by OCR.

## Constraints

| Constraint | Consequence |
|---|---|
| Windows and Linux desktop, Rust host + WebView2 page (WebKitGTK on Linux) | Win32 window and message loop, COM, WinHTTP, DPAPI, WIC (`windows` crate); GTK4 and WebKitGTK 6.0 on Linux (`architecture.md#platforms`) |
| Minimal dependencies | Only `serde`, `serde_json`, `serde_yaml_ng`, `log`, `pulldown-cmark`, `windows`, `webview2-com` (+ `oxc`, `embed-resource`, `static_vcruntime` at build time); on Linux `gtk4` (with the `gdk_pixbuf` it brings), `webkit6`, `soup3`, `gio`, `glib`, `glib-unix` and `libc` instead of `windows` and `webview2-com`. SQLite is the system's (`winsqlite3.dll`, libsqlite3), a library of the platform as WinHTTP and libsoup are, through a hand-written binding (`av-store`). UUIDv7, dates, the inflater (`av-utils`, from the author's `miniz-rs`) and PDFs (`av-pdf`) are hand-written |
| No npm, no third-party bundler | ES modules, checked and bundled into one classic script by `app/build.rs` with `av-bundle` (`development.md#the-bundled-page`) |
| Small executable | release profile for size (`architecture.md#build`), no async runtime, PNG icon frames, CSS minified; new code and dependencies are measured with a `bloat` profile (`development.md#measuring-size`) |
| The vault is the source of truth | No database of record, no drafts on disk (a new entity is a draft card in the page until it is made, `panel.md#draft`); every edit is written to the file; the index is built by an indexing; the catalog beside the vault holds nothing the files don't, and spares the indexing reading unchanged files (`vault.md#catalog`) |
| Host resolves, page presents | Filenames, conflicts, structural rules live in Rust; the page renders and forwards intents |
| One coordinator | `App` alone runs background work (`run*`, `UiQueue`) and posts to the page; workers never reach the webview (no `Send` wrapper around `ICoreWebView2`) |

## Documents <a id="documents"></a>

| Document | Covers |
|---|---|
| **`architecture.md`** | this file: what the app is, principles, structure, runtime, storage, build, conventions |
| `development.md` | checks, the bundled page, testing session end, diagnosis, measuring size, line endings |
| `vault.md` | the file format: identity, roles, types, sources, metadata, file names, layout; opening and indexing a vault, the catalog, the search, the trash, the Vault page |
| `api.md` | the protocol: transport, rules, the page's side, every command and event |
| `ui.md` | the main window: layout, vault panel and filters, lenses, status bar, jobs panel, keys, New, preferences |
| `panel.md` | the side panel: cards, threads, the day view, the context column, links, labels, projects, structural edits and how they are saved |
| `editor.md` | the WYSIWYG editor |
| `markdown.md` | markdown ⟷ editor DOM: the contract, both converters, their tests |
| `saving.md` | dirty state, autosave, the staleness guard, conflicts, closing |
| `accounts.md` | Google and AI accounts: files, secrets, states, the Accounts page, AI uses and the spend limit |
| `email.md` | Fetch email, the email auto-sync, the email analysis |
| `calendar.md` | calendar sync, its rules, auto-sync and sync points |
| `import.md` | folder watches, Import scans |
| `assistant.md` | the assistant: conversations, context, streaming |
| `localization.md` | the app's texts: the catalogs, the page's and the host's, the run's language, the build's check, a new language |

## Doc conventions

- **One owner per fact**: each fact is described in one document; the
  others link to it.
- **References** point at an id: `` `saving.md#conflicts` `` in the docs,
  `` `#conflicts` `` alone within `saving.md` itself (a `` `#x` `` that
  names no section of its doc is an element id of the page), and the full
  form, ASCII, in code comments (`saving.md#conflicts`; every `.md` file
  is in `docs/`).
  A section something refers to carries its id at the end of its heading
  line: `` ## Conflicts <a id="conflicts"></a> ``. An id is a lowercase
  word or two, at most 16 characters, unique within its file and never
  reused for another section, so a heading can be reworded without
  touching a reference. Headings carry no numbers. A test checks them
  (`development.md#checks`).
- **`file:line`**: host files are under `app/src/`, page files under
  `app/static/`, crate files under `crates/<crate>/src/` (written
  `av-vault/vault.rs:201`).
- **Flows** have ids by area (A vault, B navigation, C projects, D
  documents and cards, E threads, F lenses, G sync sources, H Google
  accounts, I AI accounts, J watches, K Import scans, L assistant): trigger
  on the page → page functions → command → what the host does and sends
  back → result. Message names are the protocol's (`api.md`).
- **Tables are unaligned** (`|---|`); the docs are LF.

## Principles <a id="principles"></a>

The rules the code follows, and every change to it:

1. **Keep the stack, add no dependency.** Rust + WebView2 (WebKitGTK on
   Linux), hand-written utilities, no npm, no third-party bundler, no async
   runtime.
2. **One way to do each thing.** One message format and one way to answer,
   one job helper, one account persistence, one confirm widget, one place
   that turns vault changes into page updates.
3. **Boundaries by types and rules, not by layers of folders.** The host
   doesn't need `services/` and
   `repositories/`; it needs state that can only be changed through its own
   methods, and modules that don't reach into each other.
4. **Every request gets exactly one answer; every failure is visible.**
5. **The vault crate owns the vault's schema**; the app orchestrates; the
   page presents.
6. **Pure core, thin shell.** Mapping and decision logic are pure functions
   with tests (as `av_google::calendar_sync::plan` is); I/O sits at the
   edges.
7. **One coordinator.** `App` (`app.rs`) is the only place that runs
   background work and posts to the page; no worker gets a path of its own to the webview. Each new
   coordination primitive (`Req`, `on_ui`, the re-entrancy net, the jobs
   helper, the clock, the catalog's thread) is designed with
   the user before it is built.
8. **The UI thread never waits for the page.** The page is reached only
   through the UI thread's message queue, so a wait for it there needs a
   nested message loop. A Win32 message that needs an answer at once
   (`WM_QUERYENDSESSION`) gets it from what the host already knows, and the
   page keeps the host up to date ahead of time: `set_editing`, saves within a second of a
   pause, the saves a conflict refused. Modal dialogs through `on_ui` stay
   the only modal loops. A request the page can answer later (`WM_CLOSE`)
   is asked, with a timeout.

## Big picture

```mermaid
flowchart LR
    subgraph page["WebView2 / WebKitGTK page — app/static"]
        ui["sidebar · lenses · panel<br/>workspace pages · assistant"]
        ed["editor.js · markdown.js"]
        br["rpc.js<br/>rpcCall · rpcOn"]
    end
    subgraph host["Rust host — app/src"]
        win["shell.rs · page.rs · sys/"]
        ses["App<br/>protocol + runtime + all state"]
        feat["structure · documents · trash · close<br/>api · ai · chat · email · calendar · scans · folders"]
        cfg["config · accounts<br/>ai::accounts · watches"]
    end
    subgraph crates["crates"]
        vault["av-vault"]
        store["av-store"]
        google["av-google"]
        llm["av-llm"]
        pdf["av-pdf"]
        http["av-http"]
        utils["av-utils"]
    end
    br <-- "JSON envelope" --> win
    win --> ses --> feat
    feat --> cfg
    feat --> vault
    feat --> store
    feat --> google
    feat --> llm
    feat --> pdf
    google --> http
    llm --> http
    vault --> utils
    store --> utils
    pdf --> utils
    http --> utils
    vault --> files[("vault folder")]
    store --> catalog[("catalog")]
    cfg --> conf[("config folder")]
    google --> gapi(["Google APIs"])
    llm --> lapi(["LLM providers"])
```

## Runtime

### Threads

| Thread | Runs | Back to the UI thread via |
|---|---|---|
| UI thread | Win32 message loop (GTK's main loop on Linux); owns `App` and everything in it; every command handler; the indexing's batches into the index; every vault write; every change of the index queued for the catalog | — |
| `avelor-catalog`, one for the app's life (`catalog.rs`) | the open vault's catalog (`vault.md#catalog`): its opening, the changes `Vault` queues, in that order, and what is asked of it after them: a worker's `CatalogThread::ask` (the indexing's cached rows), and `App::run_on_catalog`'s work (a search, `search.rs`) | a worker's `ask` waits for its answer, the UI thread never does; `run_on_catalog` as the run family |
| `avelor-worker`, one new OS thread per `App::run*` (`app.rs`) | the indexing's walk and reads (`indexing.rs`); HTTP (Google, LLMs), the OAuth wait, LLM streams, WIC previews (gdk-pixbuf on Linux); email header pages fan out to 8 scoped threads (`gmail::get_headers`); the close's wait | `UiQueue::send_to_ui` → `PostMessageW(WM_TASK)` → `Shell::run_tasks` (`app.rs`, `shell.rs`; on Linux an idle callback) |
| WebView2 processes (WebKitGTK's on Linux) | the page | `PostWebMessageAsString` / `WebMessageReceived` (on Linux `evaluate_javascript` / the script message handler) |

### The run family

Three variants: plain (`run`), with ordered progress callbacks
(`run_with_progress`), and superseding per channel (`run_superseding`;
channels `connect_ai`, `email-preview`, `search`); continuations are
`FnOnce(&mut App, T)`. `run_on_catalog` and `run_superseding_on_catalog`
run the work on the catalog's thread instead, after the changes queued
there before it (`catalog.rs`). One OS thread per task is right without an async runtime:
the OAuth wait and AI streams block for minutes, and a bounded pool would
queue previews behind a sign-in. `Task(Cancel)` aborts
(`av-utils/cancel.rs`) and skips `apply` if it isn't queued yet;
dropping a `Task` leaves its work running. A sign-in runs under a
`Cancel` of its own, the pending connect's (`App::begin_connect`), from
the OAuth wait to the last request. Continuations re-check identity themselves
(`writing.id`, `job_id`, batch ids), because an already queued `apply`
still runs, and so does progress queued before the abort.

### Coalesced progress <a id="progress"></a>

`Progress::send_latest` (`app.rs`) replaces a not-yet-delivered update
of the same stream, so at most one `ai_delta` per run of the UI queue. The
reply's worker and the indexing read's word count use it; previews keep
`send`, where every item counts.

### One way in <a id="one-way-in"></a>

Every way into `App` — page messages, worker results, the window
state — goes through the `UiQueue` queue, and `App` is taken only
while a task runs:

- `run` owns the `WindowProc` until the message loop has ended, and
  `GWLP_USERDATA` only points to it.
- The wndproc and `with_window` hand out `&WindowProc`; its own fields are
  `Cell`s and `RefCell`s, and no COM call runs while a borrow is held.
- `run_tasks` runs the tasks queued so far, one at a time, each with
  `RefCell<Option<App>>` taken by `try_borrow_mut`. A task queued
  meanwhile (a batch of the indexing) waits for its own `WM_TASK` (on Linux an idle callback), so
  the message loop runs in between. Nothing else takes `App`, and nothing
  calls `borrow_mut()` on it: with `panic = "abort"` a second borrow ends
  the app.
- A message that finds `App` taken leaves its task in the queue. The task
  holding it re-posts `WM_TASK` (on Linux an idle callback) once it
  returns.
- Because every page message goes through the queue, a later one can't
  run before a deferred one.

### Dialogs

`App::on_ui(work: FnOnce(&Owner) -> T, apply: FnOnce(&mut App, T))`
queues a `UiTask::Modal`, whose `work` runs on the UI thread *after* the
current handler has released `App`. The main window is the dialog's
owner, and `apply` then runs with `App`. While the dialog is open,
other messages run normally, and a close is refused. The folder pickers,
Explorer and opening URLs or files use it. On Linux the dialog
waits in a nested `glib::MainLoop` (`sys/linux/dialogs.rs`), as a Win32
dialog waits in its modal loop.

### Jobs <a id="jobs"></a>

`App::start_job` / `update_job` / `end_job` / `run_job` (`jobs.rs`): a
card per job in the jobs panel (the `job` event, `ui.md#jobs-panel`),
its requests under one `Cancel`, Stop through `cancel_job {id}`. A stopped
job's `apply` still runs, so a calendar sync records the pushes it made
before the stop and ends itself. Feature slots that hold snapshots for
`apply` (`email.fetch`, `calendar.sync`) stay in their sub-states. A
failure of one entity (an email, an event) is a `notice` plus the job's
summary. `end_job` keeps its summary in the log too, under the job's kind;
`end_job_with` makes it of the job's counts, "stopped · " before
them or "· spend limit reached" after them (`job_summary`). A job that has
a model take its items one after the other keeps them in an `AiQueue`
(`ai/queue.rs`). A vault switch stops the jobs bound to the vault and the
assistant's replies, and opens the new vault once they have ended
(`switch_vault_when_stopped`); Rebuild index opens the same vault this
way. A new long operation becomes a job through
this helper, not a slot with callbacks of its own.

### The clock <a id="the-clock"></a>

Work the app starts on its own goes through `App::on_clock`: the
window's timer (`SetTimer`, once a minute; on Linux a GLib timeout) queues it through the UI queue
as `WM_ACTIVATE` queues `on_activated`, so it runs like every other way
in, with no thread or handle of its own. It starts the email and calendar
auto-syncs that are due (`email::auto_sync`, `calendar::auto_sync`, which
wait for the indexing and for a running fetch or sync); the end of an
indexing calls them too. Their schedule, the look in flight and the
failure said once (`SYNC_EVERY`, `AutoSyncs`) are shared in `google.rs`.
A calendar is also made due by its sync points
(`calendar.md#sync-points`).

### Closing

The window's Close asks the page to save first and goes on once the saves
are answered; Windows ending the session waits
for nothing (principle 8); on Linux the session manager's `query-end`,
SIGTERM and SIGHUP end it the same way. Both are
`saving.md#closing-the-app`.

### Vault edits and the outbox <a id="vault-edits"></a>

Every vault edit by the app goes through `App::edit`, which hands the
edit av-vault's `Index` and pushes every entity it wrote or trashed —
also when it failed part-way:

```rust
pub fn set_title(app: &mut App, req: Req, id: Id, title: String) {
    let step = Text::new("undo.set_title").arg("title", named(app, &id, &title));
    let result = app.step(&req, step, |index, changes| index.set_title(&id, title, changes));
    answer(app, req, result, Some(id));
}
```

av-vault's `Changes` names what an edit wrote, trashed, and whose body it
changed, so the same push sends `entity`, `entity_deleted` and `content`
(`post_entity` serves indexed entries and edits alike). The
editor's own save (`Vault::save_content`, answered by `save_doc_content`)
and taking in a change made outside the app (`Vault::reload`,
`Vault::refresh`) aren't edits by the app and stay apart. An edit the
user makes goes through `App::step`, `App::edit` keeping what it wrote
as a step to undo (`saving.md#undo`); the editor's saves keep theirs
too. `Vault` (`app.rs`), the open
vault's folder and index, keeps both private: the index is read through
`Vault::index` and changed only by `App::edit`, the indexing and these
methods.

`App` posts through an **outbox** (`Box<dyn Fn(String)>`, given to
`App::new`): the web view in the app, a `Vec<String>` in tests. With the
test running the UI queue itself, a command is dispatched against a
temporary vault and its events and reply asserted (`testing.rs`:
`Harness::call`, `notes`); work on a worker is waited for (`wait_for`,
`wait_indexed`). Every feature module has such dispatch tests;
the assistant's run against a fake Ollama (`testing::FakeOllama`).

## Crates <a id="crates"></a>

```mermaid
flowchart BT
    utils["av-utils<br/>error, datetime, schedule, base64, sha256,<br/>secret (DPAPI), logger, text, url, fs, cancel, inflate"]
    http["av-http<br/>WinHTTP / libsoup client"]
    vault["av-vault<br/>envelope, document, index, schema,<br/>filenames, markdown, email, html, conversation"]
    store["av-store<br/>SQLite binding, catalog"]
    google["av-google<br/>OAuth, Gmail, Calendar, sync engine"]
    llm["av-llm<br/>5 providers, streaming, usage, prices"]
    pdf["av-pdf<br/>text layer, scanned pages, writing"]
    bundle["av-bundle<br/>the page's scripts: dependency check<br/>(build only)"]
    texts["av-texts<br/>the app's texts: catalogs, check<br/>(build only)"]
    app["app"]
    http --> utils
    vault --> utils
    store --> utils
    pdf --> utils
    google --> http
    llm --> http
    app --> vault
    app --> store
    app --> google
    app --> llm
    app --> pdf
    app -. build .-> bundle
    app -. build .-> texts
```

| Crate | Owns | Notes |
|---|---|---|
| av-utils | shared `Error`, `DateTime`, the serde skip `is_false`, the days of an RRULE line (`schedule`), `Id` (UUIDv7), base64, SHA-256 (BCrypt), FNV, DPAPI `secret`, logger, random (BCrypt), url (the link schemes, `is_link`), text normalization, unique names and a text cut with a marker (`text`), charsets, a file's stamp and atomic writes (`fs`: `FileStamp`, `write_atomic`), MIME types by extension (`mime`), a patch's fields, absent or null (`patch`), cross-thread `Cancel`, DEFLATE and zlib data inflated (`inflate`, from the author's miniz-rs) | |
| av-http | blocking `Client` / `Request` / `Response`; the process's one session (`Client::shared`); `Request::send_logged`, the one way the APIs are called: logged as `METHOD host/path`, timed, a non-2xx status as `Error::Http` with the reason the caller's parser reads from the body | WinHTTP; libsoup 3 on Linux, one session per worker thread; WinHTTP's IPv6 fast fallback on (a network whose IPv6 goes nowhere cost 21 s per new connection); a streamed read takes what has arrived (`WinHttpQueryDataAvailable`) |
| av-vault | entity model (`envelope.rs`, `document.rs`, `shared.rs`), the schema's names (`schema.rs`), `Index` with structural ops, `Changes` and their steps undone and redone (`vault.rs`, its parts in `vault/`: `indexing.rs`, `create.rs`, `structure.rs`, `properties.rs`, `trash.rs`, the tests in `vault/tests.rs`), the entity edits' `Change` (`change.rs`, `Index::edit_entities`), the indexing (`index_vault`, events applied by `Index::apply`), filenames, `to_html`, `MdWriter`, `plain_title`, a document's `plain_text`, HTML → markdown, MIME parser, conversation trees | no network, no UI; also hosts email parsing |
| av-store | what Avelor keeps beside a vault, nothing only it holds: the catalog, a row per file of the index with its words in FTS5 (`Catalog`: `open`, `cached` with each file's `FileStamp`, `put`, `forget`, `keep_only`, `search`; `SETTLED`; `vault.md#catalog`), the SQLite binding (`sqlite.rs`: `Db`, `Stmt`, closed on drop) | plain types only, no av-vault; the system's SQLite (`architecture.md#platforms`); a connection stays on the thread that opened it |
| av-google | OAuth loopback + PKCE (the wait under a `av_utils::cancel::Cancel`), token refresh, Gmail (filter → query, ids, headers, raw), Calendar v3 subset, the two-way sync engine (`plan` is pure); Google's error bodies (`reason`) | sends through `send_logged` |
| av-bundle | the page's bundler (oxc): the ES modules from the entry through their imports, in the order they run, each a function scope in one classic script; its checks (no import cycle, named imports and exports only, no exported `let`, no write to an import, no name used without its import, the layers; `development.md#the-bundled-page`) | build-only: `app`'s build dependency, nothing of it in the exe |
| av-texts | the app's texts (`localization.md#catalogs`): a catalog read (`Catalog::parse`) and checked against English and the keys used (`check`), a language over English (`over`), the keys the page's script uses (its `t` calls, through oxc) and the host's (`texts::text`) | build-only: `app`'s build dependency, nothing of it in the exe |
| av-pdf | a PDF read (`Document::parse`: cross-reference tables and streams, object streams, rebuilt by scanning for `n g obj`; the page tree) and its pages (`Document::page` → `Page`: the text layer through the fonts' `ToUnicode` maps or encodings, laid out in lines and paragraphs, `usable`; `blank`; `cover`; an indexing's JPEG); writing one (`Writer`, `zlib_stored`), a PDF of image pages (`image_pages`: a JPEG as it is, its headers read by `jpeg.rs`; a PNG's image data with its predictors, its chunks read by `png.rs`); Flate through av-utils' `inflate` | av-utils only; an encrypted file refused (`Error::Encrypted`); tolerant: what can't be read is left out, never a panic; the module doc lists what it leaves out |
| av-llm | `Client` over Anthropic, OpenAI (Responses API, not Chat Completions), Mistral, Gemini, Ollama; the providers' facts in one table (`ProviderSpec`, `PROVIDERS`: names, alias, default URL, key, uploads, auth) and one request builder from it (`Client::request`); streaming (SSE/NDJSON); normalized `Usage`, price table, `Ledger`; the providers' error bodies | sends through `send_logged`; dispatch by macro, no trait |

The graph is acyclic and the network
side never touches the vault side; `app` is the only bridge. av-store
knows neither: `app` turns the indexing's texts into its rows.

### av-vault's core

- **`schema`**: the remote keys and folder names as constants, next to the
  roles they describe; the app imports them (`vault.md#remote`,
  `vault.md#vault-layout`).

  ```rust
  pub mod remote { pub const GMAIL_ID: &str = "gmail-id"; pub const MESSAGE_ID: &str = "message-id"; /* calendar-id, sync-token, time-min, event-id, etag, snapshot */ }
  pub mod layout { pub const PROJECTS: &str = "Projects"; pub const EMAIL: &str = "Email"; /* the folders New writes to, Calendar, Conversations, .trash, .avelor/accounts, .avelor/watches */ }
  ```

- **One copy of the truth in `Document`**: the envelope holds the metadata,
  read through accessors and changed through setters. `envelope` is an
  `Option` — a markdown file without frontmatter stays without one until
  metadata is set, where an empty `Envelope` would write `{}` — and
  `Document` keeps its `id` beside it, since a file without `id` gets one
  made up per indexing. `OnDisk` holds what the file held at the last read or
  write, with its stamp (`saving.md#the-stamp`); a failed write in
  `Index::update` puts back a clone of envelope and body.

  ```rust
  pub struct Document {
      id: Id,
      envelope: Option<Envelope>,  // the only copy of the metadata
      content: String,
      format: FileFormat,
      disk: Option<OnDisk>,        // what the file held at the last read or write
      err: Result<()>,
  }
  ```

- **`conversation`**: an `impl Conversation` with `path_to`, `leaf_from`,
  `add`, `activate` and `ConversationMessage::is_reply`, the tree rules the
  assistant follows.

## Host modules

| Module | Responsibility |
|---|---|
| `main.rs` | logger, panic hook, the config (`load_config`), the run's language (`texts::init`), a failed start's box with the log's folder |
| `shell.rs` | what the main window decides on every platform: its state, the UI task loop (`run_tasks`) and the re-entrancy net, Close to `App` (`architecture.md#platforms`) |
| `page.rs` | the page (`app/static/index.html`), the same on every platform, with its preferences, the shipped blueprints, the texts in the run's language, its own filled in (`worded`), the platform's `BRIDGE_JS`, and its content security policy (`with_policy`) |
| `texts.rs` | the catalogs the build wrote, the run's language (`init`, `locale`: its catalog and its tag), the texts the host shows or writes itself (`text`, `filled`, `of`, `Text::render`), a text the page shows (`Text`: a key and its arguments, a date or money among them); its tests |
| `blueprints.rs` | the shipped blueprints (`app/blueprints/*.yaml`, `vault.md#project`): their facts with their types, checks and labels, read with `shipped`, as the page's data block (`json`), their words keys of the catalogs worded in the run's language; their tests |
| `sys.rs` | the platform layer: what each backend provides (`architecture.md#platforms`) |
| `sys/win32/paths.rs`, `sys/win32/dialogs.rs` | the app's folders (`paths`: the config, the log, the web view's profile, the catalogs), the mark of the web; folder picker, message boxes, ShellExecute |
| `sys/win32.rs`, `sys/win32/window.rs` | `run`; Win32 window, wndproc, size persistence, the session's end; the clock's timer (`WM_TIMER` → `App::on_clock`); `UiSignal` (`WM_TASK`, `WM_CLOSE_NOW`); Close to `App` |
| `sys/win32/images.rs` | image previews with WIC (`preview`): decoded, upright by the EXIF orientation, scaled to fit, JPEG; COM on a worker (`ComThread`) |
| `sys/win32/webview.rs` | WebView2 environment/controller, message handler, the page held to itself (`hold`), `NavigateToString`; App's outbox (`PostWebMessageAsString`) |
| `sys/linux/paths.rs`, `sys/linux/dialogs.rs` | the same in the XDG folders, with GTK |
| `sys/linux.rs`, `sys/linux/window.rs` | `run`: the `gtk::Application`, the session's end; the window, size persistence; the clock (a GLib timeout); `UiSignal` (`glib::idle_add_once`); Close to `App` |
| `sys/linux/images.rs` | the same with gdk-pixbuf |
| `sys/linux/webview.rs` | the WebKitGTK web view with its network session, the script message handler `host`, the page held to itself (`hold`), App's outbox (`evaluate_javascript`), `load_html` |
| `app.rs` | `App` (all state) and `Vault` (the open vault's folder, av-vault's `Index` and the catalog's thread, private: read through `index`, edited through `App::edit`; `content`, `save_content`, `reload`, `refresh`, `changed_at`, `require_written`; every change of the index sent to the catalog in its order, `catalog_key`; its steps, `history`), the config writes no request asked for (`save_window_state`, `remember_vault`), `on_web_message` (every message to `protocol`), `post_text` / `post_event` through the outbox, `edit` and `edit_written`, `run*` (with `run_on_catalog`), `on_ui`, `Progress` (`send`, coalescing `send_latest`), `UiQueue` / `UiTask` (`close_window`), entity pushes, the pending sign-in, `on_clock` |
| `catalog.rs` | the catalog's thread (`CatalogThread`, `vault.md#catalog`): `open`, the writes `put`, `forget`, `keep_only`, logged when they fail and leaving the catalog behind, `ask` for a worker; what it holds (`CatalogState::current`, `Unavailable`) |
| `search.rs` | `search` (`vault.md#search`): an entity's words (`words_of`), the catalog's rows on its thread, one rule for them (`holds`), the excerpt; its dispatch tests |
| `indexing.rs` | the indexing (`vault.md#opening`): `run_indexing` with av-vault's `index_vault` on a worker, which asks the catalog for its cached rows, its batches into the index (`Vault::apply_indexed`, the reads to the catalog with `keep_indexed`, the catalog left with the index's files by `indexing_done`) and to the page (`vault`, `entity`, `vault_error`, the indexing's `job`), a replaced scan's batches dropped, `indexing`; its dispatch tests |
| `protocol.rs` | the protocol: envelope, `Command`, router, `Req`; the dispatch tests |
| `testing.rs` | the test harness: a temporary vault, `Harness::call`, the collected events, `undoes`, `FakeOllama` |
| `close.rs` | closing the app (`CloseState`): the `close` event, its answer `close_ready`, the wait for a page that doesn't answer; the session ending, answered without the page (`on_session_end`); its dispatch tests |
| `jobs.rs` | the jobs helper: start, update, stop and end a job (the `job` event), `run_job`, `cancel_job`, the vault switch that waits for the old vault's jobs, and Rebuild index's reopen (`rebuild_when_stopped`); a job's summary (`count_summary`, `job_summary`); its tests |
| `api.rs` | `ready`, vault open (with its accounts, watches and settings), switch, reveal and `rebuild_index`, `set_prefs`, `open_url`, `open_file` and the types it opens (`may_open`), a navigation the web view refused (`open_refused`), `page_error` |
| `documents.rs` | a document's content in the editors: `load_doc_content`, `save_doc_content`, `check_files`, taking in files changed outside the app (`post_taken_in`); `set_editing`; `DocumentState`, the documents in edit mode and the app's version of each one in conflict; the conflict copy shared with calendar sync (`keep_conflict_copy`, `keep_held`); their dispatch tests |
| `trash.rs` | the trash: `list_trash`, `restore_entity` (an edit, answered like the structural commands), `delete_trashed`, `empty_trash` |
| `structure.rs` | the structural commands: `create_entity`, threads, projects (`create_project`, `set_project`), titles, stars, archiving, labels, links (a transaction's too), `edit_entities`, deletes, `reload_entity`; their answers; their dispatch tests |
| `undo.rs` | undo and redo (`saving.md#undo`): `History`, `App::step`, the commands `undo` and `redo`; their dispatch tests |
| `config.rs` | `config.yaml` (`ConfigStore`: read through its getters, each setter writes the file) |
| `accounts.rs` | both kinds of account: the one pass over `.avelor/accounts/` on vault open, the events `accounts` / `account_removed`, `disconnect_account`, `remove_account`; their dispatch tests |
| `google/accounts.rs`, `google/oauth.rs`, `google.rs` | Google account files, scope levels, the `GoogleAccounts` sub-state; the OAuth client, the build's or the user's own (`google.client`); the Google commands (connect, cancel, calendars, disconnect); `finish_sync`, the end of every sync of an account; the auto-syncs' schedule, `SYNC_EVERY` and `AutoSyncs` |
| `config/files.rs` | the config-file helpers (`accounts.md#design`): `SecretFile`, `ConnectionStatus`, `write_yaml`, `load_config_files`, `free_stem` |
| `settings.rs` | what the Settings page sets: the language as `ready` tells it (`language`), `set_language`, `set_theme`; its dispatch tests |
| `me.rs` | who the user is in the open vault (`config.yaml` `me`): `set_me`, `by`, which `App::edit` has a new entity's source name (`vault.md#sources`); its dispatch tests |
| `kept.rs` | what Avelor keeps on this PC outside the vaults (`Folders`): `kept`, `reveal_data`, `remove_data`, and the removal once the window is gone (`remove`); its dispatch tests |
| `vault_settings.rs` | the vault's own settings (`.avelor/vault.yaml`): the `VaultSettings` sub-state, loaded on vault open, `set_vault_settings`, the event `vault_settings`; its dispatch tests |
| `ai/context.rs` | the context builder: what the AI is told about a conversation's linked document or thread, each as its file with its attachments' read text, within a budget, each as far as its classification reaches the model; its tests |
| `ai/policy.rs` | what reaches a model: how far an entity's classification reaches one on this PC and one off it, whether a model runs on this PC, a `masked` entity's values as placeholders and the reply's values filled in; its tests |
| `chat.rs` | the assistant: `chat_open`, `chat_send`, `chat_stop`, `chat_select`, the events `conversation` and `ai_delta`; streamed replies, the request from the shown path and the linked entity; `create`, a new AI conversation (also the email analysis's chat); dispatch tests against a fake Ollama |
| `ai/accounts.rs`, `ai.rs` | AI account files, key and spend file, the `AiAccounts` sub-state with `choose` and `check_limit` (a feature's model pick, `ModelChoice`, checked; the spend limit); the AI commands (connect, models, settings) and `account_used` |
| `ai/queue.rs` | an AI job's queue (`AiQueue`): its items one after the other on one account, what a stop left, the spend limit, the failures and their notice; the email analysis's, the attachments' and the scans' reading; its tests |
| `email.rs`, `email/` | Fetch email: `email_labels`, `email_search`, `email_headers`, `email_fetch`, `set_email_sync`; `EmailState`; the fetch job as plan → run → apply with its pure pieces (`plan_fetch`, `Downloaded::parse`, `attachment_wanted`, `fetch_action`, `email_entity`, `attachment_entity`); the attachments saved in the apply; the auto-sync (`auto_sync`, `sync_query`); a fetch's attachments handed to their reading, its new emails to the analysis; their tests; its parts in `email/`: `preview.rs`, `fetch.rs`, `entities.rs`, `apply.rs`, `auto_sync.rs`, the tests in `email/tests.rs` |
| `attachments.rs` | the attachments' reading (`email.md#attachments`): what a document takes (`step_for`, pure), the read job one file at a time (`read`, `AttachmentState`), the text written while the body is empty, the analysis started at its end; its tests against a fake Ollama |
| `analysis.rs`, `analysis/` | the email analysis (`email.md#analysis`): the vault's `context`, the request per level, its parser and its writes (pure: `request_for`, `parse`, `apply_lite`, `medium_messages`), a document's, an attachment or an indexing (`document_request`, `parse_document`, `apply_document`), the analysis job one email or document at a time (`analyse`, named by its `Batch`, `AnalysisState`), the `Analysis` settings an email filter and a watch share; its tests against a fake Ollama; its parts in `analysis/`: `context.rs`, `request.rs`, `answer.rs`, `apply.rs`, `job.rs`, the tests in `analysis/tests.rs` |
| `calendar.rs`, `calendar/` | Google ⟷ vault event mapping (pure: `from_google`, `to_google`, `event_entity`, `exception_of`); `calendar_sync` and `set_calendar_sync`; `CalendarState`; the sync job as plan → run → apply, with the trash's deletes; the auto-sync (`auto_sync`, `on_auto_listed`) and its sync points (`changed`, `activated`, `waiting_delete`); their dispatch tests; its parts in `calendar/`: `mapping.rs`, `auto_sync.rs`, `plan.rs`, `apply.rs`, the tests in `calendar/tests.rs` |
| `watches.rs`, `watches/folders.rs` | watch files in `.avelor/watches/` (`images` and `pdf` watches, with a watch's analysis and Keep original; `statements` watches with their bank account), the `Watches` sub-state, globbing, listing; the watch commands, the events `watches` / `watch` / `watch_removed`, their dispatch tests |
| `bank.rs`, `bank/` | account statements (`import.md#statements`): the reader of comdirect's CSV export (`read`, pure), what a statement adds to its account (`plan`, pure: what is there, the pending transactions replaced, the balances checked), the transfers it pairs with the vault's other accounts (`pairs`, pure), `statements_import` as a job, `new_account`, `set_bank_account`; their tests, the reader's on `data/statements`; its parts in `bank/`: `comdirect.rs`, `plan.rs`, the tests in `bank/tests.rs` |
| `recurring_payments.rs` | recurring payments (`vault.md#recurring`): `create_recurring_payment`, `set_recurring_payment`, `set_transaction_recurring_payment`; the recurring payment a new transaction of the import joins (`join`, pure; `join_new`), a recurring payment's days from av-utils' `schedule`; their tests, on the forged vault's recurring payments and the December statement |
| `scans.rs`, `scans/` | Import scans and PDFs: `scan_list`, `scan_read`, `scan_previews`, `scan_open`, `scan_import`, the events `scan_page` / `scan_preview`, reading as a job (`readable` by the watch's type, the account and model resolved, else `Engine::None` for a PDF watch, the spend limit checked per file, the file read through `reader.rs`), `scan_document` (pages → document), a file kept beside a document of one page (an encrypted PDF always, marked), a PDF of a document's images, the documents imported handed to the analysis; their dispatch tests; its parts in `scans/`: `read.rs`, `import.rs`, `previews.rs`, the tests in `scans/tests.rs` |
| `reader.rs` | what a file reads to, shared by Import scans and the attachments' reading: its `Kind` by extension; `read` with an `Engine` (a model), its failures in words (`ReadError`); a PDF by `plan_pdf` (`import.md#reading-pdfs`, pure, tested): its text layer through `av-pdf`, page by page with its scans by the model, the whole file where that can't be done, an encrypted one not at all; a text file's markdown (`text_of`), the model's request (`ask`, streamed) and its answer cleaned (`clean`, `word_count`); its tests |

## Platforms <a id="platforms"></a>

**Platform code, by area**

| Area | Windows | Where | Linux |
|---|---|---|---|
| Folders | `%LOCALAPPDATA%\Avelor` (known folder): config, log, WebView2 profile, catalogs | `sys/*/paths.rs` | `$XDG_CONFIG_HOME/avelor` for the config; `$XDG_STATE_HOME/avelor/log`; `$XDG_DATA_HOME/avelor/webkit` and the cache folder for the web view; `$XDG_CACHE_HOME/avelor/catalog` for the catalogs |
| Window, main loop, UI queue | Win32 window; `PostMessageW(WM_TASK)` wakes `run_tasks` | `sys/*/window.rs` (`UiSignal`) | `gtk::Application` (`NON_UNIQUE`, one process per start as on Windows) and an `ApplicationWindow`; `glib::idle_add_once` wakes the queue (`MainContext::invoke` would run it at once on the UI thread, inside the task that woke it) |
| Fullscreen | F11 from the controller's `AcceleratorKeyPressed`, or the page's `window`; the frame removed and the monitor filled, the placement from before restored | `sys/*/window.rs`, `sys/win32/webview.rs` | F11 from an `EventControllerKey` in the capture phase, ahead of the web view; `fullscreen()`, `unfullscreen()` |
| Zoom | Ctrl+Plus, Ctrl+Minus and Ctrl+0 from the controller's `AcceleratorKeyPressed`, or the page's `window`, `SetZoomFactor`; Ctrl+wheel and a pinch are WebView2's own (`SetIsZoomControlEnabled`) | `sys.rs` (`Zoom`: Chromium's levels), `sys/linux/window.rs`, `sys/win32/webview.rs` | the keys from F11's `EventControllerKey`, Ctrl+wheel from an `EventControllerScroll`, both in the capture phase, ahead of the web view; `set_zoom_level`; a pinch is WebKitGTK's own |
| Session end | `WM_QUERYENDSESSION`, `WM_ENDSESSION` | `sys/win32/window.rs`, `sys/linux.rs` | `query-end` (the application registers with the session manager), SIGTERM and SIGHUP; SIGINT closes as the window's Close does |
| Web view | WebView2: `PostWebMessageAsString`, `WebMessageReceived`, `NavigateToString`; `NavigationStarting` and `NewWindowRequested` refuse leaving the page, the context menu and DevTools in a debug build only, `ProcessFailed` logged | `sys/*/webview.rs` (`BRIDGE_JS`, `hold`); `rpc.js` | WebKitGTK 6.0: a script message handler `host`, `evaluate_javascript`, `load_html`; `decide-policy` refuses leaving the page, the context menu (`context-menu`) and developer extras in a debug build only, `web-process-terminated` logged |
| Theme | the one picked (`config.yaml` `theme`), else Windows' theme for apps (`AppsUseLightTheme`): the title bar (`DWMWA_USE_IMMERSIVE_DARK_MODE`) and the web view's ground before the page paints (`SetDefaultBackgroundColor`), again on `WM_SETTINGCHANGE` and when another is picked (`UiSignal::theme`); WebView2's `prefers-color-scheme` follows Windows' setting | `sys/win32/window.rs`, `sys/win32/webview.rs` | GTK's theme: the window's, whatever is picked, and through it WebKitGTK's color scheme |
| Language | the display languages (`GetUserPreferredUILanguages`); WebView2's `Language` | `sys/*.rs` (`languages`), `sys/*/webview.rs` | the languages GLib reads from the environment (`language_names`); the web context's preferred languages |
| Dialogs and shell | `IFileOpenDialog`, `MessageBoxW`, `ShellExecuteW` | `sys/*/dialogs.rs` | `gtk::FileDialog`, `gtk::AlertDialog`, `gtk::UriLauncher` and `FileLauncher` (they use the XDG desktop portal where there is one) |
| Saved attachments | the mark of the web: zone 3 in the file's `Zone.Identifier` stream (`mark_downloaded`), for SmartScreen and Office's Protected View | `sys/*/paths.rs` | none: Linux has no such mark |
| Image previews, EXIF, HEIC | WIC | `sys/*/images.rs` | gdk-pixbuf (`apply_embedded_orientation`); WebP and HEIC where their loaders are installed |
| HTTP | WinHTTP; cancelling closes the handles | `av-http/src/win32.rs`, `linux.rs` | libsoup 3 (`soup3`): it comes with WebKitGTK as WinHTTP comes with Windows; a session per worker thread and timeout, since a libsoup session stays on the thread that made it; cancelled through a `gio::Cancellable` |
| SQLite | `winsqlite3.dll` in System32, linked by `raw-dylib` (no import library); FTS5's trigram tokenizer needs 3.34 or later | `av-store/sqlite.rs` | libsqlite3, which WebKitGTK brings; `libsqlite3-dev` to link |
| Secrets at rest | DPAPI | `av-utils/secret.rs`, `fs.rs` | a key file only the user can read: `protect` keeps the bytes, `fs::write_private` writes the file (0600) |
| Random | BCrypt | `av-utils/random.rs` | `getrandom` |
| SHA-256 | BCrypt | `av-utils/sha256.rs` | GLib's `Checksum`, as glib comes with GTK |
| File permissions | `readonly()`, `set_readonly(false)` before deleting from the trash | av-vault `vault/trash.rs` | a Unix form for each: `readonly()` is true only when no one may write; `set_readonly(false)` makes a file world-writable (`delete_trashed`) |
| Editor | developed against WebView2 (Chromium): `contenteditable`, the selection | `editor/editor.js` | WebKitGTK's `contenteditable` and selection differ; `markdown.selftest.html` runs in both (`markdown.md#testing`) |
| Icons and fonts | inline SVGs from Fluent UI System Icons (MIT), one sprite (`app/static/icons`, `build.rs`); Segoe UI | `css/tokens.css` (`--f-chrome`) | the same SVGs, since the Segoe font can't be shipped; `--f-chrome` already ends in `system-ui` |

**The platform layer.** `app/src/sys/` holds a module per platform;
`sys.rs` states the contract and re-exports one backend's items. It is
the only `cfg(target_os)` in `app/src`:

- `paths()`: the config, log, web view and catalog folders.
- `run(paths, config)`: the window, the web view and the main loop;
  returns once the window is gone.
- `UiSignal` (`Send + Sync`): `wake()`, `close()` and `step()`, the page's zoom
  or fullscreen (`WindowStep`). `UiQueue` holds one instead of an
  `HWND`; tests use `UiSignal::none()`.
- `Owner`: what a `UiTask::Modal` gets instead of an `HWND`.
  `pick_folder(&Owner, title)` and `pick_file` wait in the platform's
  modal loop (`IFileOpenDialog`; `gtk::FileDialog` in a nested
  `glib::MainLoop`), so `App::on_ui` keeps its meaning.
- `open_url`, `open_path`, `show_error` (a failed start: `MessageBoxW`, or `gtk::AlertDialog` if GTK
  is up, else stderr), `preview`, the watch folder expansion,
  `mark_downloaded`, the user's `languages`, and `BRIDGE_JS`.

**The shared shell.** `shell.rs` holds what the main window decides:

- the queue runner and the re-entrancy net (`App` in a
  `RefCell<Option<_>>`, the deferred flag, the modal count;
  `architecture.md#one-way-in`);
- close, the app's own close and the session end;
- the window active again (`App::on_activated`);
- the clock (`App::on_clock`);
- the window state;
- a navigation the web view refused (`api::open_refused`).

Each platform's window only turns its native events into calls on it,
so the subtle part is written once:

| Event | Windows | GTK4 |
|---|---|---|
| Close | `WM_CLOSE` | `close-request` |
| Activation | `WM_ACTIVATE` | `notify::is-active` |
| Clock | `WM_TIMER` | `timeout_add_seconds_local(60)` |
| Window state | `WM_EXITSIZEMOVE`, a change of maximized; never while fullscreen | `notify::maximized`; the size on close (GTK4 has no window position); before fullscreen, never during it |

**The page's bridge.** `page.rs` builds the page for both platforms:
the bundle, the preferences and texts blocks and the platform's
`BRIDGE_JS` inlined before the bundle. The
bridge defines `window.host.post(text)` and calls
`window.host.receive(text)`, which `rpc.js` sets. On WebKitGTK the page
posts to `webkit.messageHandlers.host`, and a reply is
`evaluate_javascript` of `host.receive(…)` with the text as a JSON string
literal.

**The page held to itself.** The page trusts its own bundle only
(`architecture.md#threats`), and the web view holds it to that:

- **Its policy**: a content security policy (`page::with_policy`) lets
  only the two scripts the page carries run, by their SHA-256, so no
  handler or `javascript:` link in a document or a model's reply does;
  styles inline; images only as `data:` URLs (the scans' previews), so a
  web image never loads, nor carries anything out in its address; nothing
  fetched, framed or sent.
- **No way out**: the page's own load goes through, every navigation after
  it and every new window is refused (`hold`); the app opens a web, email
  or phone link in its default app, as `open_url` does
  (`api::open_refused`), and only logs anything else. The page refuses a
  drop nothing takes, so a file dropped onto the window never replaces it
  (`ui/dropzone.js`).
- **Release builds**: no context menu and no DevTools; a debug build has
  both.

**The crates.** av-http's `client.rs` is portable (the builder, logging,
multipart, `send_logged`), with a backend module per platform behind the
same `Client` / `Request` / `Response`; a `Response` isn't
`Send` on either. av-utils picks its `random` and `secret` per platform.
av-store's binding is the same on both; only the library it links
differs (`cfg_attr` on its `extern` block).

## Page <a id="page"></a>

ES modules
make the page, each with its own scope: a file's dependencies are its
`import` lines, and the order the modules run in follows from them
(`development.md#the-bundled-page`). Every module but `app.js`, the entry,
is in a layer folder (`core/`, `editor/`, `model/`, `ui/`, `features/`) and
imports only from its own layer or a lower one, which the build checks;
nothing imports `app.js`.

| File | Role |
|---|---|
| `core/dom.js`, `core/store.js`, `core/cache.js`, `core/format.js` | `$`, the element factory `accEl` (every element `features/` and `ui/` make), `pressable` (with the keydown that presses one), `titled`, `oncePerFrame`, `placeChildren`, `typing`; observable store (`setState`/`subscribe` with `Object.is`, one selector or an array of them); per-id content cache; the date helpers (`parseDate`, `isoInstant`, `dayStart`, `daysBetween`, `addDays`, `sameDay`, `weekStartOf`, `monthStartOf`, `monthEndOf`, `localDay`, `everyMidnight`, `relativeDay`), and every date format of the page (`TIME_FORMAT`, `MONTH_YEAR`, …, `shortDate`, `pageDate`) and the order of texts (`compareText`), in the page's locale |
| `core/text.js` | the page's texts: `t`, a text of the run's catalog, `parts`, one with an element in it, `textOf`, one the host sent, `ordinal`, and the run's `locale` (`localization.md`) |
| `core/rpc.js` | the protocol: `rpcCall`, `rpcOn`, `RpcError` (`api.md#the-page-side`); an undo step's group (`newGroup`) |
| `core/actions.js`, `core/match.js` | the action registry, the commands and the keys Go to lists (`listedActions`, `keyedActions`), the one global key handler and the keys' names (`shortcutKeys`, `shortcutText`); names found by words typed (`fit`) |
| `editor/editor.js`, `editor/markdown.js` | `createEditor`, which makes its parts for each editor (`editor/selection.js`, `editor/blocks.js`, `editor/tables.js`, `editor/shortcuts.js`, `editor/paste.js`); `isLink` (`editor/links.js`); the tags both read (`editor/tags.js`); `serializeMarkdown` |
| `model/state.js` | the store and its schema (the vault's state, the selection and filters, the words and their answer cleared by `NO_WORDS`, the counters, `dirty_docs`, `conflicts`, `notices`, the preferences); `savePrefs`, `setNotice`, `showError` |
| `model/entities.js` | the Maps `entities`, `projects`, `threads`, `accounts`, `watches`, the vault's settings (`vaultSettings()`); the selectors (`matchesFilter` with `NO_PROJECT`, `projectScope`, `projectsInOrder`, `projectNames`, `sourceKeyOf`, a synced calendar's entity (`calendarEntityOf`), `accountWatch`, `getMeta`, `knownThreads`, `threadMembers`, `threadProjects`, the conversations with an AI); the selection published minus what is gone (`commitExistingSelection`) |
| `model/bank.js` | the bank accounts: their types (`ACCOUNT_TYPES`), an account's statements (`statementsOf`) and how far they go (`statementsEnd`), its ledger, the posted transactions with the balance after each (`ledger`); a transaction's types, party, name, a transfer's other half and its projects through its link or its recurring payment (`TRANSACTION_TYPES`, `partyRef`, `transactionName`, `pairOf`, `linkProjects`); the accounts and statements in the filter, and whether a transaction or a recurring payment is in a project (`bankScope`, `transactionInProject`, `recurringPaymentInProject`); the recurring payments: a transaction's day, party's name and recurring payment, an account's recurring payments, their amounts, party names, occurrences paid, off, missed or extra, those expected to a day, the next, how often and how many a year (`transactionDay`, `partyName`, `recurringPaymentOf`, `recurringPaymentsIn`, `amountsOf`, `withAmount`, `amountOn`, `partyNames`, `occurrences`, `expectedIn`, `nextOccurrence`, `howOften`, `perYear`); the horizon and an account's balance on it (`horizonOf`, `balanceOn`) |
| `model/calendar.js` | what the calendar shows: the events in the filter with their recurring events expanded (RFC 5545 lines), the tasks due, the day an entity shows on (`calendarDayOf`) |
| `model/schedule.js` | the schedules, an event's and a recurring payment's: their RFC 5545 lines read (`parseSchedule`, `ruleSupported`) and expanded from a first start (`scheduleStarts`); a recurring payment's period and its day as an RRULE line and back, and how many times a year a rule repeats (`scheduleLine`, `periodOf`, `timesAYear`) |
| `model/documents.js` | the save pipeline (`saving.md`): the content cache, the dirty state, saves and autosave, an editing session's group (`beginEditSession`), the conflicts, the documents in edit mode (`editorRoots`, `reportEditing`) |
| `model/trip.js` | a trip's events: their types, means and states, whether an event is one (`isTripEvent`), the zones of its ends, the places the vault's name and the zone each had last (`placeZone`, `tripPlaces`), what a row tells of one (`tripSummary`); a trip's route, its stops and legs from its events or its sketch (`routeOf`, `nightsOf`), where it doesn't go on as it arrived (`issueText`, `zoneIssuesOf`), where an event is on it (`routeSpot`), what it lacks (`tripGaps`), the trips a person's ID documents run out before (`tripsShortOf`), what setting its dates proposes (`setDatesProposal`), and its travelers' work calendars (`workCalendarsOf`) |
| `model/id-documents.js` | a person's ID documents: their contact's list, a type in words and its icon, a type's own fields in words (`idDocumentExtras`), how an expiry stands (`idDocumentState`), a new item's id |
| `model/when.js` | a sketched trip's When: its window's days, its lanes and their spans, the windows that fit and what each costs in workdays (`whenOf`) |
| `model/blueprint.js` | the shipped blueprints from the page's blueprints block (`BLUEPRINTS`, `blueprintOf`), and the block a new project of one starts with (`newProjectBlock`); a project's facts as its header shows them (`factsOf`, `withFact`), its entries (`membersOf`), what it has spent (`spentIn`), and what its checks find: the gaps, the next steps, how far it is (`checksOf`) |
| `model/structure.js`, `model/events.js`, `model/drafts.js` | the structural requests (`structureCall`, the panel's `joinThreadLocal`, …, `setProjectLocal`, `editEntityLocal`, `setTitleLocal`, `reloadEntity`), a draft's own changes for its id; the draft, an entity not made yet (`openDraft`, `changeDraft`, `createDraft`, `panel.md#draft`); the host's events into the model (`entity`, `entity_deleted`, `content`, the accounts and watches, `vault`, `vault_settings`, `activated`; `forgetVault`), the only writer of the Maps |
| `ui/icons.js`, `ui/menu.js`, `ui/confirm.js`, `ui/toolbar.js` | the icons (`KIND_ICONS`, an entity's by its kind, type or part of a trip: `docIcon`, `iconEl`) and a kind in words (`kindName`); the popup menu (`showMenu`) with its Esc, and a drop-down box that opens one (`dropBox`); the two-click confirm (`armedButton`: one armed button page-wide, which a page rebuilt on every change names by a key); the editors' toolbars, and a tool button with a label (`toolButton`) |
| `ui/page.js`, `ui/field.js`, `ui/segmented.js`, `ui/text-list.js` | the vault area (`openVaultOp`, `closeVaultArea`) and its pages: their Esc and Ctrl+Enter (`registerVaultPage`), `replaceVaultPage`, which keeps the focus; the parts of a list + detail page (`listDetailPage`, `pageList`, `itemCard`, `addTile`, `detailPane`, `detailHead`, `stats`, `notice`, `failedRetry`, `detailFooter`) and the changes a page shows until the host has them (`sentDrafts`); editing in place (`editTextInPlace`), a card's fields (`fieldInput`, `editField`), its blocks drawn anew but not while typed in or pressed (`heldWhileTyping`, `keepingFocus`), and the Workspace pages' editable field (`renderEditField`); the segmented control (`accSeg`), its options words or icons; a list of texts as a field (`textListField`) |
| `ui/marks.js` | the words of the vault panel's filter marked (`ui.md#words`): `termsRegex`, `markedNodes`, `excerptNodes` |
| `ui/statusbar.js`, `ui/dropzone.js` | the snackbar of the store's notices, failed calls nobody handled among them, an info notice going on its own; what takes a drop and marks where it would land (`dropTarget`, one mark page-wide, every drop target of the page), and dragging an entity between the lenses and the panel (the drag preview, `MEMBER_DRAG` and the lenses that take it, the panel's drop zones) |
| `features/selection.js`, `features/context.js` | the selection (`selectDocument`, `selectById`, `selectDay`, a trip's leg: `selectLeg`, the filter, `openLink`, `panelMode`, the day view's listing, the marks on what shows the selected entity: `markSelection`; kept across restarts: `restoreSelection`); the context column's tabs and the card's buttons that open them |
| `features/lenses/registry.js` | the lens registry: the tabs, the lenses' elements and renders (`registerLens`), at most one render per frame of the shown lens (`invalidateLenses`), the lens shown last on start (`restoreLens`) |
| `features/panel/details.js`, `features/panel/id-documents.js`, `features/panel/trip-fields.js`, `features/panel/event-fields.js`, `features/panel/schedule-field.js`, `features/panel/contact-fields.js`, `features/panel/draft.js`, `features/panel/panel.js`, `features/panel/transaction.js`, `features/panel/recurring-payment.js`, `features/panel/find.js` | what a card shows of its event, task, email, document, recurring payment, record or contact, a task's fields, a type, a record's own fields, a contact's ID documents (`panel.md#id-documents`), the classification, a trip event's fields (`panel.md#trip-fields`; also New's place inputs), an event's times and Where (also New's zones' menu) and the menu of calendars (`calendarMenu`), a schedule as a field and in words, a contact's fields, the draft card (`panel.md#draft`), and a recurring payment's fields editable in place; the side panel renderer (day view, thread, a trip's leg, standalone card, the blocks other modules add to a card, `registerCardBlock`), a transaction's card (`panel.md#transaction-card`), a recurring payment's sum and occurrences (`panel.md#a-recurring-card`), its chips, the structural edits it starts, its subscriptions; the find strip, the words marked in the cards shown and stepped through (`panel.md#find`) |
| `features/dashboard/dashboard.js`, `features/dashboard/cards.js` | the Dashboard lens: its cards' order and the closed ones, the Cards menu, dragging a card; the cards Week, Tasks (a task dragged to another column), Upcoming, Payments, Recent files and Recent conversations |
| `features/lenses/documents.js`, `features/lenses/timeline.js`, `features/lenses/calendar.js`, `features/lenses/bank.js`, `features/lenses/recurring.js`, `features/lenses/read-only.js` | lenses, a row's read-only tag; Documents groups its rows by project and thread, takes entities dropped on those rows, and shows an entity on request (`showInDocuments`, `revealInDocuments`); Timeline has the scrubber with its range (a trip's stop's too, `showWeeks`) and the feed of day blocks, a trip's gaps among them; Bank has the accounts in the scope as tiles, the cash flow and the transactions by month, and its Recurring view the fixed costs (`ui.md#bank`) |
| `features/lenses/types.js` | the type selection of the Documents and Timeline lenses: the types (`DOC_TYPES`, `typeOf`), a lens's bar of toggles and its preferences (`typeBar`), the lens head's Types button |
| `features/lenses/scrub.js` | the scrubber of the Timeline and Bank lenses (`scrubber`): its head with the range picked, Clear and Today, a track of a bar per week the lens draws, a dashed line at today and the labels under it, the range dragged across it or picked (`pick`); the weeks (`weeksBetween`, `weekOf`) |
| `features/workspace/models.js` | the models features pick: the AI providers, `aiModelOf`, the picker (`aiModelPicker`), `aiMissingHint`; its way to the Accounts page is the `accounts.open` action |
| `features/workspace/folders.js`, `features/workspace/accounts.js`, `features/workspace/client.js`, `features/workspace/trash.js` | Workspace pages: the folder watches with their types; the accounts, Google (a calendar's Work of, the user's own client) and AI; the trash |
| `features/import/scans.js`, `features/import/email.js` | Import scans; Fetch email, and an account's email auto-sync |
| `features/conflict.js`, `features/close.js` | the conflict dialog (`saving.md#the-dialog`); closing the app: the `close` event leaves edit mode in every editor and answers `close_ready` once the saves are answered |
| `features/jobs.js` | job cards from the `job` event, with Stop |
| `features/palette.js` | Go to (`ui.md#go-to`): the dialog; the hints naming a key |
| `features/palette-found.js` | what Go to finds: the places, the entries, the commands and the keys, as blocks of rows |
| `features/keys.js` | the keys that are no command, by where they work (Go to's Keys) |
| `features/window.js` | the page's zoom and full screen as commands |
| `features/undo.js` | Ctrl+Z and Ctrl+Y as the actions `undo` and `redo` |
| `features/words.js` | the vault panel's words box (`ui.md#words`): `search` at most every 120 ms, its answer as `word_filter`, Esc and Ctrl+F |
| `features/blueprint.js`, `features/facts.js`, `features/route.js`, `features/gaps.js`, `features/when.js`, `features/set-dates.js` | a project's blueprint under its description (`panel.md#blueprint`): its facts, a trip's route (its parts' clicks, the sketch edited in place), When, Missing and Next, Find; the menu that fills a trip's gap (`fillGap`, also the Timeline's) and its events (`eventMenu`, also When's); When, the windows a sketched trip's days could take and its lanes edited (`panel.md#when`); Set the dates, a sketched trip's days proposed as planned events (`ui.md#set-the-dates`); a fact's value and its editor by its type; the menu of contacts (`contactMenu`, also the Accounts page's and Settings') |
| `features/header.js`, `features/sidebar.js`, `features/sources.js`, `features/projects.js` | the lens header: the project's name, the words' and the source's chips, the active labels and their "+" menu, Clear filters, the project's details and description, a new project's (`openNewProject`), `leaveEditModes`; the vault panel: projects, the New project menu, No project and sources with their counts and their menus, the filter, the vault menu; the sources as both list them (`vaultSources`); a project's menu, its pill's and its header's (`projectMenu`) |
| `features/new.js`, `features/recurring-payment-new.js`, `features/linked.js` | New: the menu of types, each a draft card (`panel.md#draft`), the Task tab, `create_entity`; New recurring payment, filled from a transaction (`panel.md#a-new-recurring`); the Linked tab: links to, linked from, a search to link to |
| `features/workspace/vault.js` | the Vault page; `switchVault`, `revealVault`, Rebuild index |
| `features/workspace/settings.js` | the Settings page (`ui.md#settings`), its rows as commands, Help and settings |
| `features/assistant/tree.js`, `features/assistant/assistant.js` | conversation tree helpers; the assistant |
| `features/layout.js` | the shell's layout: the resizing grips with the widths in the preferences, the vault area's and the panel's layout, the page items, the sidebar's sections, the panel's close, open and expand buttons |
| `app.js` | the entry: its imports run the modules that act when they load, in the order they act; the lenses' start, the renders wired to the model, the dirty dots, `ready` |
| `css/` | the stylesheets in cascade order (`architecture.md#css`) |
| `icons/` | the icons, Fluent UI System Icons (MIT, `icons/LICENSE`), each a file as it comes, joined into the page's sprite by `build.rs` |

A page is drawn in one of three ways, by what it must keep while it
changes:

- **Rebuilt whole**, the focused input found again by its
  `data-focus-key` (`replaceVaultPage`): the pages of the vault area,
  forms that change as a whole and are small.
- **Built once and patched**: the panel and Fetch email, whose editors,
  scroll position and inputs must survive a change.
- **Reconciled by key** (`placeChildren`): the lenses and the sidebar,
  long lists whose rows keep their hover, focus and drop state.

### Page state <a id="page-state"></a>

The store (`model/state.js`) holds selection and UI state, the documents
with unsaved edits, the conflicts and the notices, plus revision counters
(`projects_rev`, `threads_rev`, `accounts_rev`, …). The domain data lives
in Maps outside it (`entities`, `projects`, `threads`, `accounts`,
`watches`, in `model/entities.js`), changed in place. Only the host's
events write them, in `model/events.js`, each followed by a counter's
bump; a vault being opened clears them (`forgetVault`). A page that keeps
its own state about an account or a watch (its drafts, what it shows
selected) listens to the same events, and its listener runs after the
model's: `rpcOn` calls the listeners in the order they came, and `app.js`
imports `model/events.js` before every feature.

The model calls no render. The renders subscribe to the store (`app.js`,
the features): the lenses and the sidebar to the counters, the dirty dots
to `dirty_docs`, the conflict dialog to `conflicts`, the status bar to
`notices`. A render that follows several values subscribes with an array
of selectors (`store.subscribe([s => s.selected_project, s =>
s.projects_rev], …)`), called when any of them changed.

A render never sends a change to the host. It may ask once for data it
shows and the page doesn't hold, guarded by a cache that marks it as
loading: an account's calendars and models (`features/workspace/accounts.js`),
an account's email labels (`features/import/email.js`).

### Actions <a id="actions"></a>

What a key, a menu, a button or Go to (`ui.md#go-to`) starts is registered
once (`registerAction({id, title, shortcut, outsideFields, host, when, run,
group, place, icon, also, page, end, archived, where})`, `runAction(id,
…args)`, `core/actions.js`). One `keydown` handler runs the first action
with the key's shortcut, or one of its shortcuts, whose `when` holds; a key
something else already handled (`defaultPrevented`, Ctrl+I as italic in an
editor) is left alone, so is one of an action `outsideFields` while a field
or an editor has the focus, none acts behind an open dialog, and one of a
`host` action is only shown: the host takes it. An action with a `group` is
a command Go to lists by its `title`, found by `also` too, while its `when`
holds; one with a `place` is a place it goes to, with `end()` at its row's
end. Its list of keys has the commands with a shortcut, those that don't
apply now too, under the part of the app they work in (`where`). The
commands and places of what the vault has come from functions asked each
time it opens (`registerActions(list)`), a source's import or sync, a
project, a source. A global command is registered beside what it runs. The
pages of the expanded vault area register what Esc and Ctrl+Enter do on them
(`registerVaultPage`, `ui/page.js`). An action is also how a lower layer
starts something above it: a new email's notice in the status bar runs
`documents.show` (`showInDocuments`, `features/lenses/documents.js`). The
keys in use: `ui.md#keys-and-actions`.

### CSS <a id="css"></a>

The stylesheets are split by cascade layer, not by feature: `index.html`
links them in the order they cascade, and `build.rs` joins them in that
order into the page's one `<style>`. The order stands in one place, as
`app.js`'s imports order the modules.

| File | Holds |
|---|---|
| `css/tokens.css` | the colors, each light and dark, the fonts, sizes, insets, heights, radii, shadows and layers |
| `css/base.css` | the box model, the page's ground, links, the global states (`[hidden]`, `.is-resizing`), controls |
| `css/type.css` | the type roles (`label`, `meta`, `row-title`, `page-title`, …) and the utilities (`mono`, `icon`, `truncate-start`, `push`, `stack`) |
| `css/components.css` | what several areas share: `btn`, `tool`, `seg`, `chip`, `check`, `input`, `menu`, `tile`, `list-row`, `section-head`, the lenses' scrubber (`scrub`), the words marked (`mark`, `::highlight`), the forms, `pane-head`, the dialogs, the vault area's `page`, the list + detail page (`ui/page.js`), a proposal's rows to tick (`proposal`, Set the dates), … |
| `css/areas/*.css` | one file per area, in this order: shell, sidebar (with the jobs), dashboard, documents, timeline, calendar, bank, panel, editor, assistant (with the context column), workspace (the vault area's pages and New), import (Import scans, Fetch email), header (the lens header, the titles edited in place), palette (Go to) |

Conventions the page keeps:

- Tokens for sizes, insets and heights (`--fs-*`, `--p-*`, `--h-*`);
  shared components (`btn`, `tool`, `icon-btn`, `seg`, `chip`, `input`,
  `list-row`, `pane-section`, `page`, the list + detail page's
  `list-detail`, `item-card`, `add-tile`, `detail-head`, `stats`,
  `notice`, `detail-footer`, `edit-field`, …) and type roles (`label`, `meta`,
  `row-title`, `page-title`, …) come before the areas' files, so a block
  rule can override a component; values that must change together live in
  one token or class; BEM without prefix; states as ARIA attributes where
  one fits, `is-*` otherwise; every class used is defined and nothing
  defined goes unused (`development.md#measuring-size`).
- **Layers** (the Recede design): the chrome sits on `--c-shade`, lists on
  `--c-paper`, the active content on `--c-card` with `--shadow-raise` /
  `--shadow-card`. New UI follows these layers.
- **Two themes**: every color is a token, `light-dark()` holding its light
  and its dark value and the theme picking one (`color-scheme`): the
  system's, or the one on the root (`data-theme`), which the page is loaded
  with and Settings changes (`ui.md#settings`); a rule never sets a color of
  its own, which fails the build (`#build`). Text or an icon on an accent or
  danger fill takes `--c-on-accent`, dark in both; the accent as a line of
  2px or less (a ring, a border) `--c-accent-line`, 3:1 on the grounds, and
  danger as text, an icon or a thin line `--c-danger-ink`, 4.5:1; the pale
  accent and danger stay for fills.
- **Keyboard**: what is clicked is a `<button>`, or, when it holds
  controls of its own (a row with its buttons), `pressable`
  (`core/dom.js`): Tab reaches it, Enter or Space presses it. A control
  shown on hover shows on its row's `:focus-within` too, never
  `visibility: hidden` alone, which takes it out of the Tab order. The
  focus shows as `base.css`'s ring; a rule that sets `outline: none`
  shows it another way (an input's border). A row a render replaces
  keeps its focus by a focus key (`keepingFocus`, a page's
  `data-focus-key`).
- **Names**: a control that shows a glyph (an x, a +, an arrow, a check
  mark) or an icon alone is named by its tooltip's words: `titled` in a
  script, `data-t-title` with `data-t-aria-label` in the markup, an icon
  alone by its `data-t-title`. The areas are labelled landmarks, and a
  new notice is read out (`ui.md#status-bar`). What slides or fades
  stops when the system asks for less motion (`base.css`).
- **The trade-off**: a control is made reachable, named and visible
  where that is small (a button, an attribute, a rule, a token); where
  more would add code or steps to the UI, it stays as it is and
  `ui.md#keys-and-actions` says so. No conformance to a standard is
  claimed.
- **Build from what exists** before adding a class: the utilities `push`,
  `stack`, `truncate-start`; `.page-head`, `.select-bar`; typography that
  must match a role joins that role's selector list. The global `[hidden]`
  rule and button reset aren't repeated per block.
- **Specificity**: a block rule that sets the same property as a component
  overrides it, so a block class and a component are combined only when
  their properties don't overlap. A bare `.is-x` loses to any later block
  rule of the same specificity: qualify it with its block
  (`.panel.is-expanded`), or, for a state
  across blocks, use `!important` like `[hidden]` and `.is-resizing`.
- A small visual change is fine when it simplifies the CSS, as long as it
  is pointed out.

### ASCII-only sources

No literal non-ASCII in `app/static`: `\u2026` in JS, `&#x2026;` in HTML,
`"\2026"` in CSS. The build checks it (`architecture.md#build`). The
catalogs are UTF-8, outside it (`localization.md#catalogs`).

## Storage and configuration <a id="storage"></a>

The app's configuration lives in `%LOCALAPPDATA%\Avelor\` (`config.rs`,
`config_dir`), on Linux in the XDG folders (`architecture.md#platforms`):

- `config.yaml`: the vault of the last session (`vault`), the window's
  size and maximized state, the language the user picked (`language`,
  `localization.md#language`) and the theme (`theme`: `light` or `dark`,
  else the system's), who the user is in each vault (`me`, contacts' ids,
  `ui.md#settings`), and the page's preferences (`prefs`,
  `ui.md#preferences`);
- the accounts' DPAPI secrets, `<id>.token` and `<id>.key`, and the
  user's own Google client, `google.client` (`accounts.md#secrets`);
- the WebView2 profile in `…\Avelor\WebView2`;
- the log files, one per run, in `…\Avelor\log`
  (`development.md#diagnosis`);
- the vaults' catalogs in `…\Avelor\catalog` (`vault.md#catalog`),
  written through SQLite's transactions.

Settings shows what is kept there and can remove it (`kept.rs`,
`ui.md#settings`): once the window is gone, by the files' names, the web
view's by their folder, trying again for some seconds while the web view's
processes end. A file of another name stays, and so does a folder that isn't
empty then; what can't be removed is named in a box.

A broken `config.yaml` is set aside at start, without a question, as
`config.yaml.broken`; an older one is kept, and the broken file is
dropped. A key `config.yaml` doesn't know is kept as it is, so the
settings of another version survive a save. Everything the vault's
entities name by id — accounts — is kept in the vault's `.avelor/`, and so are
the vault's own settings (`vault.md#vault-config`). Every overwriting
write, in the vault and here, is atomic (`saving.md#atomic-writes`).

## Threat model <a id="threats"></a>

Avelor runs as the user, on their PC, with their files. It trusts its own
bundle (the page and its scripts, built into the exe), the user, and the
account it runs under. Another program running as that account reads the
vault and the app's folders as the app does: that is the system's to
prevent, not the app's.

What comes from elsewhere is not trusted, and each kind is held where it
comes in:

- **Mail**: MIME and HTML from anyone (av-utils' `mime` and `charsets`,
  av-vault's `email`). The HTML becomes markdown (`html.rs`): its links
  only web, email or phone links (`is_link`), its images their text. An
  attachment is saved with the mark of the web
  (`architecture.md#platforms`), and opened from the app only if it is a
  PDF or an image (`api::may_open`).
- **Files**: PDFs (av-pdf, av-utils' `inflate`), images (`sys::preview`),
  a bank's statements (`bank.rs`), markdown other tools wrote. A
  document's original or a watch's file is opened in its default app only
  if it is a PDF or an image; a watch's file only from inside its folder
  (`invalid_path`).
- **Web pages**: never loaded. The web view holds the page to itself, and
  a link goes to the browser (`architecture.md#platforms`).
- **The models' answers**: markdown like a document's, raw HTML escaped
  (`markdown::to_html`), shown under the page's policy: no script in it
  runs and no web image loads, so an answer steered by a mail it read
  can't carry the vault out in an image's address. A link in it opens
  only when clicked, in the browser.
- **The services' replies**: Google's and the AI providers' JSON, read
  into types (serde); a reply that doesn't fit is an error, not data.
- **The page's messages**: each is decoded into a typed command
  (`protocol.rs`), and names an entity by its id.

Secrets at rest are the system's to protect: DPAPI on Windows, a file only
the user can read on Linux (`architecture.md#platforms`). The log holds
no secret, and what it keeps is said in `development.md#diagnosis`.

## Build <a id="build"></a>

`app/build.rs` bundles the page's modules with `av-bundle`, from `app.js`
through its imports, into one classic script (a syntax error, an import
cycle, a missing import or a broken layer fails the build,
`development.md#the-bundled-page`), parses it with oxc on every build, joins
the CSS files `index.html` links in their order (`architecture.md#css`;
a rule with a color of its own, outside `tokens.css`, fails the build),
minifies JS (oxc, mangling) and CSS (hand-written) in release builds,
inlines both into `index.html` in `OUT_DIR`, with the icons of
`app/static/icons` as one hidden SVG of symbols (an icon nothing refers to,
the page's sources or a shipped blueprint, or a reference to one that isn't
there, fails the build), checks the catalogs against the keys the page
and the host use and writes each language's texts for both
(`localization.md#check`), and embeds the
manifest, the icon and the version (a VERSIONINFO from the workspace's
version), on Windows; a manifest whose version isn't the workspace's
fails the build (`development.md`, CI and releases). On Linux the desktop entry and the icon are
files (`app/res`), installed as `README.md` says. A page
source with a non-ASCII character fails the build, named by its line and
column. A new module
is in the page once a module there imports it. The markdown selftest gets
a bundle of its own beside it (`markdown.md#testing`). The host fills the
preferences, the shipped blueprints and the run's texts into the page
(`page::html`), with its content security policy, and
loads it with
`NavigateToString`, on Linux with `load_html`.

The release profile is built for size: `opt-level = "z"`, LTO, one codegen
unit, `strip`, `panic = "abort"` (`Cargo.toml`); the panic hook
`log_panics` (`logger.rs`) logs thread, message and location before the
abort. Where the bundle lands and how to measure the size:
`development.md#the-bundled-page`, `development.md#measuring-size`.

## Conventions <a id="conventions"></a>

- **Every import or sync has the same shape**: *plan* on the UI thread from
  the vault (what is there, what to ask for) → *run* on a worker (remote
  I/O, no vault access) → *apply* on the UI thread through the vault edit
  (`App::edit`). Calendar sync (`build_job`, the engine, `apply`) and
  email fetch (`plan_fetch`, the downloads, `fetch_action`) work this way;
  the scan import has its plan and its apply. The mapping between remote
  data and entities is a pure function with tests (`event_entity`,
  `email_entity`, `scan_document`).

  ```mermaid
  flowchart LR
      plan["plan<br/>UI thread, reads the vault"] --> run["run<br/>worker, remote I/O only"] --> apply["apply<br/>UI thread, App::edit"]
      apply --> events["entity / content / job events"]
  ```

- **AI use** names the model its feature picked (a `ModelChoice`, kept
  with the feature's settings); the host reads the pick there and checks
  it with `AiAccounts::choose`, and never puts another model in its place
  (`accounts.md#model-choice`).
- **Naming**: a feature's commands, sub-state and module share its name. A module of several parts keeps its file and puts its parts in a folder of its name beside it (`sys.rs` and `sys/win32.rs`), a split module its tests in a file of their own there; modules that belong to another go into its folder.
- **Errors**: one `av_utils::Error` for every layer (`av-utils/error.rs`:
  logging, vault, OAuth, transport, Win32, GTK, SQLite), extended when needed; a
  separate error type only for functions local to one module; expected
  outcomes are success variants, not errors (`SaveOutcome::Conflict`,
  `PushOutcome::Conflict` in av-google). An id that names nothing is
  `NotFound`, one that names something of another kind `WrongKind`; a
  failed write is `FileWrite { path, err }`. A failed HTTP call is
  `Error::Http { status, reason, msg }` (`Request::send_logged`, the
  reason read by the calling crate's parser) or `Error::Network { msg }`;
  the service module reads the status codes it knows by matching
  `Error::Http { status, .. }`. A caller that can't ask the user turns a
  save conflict into `Error::ChangedOutside { path }` through
  `Vault::require_written`, an edit of its own through `App::edit_written`
  (`saving.md#limits`). A function whose failure
  is a reason the user reads, not an error a caller matches on, returns
  those words as a `Text` (`texts.rs`, `localization.md#host-texts`: a
  page of Import scans that read no text, an answer of the email analysis
  that wasn't JSON); one that answers a request with a code and the
  host's words returns both (`ai::accounts::Refusal`).
- **Entity ops are generic**: a vault or host op is built on a property
  every entity has, for any entity (`Index::set_title`, not
  `rename_project`), even when the UI offers it for one kind. An empty
  value removes the key (an untitled entity is valid) instead of being
  refused.
- **Google access** is Off / Read / Full per service, and nothing else
  (`accounts.md#access-levels`).
- **Remote versions are compared in one place**: the calendar sync
  engine's two hooks (`calendar.md#design`).
- **Sync and commit points come from the edit state**: the host knows the
  documents in edit mode (`set_editing` → `DocumentState`), and an edit
  ends when its document leaves edit mode, not with each autosave. The
  calendar's sync points use it.
- **Reuse before writing**: `av_vault::markdown::plain_title`,
  `jobs::count_summary`, `Req::save_failed`, `AiQueue`, `App::edit_written`,
  `text::{cut_marked, unique_trimmed}`, `av_utils::is_false`,
  `DateTime::{from_system_time, plus_seconds}`,
  `OneOrMany::{as_slice, from_vec}`; on the
  page the pieces named in `architecture.md#page` and `architecture.md#css`.
  A new shared helper replaces every existing copy of its pattern in the
  same change.
- **Formatting**: Rust code is rustfmt-formatted; `cargo fmt --all` is
  fine on any file at any time.

## Ownership

| Part | Owns | Must not |
|---|---|---|
| av-utils | primitives with no domain (dates, ids, encodings, inflating, hashing, DPAPI, atomic writes) | know about vaults, Google, LLMs; carry network types |
| av-http | transport, cancellation | know any API |
| av-vault | the vault's schema, files, index, rules, conversions | do network or UI; start threads (the app decides where indexing runs) |
| av-store | the SQLite binding, the catalog's tables | know av-vault; hold anything the vault's files don't |
| av-pdf | reading and writing PDF files | know vaults or models; read or write files itself |
| av-google / av-llm | their remote APIs, pure sync and pricing logic | touch the vault or the page's presets |
| `app` feature modules | orchestration: plan → run → apply, accounts and watches files | parse messages, render, reach into other features |
| `app` shell + protocol | Win32, WebView2, GTK, WebKitGTK, dialogs; message decoding and replies | contain feature logic |
| page | rendering, user input, the page's copy of the data | resolve filenames, conflicts or structural rules |
