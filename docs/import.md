# Avelor — Import from folder watches

> Folder watches and importing from them: the watch files, the Folders
> page, Import scans and PDFs, reading a PDF, account statements. The
> commands are `api.md#watches`, `api.md#scans` and `api.md#statements`;
> the `scan:` source, `vault.md#scan`; a statement, `vault.md#transactions`.

## Overview

A folder watch names a folder on this PC, the files to take from it
(patterns, subfolders) and how to read them (its type).
Importing turns those files into vault entities:

1. **List**: the files in the folder that match the watch, with what is
   known about each (date, size), and whether it is already in the vault.
2. **Read**: turn each file into text. Images need OCR; a PDF has its
   text, or scanned pages that need it.
3. **Arrange**: group the read files into documents (several scanned pages
   are one letter), order their pages, and give each a title. A PDF is a
   document of its own.
4. **Import**: write the documents into the vault, each with a source block
   naming the watch and its files, so a later listing knows them.

Nothing is imported without the user pressing Import; reading may cost money
(an AI provider) or time (a local model), so it only runs on the files the
user chose. An account statements watch reads its files as they are, each
into the watch's bank account (`#statements`).

## Folder watches <a id="folder-watches"></a>

Watches belong to a vault, like accounts: the documents a watch imported
name it by its `id` (`scan.watch`). Opening another vault switches to its
watches (`folders::open`, on every vault open), and without an open vault
there are none. Each enabled watch is listed in the vault panel's Sources
under its name, a statements watch under its account's; the source counts
and filters the entities imported from it, a statements watch's its
account's statements, whichever watch imported them. Its Import button,
like the one on the watch's pane, opens Import scans for an images watch,
and imports a statements watch's new files into its account
(`#statements`).

### Watch files <a id="watch-files"></a>

Each watch is a YAML file in the vault's `.avelor/watches/` folder, named
`watch_<type>.yaml`, then `watch_<type>_2.yaml`, …. It is read as an
account file is (`accounts.md#vault-accounts`). The file contains:

* `id`: UUIDv7, unique identifier for the watch
* `folder-watch`:
  * `type`: what the watch takes, chosen when it is added: `images` (scanned
    pages, OCR; several images can be one document), `pdf` (one document
    per PDF, read as `#reading-pdfs` says) or `statements` (a bank
    account's CSV statements, `#statements`)
  * `name`: shown in the sidebar; absent: the folder's name. A statements
    watch shows its account's title instead
  * `folder`: as entered; `%VAR%`s such as `%USERPROFILE%` are expanded
    where it is used (on Linux a leading `~` and `$NAME`s). A folder
    picked with Browse under the user's profile is written as
    `%USERPROFILE%\…` (on Linux `~/…`; `sys::collapse_env`). Absent: not
    chosen yet, which only a `statements` watch may be; it imports nothing
    until it is
  * `subfolders`: `true` if files in subfolders are taken too (absent:
    only the folder's own files)
  * `patterns`: file name patterns (`*.jpg`); a file matching any of them
    is taken
  * `collect`: an `images` watch's only: `together` (images taken within 2
    minutes of each other are the pages of one document) or `single` (one
    document per image)
  * `account`: a `statements` watch's only: the id of the bank account
    (`vault.md`, Bank Account) its files go into; no other watch has it
  * `models`: the model each step that sends a file to an AI picked,
    `{account, model}` (`accounts.md#model-choice`), by step: `read` for
    reading images and scanned pages (`watches::READ_STEP`). A step without
    an entry has no pick yet
  * `analysis`: the analysis of the documents it imports, `level` (`off`,
    `lite`, `medium`) and the `model` it runs with, as a mail filter's
    (`mail.md#analysis`), the model kept while the level is off; absent:
    off
  * `keep-original`: `true` if a document keeps its file beside it (one
    of several images a PDF of them, `vault.md#document`); absent: it
    doesn't, but an encrypted PDF always does
  * `pair-transfers`: a `statements` watch's only: `false` if its imports
    don't pair transfers (`#statements`); absent: they do
  * `enabled`: `false` while the watch is off; it isn't listed under
    Sources then
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
  analysis:
    level: lite
    model:
      account: 019ff8a6-aa60-7e31-ab4b-3e861cb75fe1
      model: gemini-3.8-flash
  keep-original: true
  added: 2026-01-03T10:00:00Z
```

### The Folders page <a id="the-folders-page"></a>

Workspace → Folders (`features/workspace/folders.js`) lists the open vault's
watches and adds new ones (with no vault open, the page says that
watches are kept in the
vault), following the Accounts page:

- "Watch a folder" asks for the type first (Images, PDF), then for the
  folder, suggested per type (Pictures, Documents), typed or picked with
  Browse (`pick_folder`). The folder is looked for before the watch is
  saved; the new watch takes its type's file types (and Collect: taken
  within 2 min, for images).
- "Add a bank account" adds a bank account with its statements watch, the
  two listed under Bank accounts. Its pane asks for the account: a new
  one, whose name, number and kind (nothing picked at first) it needs and
  whose bank it may have; or one of the vault's no watch has. Then for the
  folder of its statements, typed or picked with Browse, which can wait:
  without one the watch is saved anyway, its folder field stays open and
  Import waits for it, on the pane and its source. Add account saves both,
  the watch with `*.csv`.
- A watch's pane changes its name, folder, subfolders, file types, whether
  it is a source, and its import process: the reading step, which always
  runs, then Analysis and Keep original, each switched on or off by its
  check. Analysis's check turns it on at Lite (or off, keeping its model);
  while it is on, it offers Lite or Medium. A step that sends the file to
  an AI has the picker of the model it runs with in its row
  (`accounts.md#model-choice`): the reading step (`models`), for a PDF
  watch only its scanned pages', and the analysis (`analysis`). A step
  without a pick shows "Pick a model".
  What keeps a picked model from working shows under its step, with a
  button to Accounts where the fix is there.
- A statements watch's pane shows its account instead of a name: its
  balance (and what is pending), its bookings and its newest statement;
  its name, which is the watch's (`set_title`), its bank, number and kind
  (`set_bank_account`), its currency, owner and file as they are. Then its
  folder, as any watch's, and the steps a file takes: Read the rows, Skip
  what is there and Check the balance, which always run, and Pair
  transfers, switched by its check (`pair_transfers`).
- Removing a watch deletes its file; what it imported stays in the vault,
  a statements watch's account too.

## Import scans <a id="import-scans"></a>

The Import button of a watch opens Import scans, or Import PDFs for a PDF
watch (`features/import/scans.js`), in the expanded vault area:

- **Files in folder** lists the watch's files that aren't read yet and
  aren't in the vault: the files in its folder (and subfolders, if the
  watch says so) whose names match one of its patterns (`*` and `?`, in
  any case), with their date and size. A file's date is when it was made:
  its creation time, or its modified time if that is earlier (a copied
  file keeps its modified time). The section counts the files already in
  the vault and says when the folder was listed; the folder can be listed
  again. The files in the vault are hidden; "Show the N in the vault"
  lists them too, to read them again: read again, a file's pages replace
  its document's text on import.
- **Previews**: the selected file of "Files in folder" (a click on its row
  selects it; the first one at first) is shown large beside the list, and
  each document in "Ready for import" has a strip of its pages' thumbnails
  (three at a time, with arrows to the others) beside its page rows. A
  click on a thumbnail or a page row makes that page the document's
  current one, marked in both. A double-click on a preview opens the file
  in its default app (the image viewer). Previews are upright by the
  image's EXIF orientation; a file Windows (on Linux gdk-pixbuf) can't
  decode keeps a striped box ("no preview"), and so does a PDF.
- **Read** sends a file to the model the watch picked for reading it
  (`models.read`, picked in the page's foot, "Read by", or in the
  watch's pane; `accounts.md#model-choice`),
  with the instruction to transcribe the page into Markdown (`reader.rs`,
  which reads a mail's attachments too, `mail.md#attachments`), one file
  after the other, as a job in the jobs panel that can be stopped. The
  page shows each file as queued, then the words read so far. A file that
  fails shows why, and can be read again. An images watch reads JPEG, PNG,
  GIF and WebP images only, a PDF watch PDFs only (`#reading-pdfs`): their
  text without a model, so Read works without a pick, which only the
  scanned pages need; an encrypted PDF reads as "encrypted", without
  text.
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
  Read reads a document's pages again. A PDF watch's documents are a PDF
  each, not arranged: no dragging, arrows or Split.
- **Import** (per document, or Import all for the ticked ones, also
  Ctrl+Enter) writes each document to `Scans/` in the vault: a `document`
  whose text is its pages' text in order, a blank line between them, dated
  by its first page, with the source `scan: { watch, origin: [files] }`; an
  empty title takes the first page's, else the first file's name (the
  page sends a title only as edited). With Keep original, a document of
  one file (an image, a PDF) keeps it beside it as its original
  (`vault.md#document`), named as the file; one of several images keeps a
  PDF of them, a page each as it is (`av_pdf::image_pages`: a JPEG as it
  is, a PNG's image data as it is; sized by its density, else fitted into
  A4, a JPEG turned by its EXIF orientation), named after the first file;
  one with another image (WebP, GIF, a PNG with alpha) keeps none. An
  encrypted PDF always keeps its file, marked `encrypted: true`,
  and its body stays empty. A document whose pages aren't read (or
  changed since), are no longer in the folder or are in the vault already,
  isn't written and shows why. Imported documents leave the page; their
  files count as in the vault from then on.
- **Import again**: pages in the vault, read again, form a document of
  their own (an images watch joins them only with pages of the same
  document), marked "replaces “<title>”" and shown with the title the
  document has. Its Import, and Import all with it, takes a second click
  (`armedButton`, as Fetch mail's Replace); a note says the texts are
  replaced and edits to them lost. Importing replaces the document's text
  (`replace`), only when its pages are all of the document's own
  (`scan.origin`), else it shows why; its frontmatter and kept file stay,
  a title typed replaces its title, the encrypted mark follows the new
  read, and the analysis runs on it again.
- **Analysis**: with the watch's analysis on, the documents imported go to
  "Analysing scans · <watch>" ("Analysing PDFs · <watch>" for a PDF watch;
  `enrich::analyse`), the mail analysis's job
  and request for a document (`mail.md#analysis`): a clean title, the
  date the document carries, a genre, labels and projects, written at
  once; a title typed before the import stays. At Medium a document also
  gets its points to check and questions, as a chat "Document analysis ·
  <title>" linked to it, as no mail carries them. A document without text
  (an encrypted PDF) is left out. While the analysis is on without a
  model that works, nothing is read or imported, as Fetch mail doesn't
  fetch: Read and Import are off, with the note "The analysis is on: pick
  a model that works for it in the watch's pane, or turn it off.", and
  the host refuses `scan_read` and `scan_import` with the pick's
  reason.

Escape closes the page; opening it again for the same watch lists the
folder again and keeps the documents as arranged.

## Reading PDFs <a id="reading-pdfs"></a>

A PDF, a mail's attachment (`mail.md#attachments`) or a PDF watch's file
(`#import-scans`), is read through `reader.rs` and `av-pdf`
(`architecture.md#crates`), page by page:

- **Text layer**: a page whose text layer has at least 20 letters, at most
  one glyph in ten without its character, is that text, laid out in lines
  and paragraphs by position, as plain text. A page with a little text and
  no large image (a cover sheet) is text too; a blank page is skipped.
  When every page is text or blank, the file is read from its text layer
  without a model: free, at once, and nothing leaves the PC.
- **Scans by the model**: otherwise a page without a text layer whose
  largest image is a JPEG covering at least half of it (a scan, often
  fitted with a margin: its `DCTDecode` stream as it is) is read by the
  model as an image, a request per page, and a text page as it is; the
  pages follow each other, a blank line between them. A text page never
  leaves the PC.
- **The whole file**: a PDF with a page that is neither (an image of
  another kind, drawings, text whose characters its fonts don't name), or
  that can't be parsed, is sent whole to a model that takes PDFs (every
  provider but Ollama, `ProviderSpec::pdf`); so is one with a scan larger
  than 3.75 MB (Anthropic takes images up to 5 MB as base64). On Ollama
  such a PDF fails, naming the page.
- **Encrypted**: an encrypted PDF is sent to no model. Its original is
  marked (`encrypted: true`, `vault.md#document`), its body stays empty,
  and its card's File row says "encrypted".

## Account statements <a id="statements"></a>

A statements watch imports the CSV exports of one bank account
(`bank.rs`). Import, on the watch's source or pane, lists the watch's files
that aren't in the vault and reads them on a worker, then writes each into
the account, the oldest first, as a job "Importing statements ·
<account>"; its note counts the statements and bookings that came in and
the transfers paired, and names each file left out and each balance that
doesn't add up. A file counts as in the vault by its account and origin:
the `import.origin` of a statement of the account (`vault.md#import`),
whichever watch imported it.

- **The format** is comdirect's export of an account's bookings
  ("Umsätze"): UTF-8 or Windows-1252; quoted cells, each followed by `;`;
  the balance before and after the bookings ("Alter Kontostand", "Neuer
  Kontostand", with the currency); the rows after the header row
  (Buchungstag, Wertstellung (Valuta), Vorgang, Buchungstext, Umsatz in …)
  up to a blank line, newest first; days as `dd.mm.yyyy`, `offen` for a
  pending card payment; amounts as `-1.927,35`. A file without the header
  row, or with a row that can't be read, is left out.
- **A row** is a booking. Its Buchungstext names who (`Auftraggeber:`, or
  `Empfänger:` with their `Kto/IBAN:` and `BLZ/BIC:`), the purpose after
  `Buchungstext:`, and the bank's reference (`Ref.`). A card payment's
  purpose is its merchant line, followed by the time the card was used
  (and, once booked, `KFN` and `VJ`); a pending row names the account's
  own number. A transfer's or direct debit's purpose comes cut into lines
  of 35 characters joined by a space; they are joined again. The kind
  follows the Vorgang and the card's text: `card`, `direct-debit`,
  `transfer`, `cash` (Auszahlung GAA), `fee` (Kontoführungsentgelt).
- **Another account's** file, whose pending rows name another number than
  the account's (compared by their digits; one may end with the other, as
  an IBAN ends with the account number), is left out.
- **What is there** is left out: a booked row whose reference the account
  has, or without one, a row of the same day, amount and purpose; a file
  with nothing else writes no statement. The statement whose last booking
  is the account's newest replaces the account's pending bookings: they
  leave the older statements, and a new booking of a removed one's card
  day and amount takes its `id` and `linked-to`. An older statement
  brings no pending bookings.
- **Transfers**: a booked booking out whose `iban` names another bank
  account of the vault (by the number's digits, as an IBAN ends with it)
  and a booked booking of the same amount into that account, at most 3
  days apart, are the two halves of a transfer: each names the other
  (`transfer`, `vault.md#transactions`), the nearest day first, each half
  once. Whichever statement comes second pairs them, and the other
  account's statement is written too. The watch's Pair transfers step
  turns this off; the pairs made stay.
- **The balances**: the balance before plus the booked rows is the balance
  after; and the balance before is the one the statement before it ends
  with, if that one ends before this one starts. Otherwise the job's note
  names the file.
- **Written**: a transactions entity per file in `Accounts/`
  (`vault.md#transactions`), titled "<account>, December 2026" (across
  months "Nov – Dec 2026"), dated by its first booking, its bookings oldest
  first, the pending ones last. The account takes the balance of its
  newest statement, and the file's currency while it has none.

## Flows

| # | Flow | Trigger → page | Host → events | Result |
|---|---|---|---|---|
| J1 | Add a watch | "Watch a folder" → type (Images, PDF) → folder typed or Browse (`pick_folder` → `ok {path}`) → `watchSaveFolder` | `create_watch {type, folder, …}` → folder checked → `.avelor/watches/watch_<type>[_n].yaml` → `watch`, `ok {id}` | watch card and source |
| J2 | Edit | name, folder, subfolders, patterns, source on/off, collect, the models (pickers) → `watchSet` (optimistic) | `set_watch {watch, …change}` → `watch`, `null` | — |
| J3 | Remove | two clicks within 3 s | `remove_watch {watch}` → `watch_removed`, `null` | imports stay |
| J4 | Add a bank account | "Add a bank account" → a new account's name, number, kind, bank, or a vault's account picked (`renderNewAccountPane`) → folder typed, Browse or none → Add account (`addAccount`) | `create_watch {type: statements, folder?, patterns, account \| new_account}` → folder checked if given → the new account's fields checked, it is written (`entity`) → `.avelor/watches/watch_statements[_n].yaml` → `watch`, `ok {id}` | the account's card and source |
| K1 | Open Import scans | the watch source's Import or the watch pane's Import → `openScanImport` (`features/import/scans.js`) | `scan_list {watch}` → `ok {listed, files, job}`; `scan_previews` → `scan_preview` per file | file list, large preview |
| K2 | Read | per file or Read all → `scanRead` | `scan_read {watch, files, again}` → the host checks the watch's model (a PDF watch reads on without one) → `ok {job}`; one file after another on workers through `reader::read` → `scan_page` queued / reading (words) / read (words, title, encrypted) / failed; job events; `account` | text per page |
| K3 | Stop reading | Stop (or the job card's Stop) | `cancel_job {id: job}` → remaining pages `stopped`, job ends | — |
| K4 | Arrange | drag pages, merge with previous/next, split, edit titles, tick | page only (`scanPlace`, `features/import/scans.js`) | documents as arranged |
| K5 | Import | Import, Import all, Ctrl+Enter → `scanImport` | `scan_import {watch, docs: [{title, files}]}` → `entity` per document, `ok {imported, failed}`; with the watch's analysis on, the job "Analysing scans" or "Analysing PDFs" (G5's steps) → `entity`, `job`, `account` | documents in `Scans/`, analysed; one file kept beside its document |
| K6 | Open original | double-click a preview | `scan_open {watch, file}` → `on_ui` → ShellExecute (a `FileLauncher` on Linux) | default image viewer |
| M1 | Import statements | a statements watch's source or pane, once its folder is chosen: Import → `importWatch` (`features/workspace/folders.js`) | `statements_import {watch}` → `ok {job}` (`invalid_argument` without a folder); the files listed and read on the job's worker → per file `entity` (the statement, the older ones it took pending bookings from, the other accounts' statements holding a transfer's other half, the account) → `job` with the note | statements in `Accounts/`, the account's balance, its transfers paired |
| M2 | Change the account | the pane's name, Bank, Number, Kind | `set_title {id, title}`; `set_bank_account {entity, institution?, number?, type?}` → `entity`, `ok` | the account, the watch's name |

## Design <a id="design"></a>

- **Listing**: `watches.rs` matches a watch's files (patterns,
  subfolders, globbing); `scan_list` lists on a worker.
  A scan counts as in the vault by its watch and origin (`scan.watch` +
  `scan.origin`, `Index::scan_origins`), a statement by its account and
  origin (`import.origin`, `bank::account_origins`).
- **Statements**: the reader (`bank::read`), what a statement adds to its
  account (`bank::plan`) and the transfers it pairs (`bank::pairs`) are
  pure functions with tests, the reader's
  on the December statement in `data/statements`; the files
  are listed and read on the job's worker, and written on the UI thread,
  one vault edit per entity.
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
- **Reading**: reading is a job (`App::run_job_with_progress`, `jobs.rs`)
  through `reader::read`, streaming from av-llm with `Part::Inline` images
  where a model reads; the words so far reach the page coalesced
  (`assistant.md#design` shows the shared streaming shape). The watch's
  pick is checked once (`AiAccounts::choose`; a PDF watch without a usable
  one reads with `Engine::None`, its text layers only), the spend limit
  checked before each file (`check_limit`). The read text is kept in
  memory while the app runs. Ollama runs the model with a context of
  16,384 tokens (`ai::CONTEXT_TOKENS`, shared with the mail analysis, so
  the model isn't loaded anew between them): a scanned page alone came to
  4,118 tokens, which its default of 4,096 refuses.
- **Importing**: `scans::gather_pages` gathers the pages (the plan), and
  `scans::scan_document` turns pages into a document — a pure function
  with tests — written through the vault edit. The import has no run step:
  its pages are read (OCR) before the import, which leaves nothing for a
  worker to do. The import stays on the UI thread.
