# Avelor — Architecture

> The entry point to the documentation: what the app is, the constraints
> and principles it is built on, how crates are structured, build and
> conventions — and the map of
> the other documents.

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
| Minimal dependencies | Only `serde`, `log` (+ `embed-resource`, `static_vcruntime` at build time) |
| Small executable | release profile for size (`architecture.md#build`), no async runtime, PNG icon frames; new code and dependencies are measured with a `bloat` profile (`development.md#measuring-size`) |

## Documents <a id="documents"></a>

| Document | Covers |
|---|---|
| **`architecture.md`** | this file: what the app is, principles, structure, build, conventions |
| `development.md` | checks, measuring size, line endings |

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
- **`file:line`**: host files are under `app/src/`, crate files under
  `crates/<crate>/src/` (written `av-vault/vault.rs:201`).
- **Tables are unaligned** (`|---|`); the docs are LF.

## Principles <a id="principles"></a>

The rules the code follows, and every change to it:

1. **Keep the stack, add no dependency.** Rust, hand-written utilities, no
   async runtime.
6. **Pure core, thin shell.** Mapping and decision logic are pure functions
   with tests; I/O sits at the edges.

## Crates <a id="crates"></a>

| Crate | Owns | Notes |
|---|---|---|
| av-utils | shared `Error`, `DateTime`, logger | |

## Build <a id="build"></a>

`app/build.rs` embeds the manifest and icon, on Windows. On
Linux the desktop entry and the icon are files (`app/res`), installed as
`README.md` says.

The release profile is built for size: `opt-level = "z"`, LTO, one codegen
unit, `strip`, `panic = "abort"` (`Cargo.toml`); the panic hook
`log_panics` (`logger.rs`) logs thread, message and location before the
abort; how to measure the size: `development.md#measuring-size`.

## Conventions <a id="conventions"></a>

- **Errors**: one `av_utils::Error` for every layer (`av-utils/error.rs`:
  logging), extended when needed; a separate error type only for functions
  local to one module; expected outcomes are success variants, not errors.
- **Reuse before writing**: `DateTime::from_system_time`. A new shared
  helper replaces every existing copy of its pattern in the same change.
- **Formatting**: Rust code is rustfmt-formatted; `cargo fmt --all` is
  fine on any file at any time.

## Ownership

| Part | Owns | Must not |
|---|---|---|
| av-utils | primitives with no domain (dates) | know about vaults, Google, LLMs; carry network types |
