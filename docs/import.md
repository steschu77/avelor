# Avelor — Import from folder watches

> Folder watches and importing from them: the watch files, the Folders
> page, Import scans. The commands are `api.md#watches` and
> `api.md#scans`; the `scan:` source, `vault.md#scan`.

## Overview

A folder watch names a folder on this PC, the files to take from it
(patterns, subfolders) and how to read them (its type).

## Folder watches <a id="folder-watches"></a>

Watches belong to a vault, like accounts: the documents a watch imported
name it by its `id` (`scan.watch`). Opening another vault switches to its
watches (`folders::open`, on every vault open), and without an open vault
there are none.

### Watch files <a id="watch-files"></a>

Each watch is a YAML file in the vault's `.avelor/watches/` folder, named
`watch_<type>.yaml`, then `watch_<type>_2.yaml`, …. It is read as an
account file is (`accounts.md#vault-accounts`). The file contains:

* `id`: UUIDv7, unique identifier for the watch
* `folder-watch`:
  * `type`: what the watch takes, chosen when it is added: `images` (scanned
    pages, OCR; several images can be one document)
  * `name`: shown in the sidebar; absent: the folder's name
  * `folder`: as entered; `%VAR%`s such as `%USERPROFILE%` are expanded
    where it is used (on Linux a leading `~` and `$NAME`s). A folder
    picked with Browse under the user's profile is written as
    `%USERPROFILE%\…` (on Linux `~/…`; `sys::collapse_env`).
  * `subfolders`: `true` if files in subfolders are taken too (absent:
    only the folder's own files)
  * `patterns`: file name patterns (`*.jpg`); a file matching any of them
    is taken
  * `collect`: `images` only: `together` (images taken within 2 minutes of
    each other are the pages of one document) or `single` (one document
    per image)
  * `models`: the model each step that sends a file to an AI picked,
    `{account, model}` (`accounts.md#model-choice`), by step: `read` for
    reading images. A step without an entry has no pick yet
  * `enabled`: `false` while the watch is off
  * `added`: when the watch was added

```yaml
id: 0193a7c1-4b0e-7c1a-9d2f-3a51c8e05be0
folder-watch:
  type: images
  name: Scanner
  folder: '%USERPROFILE%\Scans'
  patterns: ['*.jpg', '*.jpeg', '*.png']
  collect: together
  models:
    read:
      account: 019ff8a6-aa60-7e31-ab4b-3e861cb75fe1
      model: gemini-3.8-flash
  added: 2026-01-03T10:00:00Z
```

### The Folders page <a id="the-folders-page"></a>

Workspace → Folders (`features/workspace/folders.js`) lists the open vault's
watches and adds new ones (with no vault open, the page says that
watches are kept in the
vault), following the Accounts page:

- "Watch a folder" asks for the type first (Images), then for the folder,
  suggested per type, typed or picked with Browse (`pick_folder`). The
  folder is looked for before the watch is saved; the new watch takes its
  type's file types (and Collect: taken within 2 min, for images).
- A watch's pane changes its name, folder, subfolders, file types, and
  its import process: the reading step. A step that sends the file to an
  AI has the picker of the model it runs with in its row
  (`accounts.md#model-choice`): the reading step of images (`models`). A
  step without a pick shows "Pick a model". What keeps a picked model from
  working shows under its step, with a button to Accounts where the fix
  is there.
- Removing a watch deletes its file; what it imported stays in the vault.

## Import scans <a id="import-scans"></a>

The Import button of an images watch opens Import scans
(`features/import/scans.js`) in the expanded vault area:

- **Files in folder** lists the watch's files that aren't in the vault: the
  files in its folder (and subfolders, if the
  watch says so) whose names match one of its patterns (`*` and `?`, in
  any case), with their date and size. A file's date is when it was made:
  its creation time, or its modified time if that is earlier (a copied
  file keeps its modified time). The section counts the files already in
  the vault and says when the folder was listed; the folder can be listed
  again.

Escape closes the page; opening it again for the same watch lists the
folder again.

## Flows

| # | Flow | Trigger → page | Host → events | Result |
|---|---|---|---|---|
| J1 | Add a watch | "Watch a folder" → type → folder typed or Browse (`pick_folder` → `ok {path}`) → `watchSaveFolder` | `create_watch {type, folder, …}` → folder checked → `.avelor/watches/watch_<type>[_n].yaml` → `watch`, `ok {id}` | watch card |
| J2 | Edit | name, folder, subfolders, patterns, collect, the models (pickers) → `watchSet` (optimistic) | `set_watch {watch, …change}` → `watch`, `null` | — |
| J3 | Remove | two clicks within 3 s | `remove_watch {watch}` → `watch_removed`, `null` | imports stay |
| K1 | Open Import scans | the watch pane's Import → `openScanImport` (`features/import/scans.js`) | `scan_list {watch}` → `ok {listed, files}` | file list |

## Design <a id="design"></a>

- **Listing**: `watches.rs` matches a watch's files (patterns,
  subfolders, globbing); `scan_list` lists on a worker.
  A file counts as in the vault by its watch and origin (`scan.watch` +
  `scan.origin` in the index).
