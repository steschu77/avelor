# Avelor — Development and testing

> How to check a change, find a problem, measure the executable. The rules the code
> follows are `architecture.md#principles` and `architecture.md#conventions`.

## Checks <a id="checks"></a>

- `cargo test --workspace` runs the unit tests and every feature module's
  dispatch tests (`testing.rs`: `Harness`, `call`).
  av-vault's `forged_vault_loads_with_multi_roles` indexes `data/forged`, so
  every file there must load.
- The docs (`app/tests/docs.rs`, `architecture.md`, Doc conventions): in
  the docs, the README and the code's comments a reference
  (`saving.md#conflicts`) names an id at the end of a heading of its doc;
  in a doc an id alone in backticks is a section of that doc, or else an
  element of the page (an `id` in its HTML, an `.id` a module sets); a
  file name in backticks (`.rs`, `.js`, `.css`, `.html`, with a `:line` or
  not) is the end of a file's path in the repository, `src/` left out
  (`av-vault/envelope.rs`); a link names a file there. An id is at most 16
  characters, unique in its doc and referred to; the docs are LF.
- The page's modules (`app/tests/page.rs`): each test file of
  `app/tests/page`, bundled with the modules it imports as the page is, is
  run by node in three time zones. A test takes what a module exports; the
  stand-in for the page is `app/tests/page/harness.js`. A text is its key
  there, so no wording breaks a test. Without node the tests are left out,
  but not in CI.
- The build bundles the page's modules and checks them
  (`development.md#the-bundled-page`); `av-bundle`'s rules have their own
  tests. It checks the catalogs too (`localization.md#check`), with
  `av-texts`' rules tested the same way.
- `cargo clippy --workspace`.
- The user tests new flows by hand against `data/forged`.

## The bundled page <a id="the-bundled-page"></a>

`build.rs` bundles the page into
`target/debug/build/avelor/<hash>/out/index.html`. Several hash
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
(`core/dom.js imports from ui/log.js, a higher layer (ui/)`). So does an
export no module of the page imports (`core/x.js exports b, which no
module imports`), and a non-ASCII character in any page source, a module, a stylesheet or an HTML file
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
is ending the session".

On Linux, `kill -TERM` or `kill -HUP` with the app's process id ends the
session as a logout's signal does: the log shows "SIGTERM: the session
ends" and the window's size is in `config.yaml`. Ctrl+C in the terminal
closes the window. The session manager's `query-end` needs a desktop
session; WSLg has none.

## Diagnosis <a id="diagnosis"></a>

Read the log first: `%LOCALAPPDATA%\Avelor\log` (on Linux
`$XDG_STATE_HOME/avelor/log`), one file per run
(`av-utils/logger.rs`), times in UTC; the panic hook logs a panic's
thread, message and location before the abort.

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
