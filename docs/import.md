# Avelor — Import from folder watches

> Folder watches and importing from them: the watch files, the Folders
> page, Import scans. The commands are `api.md#watches` and
> `api.md#scans`; the `scan:` source, `vault.md#scan`.

## Overview

A folder watch names a folder on this PC, the files to take from it
(patterns, subfolders) and how to read them (its type).
Importing turns those files into vault entities:

1. **List**: the files in the folder that match the watch, with what is
   known about each (date, size), and whether it is already in the vault.
2. **Read**: turn each file into text. Images need OCR.
3. **Arrange**: group the read files into documents (several scanned pages
   are one letter), order their pages, and give each a title.
4. **Import**: write the documents into the vault, each with a source block
   naming the watch and its files, so a later listing knows them.

Nothing is imported without the user pressing Import; reading may cost money
(an AI provider) or time (a local model), so it only runs on the files the
user chose.

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
    reading images (`watches::READ_STEP`). A step without an entry has no
    pick yet
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

- **Files in folder** lists the watch's files that aren't read yet and
  aren't in the vault: the files in its folder (and subfolders, if the
  watch says so) whose names match one of its patterns (`*` and `?`, in
  any case), with their date and size. A file's date is when it was made:
  its creation time, or its modified time if that is earlier (a copied
  file keeps its modified time). The section counts the files already in
  the vault and says when the folder was listed; the folder can be listed
  again.
- **Previews**: the selected file of "Files in folder" (a click on its row
  selects it; the first one at first) is shown large beside the list, and
  each document in "Ready for import" has a strip of its pages' thumbnails
  (three at a time, with arrows to the others) beside its page rows. A
  click on a thumbnail or a page row makes that page the document's
  current one, marked in both. A double-click on a preview opens the file
  in its default app (the image viewer). Previews are upright by the
  image's EXIF orientation; a file Windows (on Linux gdk-pixbuf) can't
  decode keeps a striped box ("no preview").
- **Read** sends a file to the model the watch picked for reading it
  (`models.read`, picked in the page's foot, "Read by", or in the
  watch's pane; `accounts.md#model-choice`),
  with the instruction to transcribe the page into Markdown, one file after
  the other, as a job in the jobs panel that can be stopped. The page shows
  each file as queued, then the words read so far. A file that fails shows
  why, and can be read again. Only JPEG, PNG, GIF and WebP images are read.
  What a file read to is kept while the app runs, as long as the file
  doesn't change; reading it again replaces it.
- **Ready for import** groups the read pages into documents: with the
  watch's Collect "taken within 2 min", a page joins the document with a
  page taken within two minutes of it, in the order they were taken; with
  "one per image", every page is a document of its own. Documents go by
  their first page. The user arranges them: drag a page before another
  one, onto a document's head (to its end), or onto "Drop here to make it a
  document of its own"; the arrows move a document's first page to the
  document before, its last page to the document after; Split makes a
  document of every page. Each document's title is proposed from its first
  page's text (its first heading, else its first line), and can be edited.
  Read reads a document's pages again.
- **Import** (per document, or Import all for the ticked ones, also
  Ctrl+Enter) writes each document to `Scans/` in the vault: a `document`
  whose text is its pages' text in order, a blank line between them, dated
  by its first page, with the source `scan: { watch, origin: [files] }`; an
  empty title takes the first page's, else the first file's name. A
  document whose pages aren't read (or changed since), are no longer in the
  folder or are in the vault already, isn't written and shows why.
  Imported documents leave the page; their files count as in the vault
  from then on.

Escape closes the page; opening it again for the same watch lists the
folder again and keeps the documents as arranged.

## Flows

| # | Flow | Trigger → page | Host → events | Result |
|---|---|---|---|---|
| J1 | Add a watch | "Watch a folder" → type → folder typed or Browse (`pick_folder` → `ok {path}`) → `watchSaveFolder` | `create_watch {type, folder, …}` → folder checked → `.avelor/watches/watch_<type>[_n].yaml` → `watch`, `ok {id}` | watch card |
| J2 | Edit | name, folder, subfolders, patterns, collect, the models (pickers) → `watchSet` (optimistic) | `set_watch {watch, …change}` → `watch`, `null` | — |
| J3 | Remove | two clicks within 3 s | `remove_watch {watch}` → `watch_removed`, `null` | imports stay |
| K1 | Open Import scans | the watch pane's Import → `openScanImport` (`features/import/scans.js`) | `scan_list {watch}` → `ok {listed, files, job}`; `scan_previews` → `scan_preview` per file | file list, large preview |
| K2 | Read (OCR) | per file or Read all → `scanRead` | `scan_read {watch, files, again}` → the host checks the watch's OCR model → `ok {job}`; one file after another on workers → `scan_page` queued / reading (words) / read (words, title) / failed; job events; `account` | text per page |
| K3 | Stop reading | Stop (or the job card's Stop) | `cancel_job {id: job}` → remaining pages `stopped`, job ends | — |
| K4 | Arrange | drag pages, merge with previous/next, split, edit titles, tick | page only (`scanPlace`, `features/import/scans.js`) | documents as arranged |
| K5 | Import | Import, Import all, Ctrl+Enter → `scanImport` | `scan_import {watch, docs: [{title, files}]}` → `entity` per document, `ok {imported, failed}` | documents in `Scans/` |
| K6 | Open original | double-click a preview | `scan_open {watch, file}` → `on_ui` → ShellExecute (a `FileLauncher` on Linux) | default image viewer |

## Design <a id="design"></a>

- **Listing**: `watches.rs` matches a watch's files (patterns,
  subfolders, globbing); `scan_list` lists on a worker.
  A file counts as in the vault by its watch and origin (`scan.watch` +
  `scan.origin` in the index).
- **Previews**: the host renders a preview per file with WIC (Windows
  Imaging Component, through the `windows` crate: no new dependency,
  `sys/win32/images.rs`; on Linux gdk-pixbuf, which comes with GTK,
  `sys/linux/images.rs`): decoded, turned upright by its EXIF orientation,
  scaled down to fit 360 x 508 (twice the large preview, sharp on a high-DPI
  screen), encoded as JPEG (about 80 KB for a scanned page) and sent as a
  `data:` URL. One preview serves both sizes. Previews are made on a
  worker, the file shown large first, and kept per file while it doesn't
  change (size and modified time), like the read text; a newer request
  stops what is left of the one before. The page keeps one `<img>` per
  file and moves it into each render, so a preview is decoded once.
- **Reading**: OCR is a job (`App::run_job_with_progress`, `jobs.rs`)
  streaming from av-llm with `Part::Inline` images; the words so far reach
  the page coalesced (`assistant.md#design` shows the shared streaming
  shape). The watch's pick is checked once (`AiAccounts::choose`),
  the spend limit checked before each page (`check_limit`). The read text
  is kept in memory while the app runs. Ollama runs the model with a
  context of 16,384 tokens (`ai::CONTEXT_TOKENS`, shared with the mail
  analysis, so the model isn't loaded anew between them): a scanned page
  alone came to 4,118 tokens, which its default of 4,096 refuses.
- **Importing**: `scans::gather_pages` gathers the pages (the plan), and
  `scans::scan_document` turns pages into a document — a pure function
  with tests — written through the vault edit. The import has no run step:
  its pages are read (OCR) before the import, which leaves nothing for a
  worker to do. The import stays on the UI thread.
