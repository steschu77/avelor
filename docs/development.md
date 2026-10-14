# Avelor — Development and testing

> How to check a change, find a problem, measure the executable. The rules the code
> follows are `architecture.md#principles` and `architecture.md#conventions`.

## Checks <a id="checks"></a>

- `cargo test --workspace` runs the unit tests.
- `cargo clippy --workspace`.

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
