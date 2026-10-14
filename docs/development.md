# Avelor — Development and testing

> How to check a change, measure the executable. The rules the code
> follows are `architecture.md#principles` and `architecture.md#conventions`.

## Checks <a id="checks"></a>

- `cargo test --workspace` runs the unit tests.
- `cargo clippy --workspace`.

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
