# Avelor — Development and testing

> How to check a change, release a build, find a problem, measure the executable. The rules the code
> follows are `architecture.md#principles` and `architecture.md#conventions`.

## Checks <a id="checks"></a>

- `cargo test --workspace` runs the unit tests and every feature module's
  dispatch tests (`testing.rs`: `Harness` over a temporary vault, `call` and
  the collected events and notes, `with_google` for a vault with a Google
  account, `FakeOllama` for the assistant; `architecture.md#vault-edits`).
  The vault scan reads on a worker: a test that opens or switches a vault
  runs the queue with `wait_scanned`, which waits for the scan's end.
  av-vault's `forged_vault_loads_with_multi_roles` scans `data/forged`, so
  every file there must load.
- The catalog: av-store's tests make their catalogs in the temporary
  folder; a `Harness` keeps its vault's in its own folder (`catalog`),
  closed before the folder goes (`CatalogThread::close`), and reads the
  rows with `CatalogThread::locals` and `cached`. A file the tests write
  is too new to be cached (`vault.md#catalog`); one whose modified time is
  set an hour back (`File::set_modified`) is. CI runs the tests on Linux
  only, so a change to the scan or the catalog is tested on Windows by
  hand too: there the walk's stamps come from the directory listing
  (av-vault's `a_scan_given_its_own_reads_loads_the_same_without_reading`).
  The words need SQLite's FTS5 with the trigram tokenizer, 3.34 or later.
- The build bundles the page's modules and checks them
  (`development.md#the-bundled-page`); `av-bundle`'s rules have their own
  tests.
- `cargo clippy --workspace`.
- `markdown.selftest.html` for the editor's markdown: the build's copy in
  `OUT_DIR`, opened from disk (`markdown.md#testing`).
- What every AI provider is sent (method, URL, headers, with and without
  a key) is a golden table in `av-llm/client.rs`
  (`requests_of_every_provider`); a change to the request builder or to
  `ProviderSpec` keeps it, or changes it on purpose.
- The user tests new flows by hand against `data/forged`.

## CI and releases

CI (`.github/workflows/ci.yml`) builds on Windows and Linux and runs fmt,
clippy and the tests on Linux. The OAuth client comes from the
repository's secrets `AVELOR_GOOGLE_CLIENT_ID` and
`AVELOR_GOOGLE_CLIENT_SECRET`; without them (a pull request from a fork
gets none) the build, clippy and the tests hide Google. Each build keeps
its result as an artifact: on Windows `avelor-<version>-windows-x64.exe`,
packed with UPX (a pinned release, checked against its SHA-256); on Linux
`avelor-<version>-linux-x86_64.tar.gz` with the exe, the icons and the
desktop file the README installs. A pushed tag `v<version>`, the version
in `app/Cargo.toml`, makes a draft release with both once every job has
passed; a build without the OAuth client makes none. The draft is
published by hand.

## The bundled page <a id="the-bundled-page"></a>

`build.rs` bundles the page into
`target/debug/build/avelor/<hash>/out/index.html`, and the
markdown selftest beside it (`markdown.selftest.html`). Several hash
folders exist, so take the one newer than the edited file. While the app
runs, `cargo build` compiles but can't replace the locked exe; `cargo test
-p avelor --no-run` still runs `build.rs`, so the page is bundled
and checked. The bundle holds the modules `app.js` imports, directly or
not: a new module nothing imports is simply absent from the page, and one
that registers or subscribes when it loads goes into `app.js`'s imports
at the place it acts.

`build.rs` also writes `page-deps.txt` there: a count, then each module in
the order it runs, with the names it imports by the module they come
from; a module imported for its side effects stands alone. What the
bundle can't take fails the build, each problem naming its modules:
an import cycle, a name imported that isn't
exported, an exported `let`, a write to an import, a name used without
its import (`features/x.js uses el, which core/dom.js declares, without
importing it`), a module in no layer, or one importing from a higher layer
(`core/dom.js imports from ui/log.js, a higher layer (ui/)`). So does a
non-ASCII character in any page source, a module, a stylesheet or an HTML file
(`line 273, column 53: '§' (U+00A7) isn't ASCII`), listed once per module
that imports the file.

## Testing session end <a id="session-end"></a>

Start the exe from Explorer, not with `cargo run`: under cargo the app sits
in cargo's job object, which is killed together with cargo before Windows
asks the app anything. Without a restart, a PowerShell script finds the
window with `FindWindow("Avelor", [NullString]::Value)` (`$null`
would become `""` and match nothing) and sends `WM_QUERYENDSESSION`
(0x11), then `WM_ENDSESSION` (0x16, wParam 1). The app keeps running, and
the log shows "Windows asks to end the session (flags …): yes", "Windows
is ending the session" and, per held conflict, "Kept the edits of … as …".
To stage a conflict: edit in the app, change the file outside, switch back
so `check_files` opens the dialog. The kept copy is a new entity tagged
`conflict`, named from its title (`saving.md#session-end`).

On Linux, `kill -TERM` or `kill -HUP` with the app's process id ends the
session as a logout's signal does: the log shows "SIGTERM: the session
ends" and the window's size is in `config.yaml`. Ctrl+C in the terminal
closes the window. The session manager's `query-end` needs a desktop
session; WSLg has none.

## Diagnosis <a id="diagnosis"></a>

Read the log first: `%LOCALAPPDATA%\Avelor\log` (on Linux
`$XDG_STATE_HOME/avelor/log`), one file per run
(`av-utils/logger.rs`), times in UTC; the panic hook logs a panic's
thread, message and location before the abort. Requests that take
multiples of 21 s are TCP connect timeouts on a network whose IPv6 goes
nowhere, not the code: compare the `in N ms` lines with older logs, then
`curl -4` and `curl -6` to the host. av-http's IPv6 fast fallback
(`architecture.md#crates`) covers the usual case.

A vault's scan logs how many files came from the catalog ("N of M files
from the catalog"); a catalog that couldn't be opened or written is
logged, and "no such tokenizer: trigram" is a system SQLite older than
3.34. A file the app shows as it was before a change outside it:
Rebuild index on the Vault page, or deleting `…\Avelor\catalog`, has every
file read again.

## Measuring size <a id="measuring-size"></a>

`target/release` holds the exe the user packs with UPX by hand, so it
isn't rebuilt to measure. The `bloat` profile is the same code with a PDB:
`CARGO_PROFILE_BLOAT_INHERITS=release CARGO_PROFILE_BLOAT_DEBUG=1
CARGO_PROFILE_BLOAT_STRIP=none cargo build --profile bloat -p
avelor` (`build.rs` sees `PROFILE=release`). On
`target/bloat/avelor.pdb`, `llvm-pdbutil dump -symbols`
(`S_GPROC32` records carry a code size), `-section-contribs`, `-modules`
and `-publics` break it down, `llvm-size -A` gives the sections; a change
is judged by diffing the per-function sizes of two PDBs.

## Line endings

Line endings are LF everywhere (`.gitattributes`).
