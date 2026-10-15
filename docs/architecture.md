# Avelor — Architecture

> The entry point to the documentation: what the app is, the constraints
> and principles it is built on, how host, page and crates are structured,
> the runtime, storage, build and conventions — and the map of the other
> documents.

## What the app is

Avelor manages personal documents in a **vault**: a folder of plain
markdown and YAML files that the user owns. It
holds:

* chat messages, emails and letters,
* notes and documents,
* data records and lists,
* tasks, contacts and calendar events,
* bank accounts and transactions.

The app lets the user:

* import, browse and edit documents — a WYSIWYG editor saves straight to
  the file;
* assign documents to projects and group them into threads;
* cross-link documents with each other (bills to payment transactions, a
  task to its letter, …);
* have AI-powered conversations about documents, threads or projects;
* present documents in their native UI (a calendar for events, a board
  for tasks, …).

Imports happen automatically from configured watch folders or by hand,
and add the initial metadata (an identifier, title, creation date, …):
mail from a Google account, events from Google Calendar, scanned pictures
read by OCR.

## Constraints

| Constraint | Consequence |
|---|---|
| Windows and Linux desktop, Rust host + WebView2 page (WebKitGTK on Linux) | Win32 window and message loop, COM (`windows` crate); GTK4 and WebKitGTK 6.0 on Linux (`architecture.md#platforms`) |
| Minimal dependencies | Only `serde`, `serde_yaml_ng`, `log`, `windows`, `webview2-com` (+ `oxc`, `embed-resource`, `static_vcruntime` at build time); on Linux `gtk4`, `webkit6`, `glib`, `glib-unix` and `libc` instead of `windows` and `webview2-com` |
| No npm, no third-party bundler | 1 ES module, checked and bundled into one classic script by `app/build.rs` with `av-bundle` (`development.md#the-bundled-page`) |
| Small executable | release profile for size (`architecture.md#build`), no async runtime, PNG icon frames; new code and dependencies are measured with a `bloat` profile (`development.md#measuring-size`) |

## Documents <a id="documents"></a>

| Document | Covers |
|---|---|
| **`architecture.md`** | this file: what the app is, principles, structure, runtime, storage, build, conventions |
| `development.md` | checks, the bundled page, testing session end, diagnosis, measuring size, line endings |

## Doc conventions

- **One owner per fact**: each fact is described in one document; the
  others link to it.
- **References** point at an id: `` `saving.md#conflicts` `` in the docs,
  with the file name also within `saving.md` itself (`` `#x` `` alone reads
  as an element id), and the same form, ASCII, in code comments
  (`saving.md#conflicts`; every `.md` file is in `docs/`).
  A section something refers to carries its id at the end of its heading
  line: `` ## Conflicts <a id="conflicts"></a> ``. An id is a lowercase
  word or two, at most 16 characters, unique within its file and never
  reused for another section, so a heading can be reworded without
  touching a reference. Headings carry no numbers.
- **`file:line`**: host files are under `app/src/`, page files under
  `app/static/`, crate files under `crates/<crate>/src/` (written
  `av-vault/vault.rs:201`).
- **Tables are unaligned** (`|---|`); the docs are LF.

## Principles <a id="principles"></a>

The rules the code follows, and every change to it:

1. **Keep the stack, add no dependency.** Rust + WebView2 (WebKitGTK on
   Linux), hand-written utilities, no npm, no third-party bundler, no async
   runtime.
6. **Pure core, thin shell.** Mapping and decision logic are pure functions
   with tests; I/O sits at the edges.

## Runtime

### Threads

| Thread | Runs | Back to the UI thread via |
|---|---|---|
| UI thread | Win32 message loop (GTK's main loop on Linux) | — |

### Closing

Windows ending the session waits for nothing (principle 8); on Linux the
session manager's `query-end`, SIGTERM and SIGHUP end it the same way.

## Crates <a id="crates"></a>

| Crate | Owns | Notes |
|---|---|---|
| av-utils | shared `Error`, `DateTime`, logger, atomic writes (`fs`) | |
| av-bundle | the page's bundler (oxc): the ES modules from the entry through their imports, in the order they run, each a function scope in one classic script; its checks (no import cycle, named imports and exports only, no exported `let`, no write to an import, no name used without its import, the layers; `development.md#the-bundled-page`) | build-only: `app`'s build dependency, nothing of it in the exe |

## Host modules

| Module | Responsibility |
|---|---|
| `main.rs` | logger, panic hook, the config (`load_config`) |
| `shell.rs` | what the main window decides on every platform: its state (`architecture.md#platforms`) |
| `page.rs` | the page (`app/static/index.html`), the same on every platform |
| `sys/mod.rs` | the platform layer: what each backend provides (`architecture.md#platforms`) |
| `sys/win32/paths.rs`, `sys/win32/dialogs.rs` | the app's folders (`paths`: the config, the log, the web view's profile); message boxes |
| `sys/win32/mod.rs`, `sys/win32/window.rs` | `run`; Win32 window, wndproc, size persistence, the session's end |
| `sys/win32/webview.rs` | WebView2 environment/controller, `NavigateToString` |
| `sys/linux/paths.rs`, `sys/linux/dialogs.rs` | the same in the XDG folders, with GTK |
| `sys/linux/mod.rs`, `sys/linux/window.rs` | `run`: the `gtk::Application`, the session's end; the window, size persistence |
| `sys/linux/webview.rs` | the WebKitGTK web view with its network session, `load_html` |
| `config.rs` | `config.yaml` (`ConfigStore`: read through its getters, each setter writes the file) |

## Platforms <a id="platforms"></a>

**Platform code, by area**

| Area | Windows | Where | Linux |
|---|---|---|---|
| Folders | `%LOCALAPPDATA%\Avelor` (known folder): config, log, WebView2 profile | `sys/*/paths.rs` | `$XDG_CONFIG_HOME/avelor` for the config; `$XDG_STATE_HOME/avelor/log`; `$XDG_DATA_HOME/avelor/webkit` and the cache folder for the web view |
| Window, main loop | Win32 window | `sys/*/window.rs` | `gtk::Application` (`NON_UNIQUE`, one process per start as on Windows) and an `ApplicationWindow` |
| Session end | `WM_QUERYENDSESSION`, `WM_ENDSESSION` | `sys/win32/window.rs`, `sys/linux/mod.rs` | `query-end` (the application registers with the session manager), SIGTERM and SIGHUP; SIGINT closes as the window's Close does |
| Web view | WebView2: `NavigateToString` | `sys/*/webview.rs` | WebKitGTK 6.0: `load_html` |
| Dialogs and shell | `MessageBoxW` | `sys/*/dialogs.rs` | `gtk::AlertDialog` |

**The platform layer.** `app/src/sys/` holds a module per platform;
`sys/mod.rs` states the contract and re-exports one backend's items. It is
the only `cfg(target_os)` in `app/src`:

- `paths()`: the config, log and web view folders.
- `run(paths, config)`: the window, the web view and the main loop;
  returns once the window is gone.
- `show_error` (a failed start: `MessageBoxW`, or `gtk::AlertDialog` if GTK
  is up, else stderr).

**The shared shell.** `shell.rs` holds what the main window decides:

- the window state.

Each platform's window only turns its native events into calls on it,
so the subtle part is written once:

| Event | Windows | GTK4 |
|---|---|---|
| Window state | `WM_EXITSIZEMOVE`, a change of maximized | `notify::maximized`; the size on close (GTK4 has no window position) |

## Page <a id="page"></a>

`app/static`, 1 line of JS.

| File | Role |
|---|---|
| `app.js` | the entry |

### ASCII-only sources

No literal non-ASCII in `app/static`: `\u2026` in JS, `&#x2026;` in HTML.
The build checks it (`architecture.md#build`).

## Storage and configuration <a id="storage"></a>

The app's configuration lives in `%LOCALAPPDATA%\Avelor\` (`config.rs`,
`config_dir`), on Linux in the XDG folders (`architecture.md#platforms`):

- `config.yaml`: the window's size and maximized state;
- the WebView2 profile in `…\Avelor\WebView2`;
- the log files, one per run, in `…\Avelor\log`
  (`development.md#diagnosis`).

A broken `config.yaml` is set aside at start, without a question, as
`config.yaml.broken`; an older one is kept, and the broken file is
dropped. A key `config.yaml` doesn't know is kept as it is, so the
settings of another version survive a save. Every overwriting write here
is atomic.

## Build <a id="build"></a>

`app/build.rs` bundles the page's modules with `av-bundle`, from `app.js`
through its imports, into one classic script (a syntax error, an import
cycle, a missing import or a broken layer fails the build,
`development.md#the-bundled-page`), parses it with oxc on every build,
minifies JS (oxc, mangling) in release builds, inlines into `index.html` in
`OUT_DIR`, and embeds the manifest and icon, on Windows. On
Linux the desktop entry and the icon are files (`app/res`), installed as
`README.md` says. A page
source with a non-ASCII character fails the build, named by its line and
column. A new module
is in the page once a module there imports it. The host loads
the page (`page::html`) with `NavigateToString`, on Linux with
`load_html`.

The release profile is built for size: `opt-level = "z"`, LTO, one codegen
unit, `strip`, `panic = "abort"` (`Cargo.toml`); the panic hook
`log_panics` (`logger.rs`) logs thread, message and location before the
abort. Where the bundle lands and how to measure the size:
`development.md#the-bundled-page`, `development.md#measuring-size`.

## Conventions <a id="conventions"></a>

- **Errors**: one `av_utils::Error` for every layer (`av-utils/error.rs`:
  logging, the operating system), extended when needed; a separate error type only for functions
  local to one module; expected outcomes are success variants, not errors;
  a failed write is `FileWrite { path, err }`.
- **Reuse before writing**: `DateTime::from_system_time`. A new shared
  helper replaces every existing copy of its pattern in the same change.
- **Formatting**: Rust code is rustfmt-formatted; `cargo fmt --all` is
  fine on any file at any time.

## Ownership

| Part | Owns | Must not |
|---|---|---|
| av-utils | primitives with no domain (dates, atomic writes) | know about vaults, Google, LLMs; carry network types |
| `app` shell | Win32, WebView2, GTK, WebKitGTK | contain feature logic |
