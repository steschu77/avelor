# Avelor — Architecture

> The entry point to the documentation: what the app is, the constraints
> and principles it is built on, build and conventions — and the map of
> the other documents.

## What the app is

Avelor manages personal documents in a **vault**: a folder of plain
markdown and YAML files that the user owns. It
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
| Small executable | release profile for size (`architecture.md#build`), no async runtime, PNG icon frames; new code and dependencies are measured with a `bloat` profile (`development.md#measuring-size`) |

## Documents <a id="documents"></a>

| Document | Covers |
|---|---|
| **`architecture.md`** | this file: what the app is, principles, build, conventions |
| `development.md` | checks, measuring size, line endings |

## Doc conventions

- **One owner per fact**: each fact is described in one document; the
  others link to it.
- **References** point at an id: `` `development.md#checks` `` in the
  docs, `` `#checks` `` alone within `development.md` itself (a `` `#x` ``
  that names no section of its doc is an element id of the page), and the
  full form, ASCII, in code comments (`development.md#checks`; every `.md`
  file is in `docs/`).
  A section something refers to carries its id at the end of its heading
  line: `` ## Conflicts <a id="conflicts"></a> ``. An id is a lowercase
  word or two, at most 16 characters, unique within its file and never
  reused for another section, so a heading can be reworded without
  touching a reference. Headings carry no numbers. A test checks them
  (`development.md#checks`).
- **`file:line`**: host files are under `app/src/`.
- **Tables are unaligned** (`|---|`); the docs are LF.

## Principles <a id="principles"></a>

The rules the code follows, and every change to it:

1. **Keep the stack, add no dependency.** Rust, no async runtime.

## Build <a id="build"></a>

`app/build.rs` embeds the manifest, the icon and the version (a
VERSIONINFO from the workspace's version), on Windows; a manifest whose
version isn't the workspace's fails the build. On Linux the desktop entry
and the icon are files (`app/res`), installed as `README.md` says.

The release profile is built for size: `opt-level = "z"`, LTO, one codegen
unit, `strip`, `panic = "abort"` (`Cargo.toml`); how to measure the size:
`development.md#measuring-size`.

## Conventions <a id="conventions"></a>

- **Formatting**: Rust code is rustfmt-formatted; `cargo fmt --all` is
  fine on any file at any time.
