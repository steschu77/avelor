// Import scans (import.md#import-scans;
// api.md#scans): the import of an images folder watch, opened by the Import
// button of its source (features/sidebar.js) or of its pane (features/workspace/folders.js,
// through the `watch.import` action), and rendered
// into the expanded vault area while its op is "scans". The host lists the
// watch's files, reads the ones asked for with the model the watch picked
// for OCR (its `models`' `read`, picked here or in its pane) as a job, which Stop ends
// (`cancel_job`) and the account's spend limit stops, and writes the
// documents into `Scans/`. The page groups the read pages into
// documents, which the user arranges (drag and drop, merge, split) and
// titles before importing them. The files are shown as previews: the host
// renders them (`scan_previews`), and a double-click opens one in its
// default app (`scan_open`). On the page, Esc closes it and Ctrl+Enter
// imports the ticked documents (`registerVaultPage`).

import { accEl, oncePerFrame } from '../../core/dom.js';
import { rpcCall } from '../../core/rpc.js';
import { rpcOn } from '../../core/rpc.js';
import { TIME_FORMAT, pageDate, parseDate } from '../../core/format.js';
import { fileSize, plural } from '../../core/format.js';
import { registerAction } from '../../core/actions.js';
import { store } from '../../model/state.js';
import { watches } from '../../model/entities.js';
import { iconEl } from '../../ui/icons.js';
import { IMPORT_ICON } from '../../ui/icons.js';
import { clearDropMark, dropTarget } from '../../ui/dropzone.js';
import {
  closeVaultArea, openVaultOp, registerVaultPage, replaceVaultPage,
} from '../../ui/page.js';
import {
  aiMissingHint, aiModelOf, aiModelPicker,
} from '../workspace/models.js';
import {
  WATCH_READ_STEP, watchSetModel, watchSettings,
} from '../workspace/folders.js';

const SCAN_GROUP_MS = 2 * 60 * 1000; // pages taken this close are one document
const SCAN_STRIP = 3; // thumbnails a document's strip shows at once
const SCAN_READ_ICON = '#i-book-open';
const SCAN_SPLIT_ICON = '#i-cut';
const SCAN_LIST_ICON = '#i-arrow-sync';
const SCAN_BEFORE_ICON = '#i-chevron-left';
const SCAN_AFTER_ICON = '#i-chevron-right';

let scan = null; // the page's state, see openScanImport

// ----------------------------------------------------------------------------
// opening and closing

// Open the page for `watch_id` and list its files. The page of the same
// watch keeps its documents, as they were arranged.
export function openScanImport(watch_id) {
  if (scan?.watch !== watch_id) {
    scan = {
      watch: watch_id,
      listing: false, // while `scan_list` is being answered
      listed: null, // when the files were listed
      listError: null,
      // file -> { file, size, taken, in_vault, read: { words, title } | null,
      // status: 'queued' | 'reading' | 'failed' | null, words, error }
      files: new Map(),
      // [{ key, title (once edited), edited, pages: [file], checked, error }]
      docs: [],
      nextDoc: 1,
      job: null, // the job reading this watch's files, which Stop ends
      error: null, // a refused read, import or open
      importing: false, // while `scan_import` is being answered
      dragging: null, // the page being dragged; rendering waits for the drop
      current: null, // the file shown large in "Files in folder"
      // `file|size|modified` -> { url, img }: a file's preview once the host
      // sent it (`url` empty: none), its <img> made once
      previews: new Map(),
    };
  }
  openVaultOp('scans', 'Import scans');
  scanList();
}

registerVaultPage('scans', {
  close: closeVaultArea,
  submit: () => {
    if (scan) scanImport(scan.docs.filter(doc => doc.checked));
  },
});
registerAction({ id: 'watch.import', title: 'Import from the folder', run: openScanImport });

// ----------------------------------------------------------------------------
// host answers and events

// The files as listed (`scan_list`'s answer) replace the ones shown; a
// failed read stays shown until the file is read again.
function onScanFiles(listing) {
  scan.listing = false;
  scan.listError = null;
  scan.listed = parseDate(listing.listed);
  scan.job = listing.job;
  const files = new Map();
  for (const f of listing.files) {
    const before = scan.files.get(f.file);
    const failed = !f.status && before?.status === 'failed';
    files.set(f.file, {
      file: f.file,
      size: f.size,
      taken: parseDate(f.taken),
      modified: f.modified,
      in_vault: f.in_vault,
      read: f.read,
      status: f.status ?? (failed ? 'failed' : null),
      words: before?.words ?? 0,
      error: failed ? before.error : null,
    });
  }
  scan.files = files;
  scanArrange();
  scanRequestPreviews();
  scheduleRenderScans();
}

// A file's preview: a JPEG `data:` URL, or empty if it has none.
rpcOn('scan_preview', ({ watch, file, url }) => {
  const f = scan?.watch === watch ? scan.files.get(file) : null;
  if (!f) return;
  scan.previews.set(scanPreviewKey(f), { url, img: null });
  scheduleRenderScans();
});

// Where a file stands in a read: queued, reading (with the words so far),
// read, failed, or stopped.
rpcOn('scan_page', page => {
  const f = scan?.watch === page.watch ? scan.files.get(page.file) : null;
  if (!f) return;
  if (page.status === 'read') {
    Object.assign(f, { read: { words: page.words, title: page.title }, status: null, error: null });
    scanArrange();
  } else if (page.status === 'failed') {
    Object.assign(f, { status: 'failed', error: page.error });
  } else if (page.status === 'stopped') {
    f.status = null;
  } else {
    Object.assign(f, { status: page.status, words: page.words ?? 0, error: null });
  }
  scheduleRenderScans();
});

// Imported documents (`scan_import`'s answer) leave the page, their files
// now in the vault; a document that failed stays, with its error.
function onScanImported(reply) {
  for (const { files, id } of reply.imported) {
    for (const file of files) {
      const f = scanFile(file);
      if (f) f.in_vault = id;
    }
  }
  for (const { files, error } of reply.failed) {
    const doc = scan.docs.find(d =>
      d.pages.length === files.length && d.pages.every((page, i) => page === files[i]));
    if (doc) doc.error = error;
  }
  scanArrange();
  scheduleRenderScans();
}

// ----------------------------------------------------------------------------
// files and documents

function scanFile(file) {
  return scan.files.get(file);
}

// A page that belongs in a document: read, and not in the vault yet.
function scanReady(f) {
  return !!f?.read && !f.in_vault;
}

// Being read, or waiting for it.
function scanBusy(f) {
  return f?.status === 'queued' || f?.status === 'reading';
}

// The files not read and not in the vault, by when they were taken.
function scanUnread() {
  return [...scan.files.values()].filter(f => !f.read && !f.in_vault).sort(scanByTaken);
}

// The file shown large: the one chosen, else the first of `unread`.
function scanCurrent(unread = scanUnread()) {
  return unread.find(f => f.file === scan.current) ?? unread[0];
}

function scanByTaken(a, b) {
  return a.taken - b.taken || a.file.localeCompare(b.file);
}

function scanNewDoc(pages) {
  return { key: scan.nextDoc++, title: '', edited: false, pages, checked: true, error: null };
}

// Keep the documents in step with the files: pages that are gone, imported
// or no longer read leave their documents, and read pages that are in none
// are placed. What the user arranged stays.
function scanArrange() {
  for (const doc of scan.docs) doc.pages = doc.pages.filter(page => scanReady(scanFile(page)));
  scan.docs = scan.docs.filter(doc => doc.pages.length);
  const placed = new Set(scan.docs.flatMap(doc => doc.pages));
  const fresh = [...scan.files.values()].filter(f => scanReady(f) && !placed.has(f.file));
  for (const f of fresh.sort(scanByTaken)) scanPlace(f);
}

// Place the read page `f`: with the watch's Collect "together", into the
// document that has a page taken within two minutes of it, before its
// first page taken later; otherwise into a document of its own. Documents
// go by the time of their first page.
function scanPlace(f) {
  const together = (watches.get(scan.watch)?.collect ?? 'together') === 'together';
  const near = page => Math.abs(scanFile(page).taken - f.taken) <= SCAN_GROUP_MS;
  const doc = together && scan.docs.find(d => d.pages.some(near));
  if (doc) {
    const later = doc.pages.findIndex(page => scanFile(page).taken > f.taken);
    doc.pages.splice(later < 0 ? doc.pages.length : later, 0, f.file);
    return;
  }
  scan.docs.push(scanNewDoc([f.file]));
  scan.docs.sort((a, b) => scanFile(a.pages[0]).taken - scanFile(b.pages[0]).taken);
}

// The document's title: as edited, else its first page's, else the file's
// name.
function scanDocTitle(doc) {
  if (doc.edited) return doc.title;
  return scanFile(doc.pages[0])?.read?.title || scanStem(doc.pages[0]);
}

function scanStem(file) {
  return file.split(/[\\/]/).pop().replace(/\.[^.]*$/, '');
}

// ----------------------------------------------------------------------------
// previews

// The key of `f`'s preview: a changed file needs a new one.
function scanPreviewKey(f) {
  return `${f.file}|${f.size}|${f.modified}`;
}

// Ask for the previews the page lacks: the file shown large first, then
// the documents' pages, then the other files not in the vault.
function scanRequestPreviews() {
  const order = [scanCurrent()?.file, ...scan.docs.flatMap(doc => doc.pages), ...scan.files.keys()];
  const files = [...new Set(order)].filter(file => {
    const f = file && scanFile(file);
    return f && !f.in_vault && !scan.previews.has(scanPreviewKey(f));
  });
  if (files.length) rpcCall('scan_previews', { watch: scan.watch, files });
}

// The <img> of `f`'s preview, or null while there is none. It is made once
// and moved into every render, so it isn't decoded again.
function scanPreviewImg(f) {
  const preview = scan.previews.get(scanPreviewKey(f));
  if (!preview?.url) return null;
  if (!preview.img) {
    preview.img = accEl('img', 'scan-preview__img');
    preview.img.alt = '';
    preview.img.draggable = false;
    preview.img.src = preview.url;
  }
  return preview.img;
}

// A box (`tag`, `className`) with `f`'s preview: striped until it came, and
// a double-click opens the file in its default app.
function scanPreviewBox(tag, className, f, title) {
  const box = accEl(tag, className);
  const img = scanPreviewImg(f);
  if (img) box.append(img);
  const none = scan.previews.get(scanPreviewKey(f))?.url === '';
  box.title = `${title} \u00b7 ${none ? 'no preview' : 'double-click to open it'}`;
  box.addEventListener('dblclick', () => scanOpen(f.file));
  return box;
}

// ----------------------------------------------------------------------------
// actions: each answer is dropped if the page was opened for another watch
// meanwhile (`scan` is no longer the state that asked)

function scanList() {
  const s = scan;
  s.listing = true;
  rpcCall('scan_list', { watch: s.watch }, { channel: 'scan_list' }).then(listing => {
    if (scan === s) onScanFiles(listing);
  }, e => {
    if (e.code === 'superseded' || scan !== s) return;
    s.listing = false;
    s.listError = e.message;
    scheduleRenderScans();
  });
  scheduleRenderScans();
}

// The model the watch picked for reading its files, or what keeps it from
// reading (`aiModelOf`).
function scanSetup() {
  return aiModelOf(scanReadModel());
}

// The model the watch picked for reading its files, or null.
function scanReadModel() {
  return watchSettings(scan.watch).models?.[WATCH_READ_STEP] ?? null;
}

// Pick the model that reads the watch's files: kept in the watch.
function scanPickModel(choice) {
  const s = scan;
  watchSetModel(s.watch, WATCH_READ_STEP, choice, e => {
    if (scan !== s) return;
    s.error = e.message;
    scheduleRenderScans();
  });
  scheduleRenderScans();
}

// Read `files` with the OCR model; `again` reads files read before too.
function scanRead(files, again = false) {
  const ai = scanSetup();
  if (ai.problem || !files.length) return;
  const s = scan;
  s.error = null;
  const args = { watch: s.watch, files, again };
  rpcCall('scan_read', args).then(({ job }) => {
    if (scan === s) s.job = job;
  }, e => {
    if (scan !== s) return;
    s.error = e.message;
    scheduleRenderScans();
  });
}

// Stop the read: the file being read and those waiting report `stopped`.
// A job that ended meanwhile needs no stop.
function scanStop() {
  if (!scan.job) return;
  rpcCall('cancel_job', { id: scan.job }).catch(e => {
    if (e.code !== 'not_found') throw e;
  });
}

// Open `file` in its default app; a refusal shows on the page.
function scanOpen(file) {
  const s = scan;
  rpcCall('scan_open', { watch: s.watch, file }).catch(e => {
    if (scan !== s) return;
    s.error = e.message;
    scheduleRenderScans();
  });
}

// Import `docs`, leaving out those with a page being read.
function scanImport(docs) {
  const ready = docs.filter(doc => !doc.pages.some(page => scanBusy(scanFile(page))));
  if (!ready.length || scan.importing) return;
  for (const doc of ready) doc.error = null;
  const s = scan;
  s.error = null;
  s.importing = true;
  const args = { watch: s.watch, docs: ready.map(doc => ({ title: scanDocTitle(doc), files: doc.pages })) };
  rpcCall('scan_import', args).then(reply => {
    if (scan === s) onScanImported(reply);
  }, e => {
    if (scan === s) s.error = e.message;
  }).finally(() => {
    s.importing = false;
    scheduleRenderScans();
  });
  scheduleRenderScans();
}

// Move the page `file` into `doc`, before its page `before` (none: at its
// end); or, with no `doc`, into a document of its own after the one it
// left.
function scanMove(file, doc, before = null) {
  const from = scan.docs.find(d => d.pages.includes(file));
  if (!from || before === file) return;
  from.pages.splice(from.pages.indexOf(file), 1);
  if (doc) {
    const at = before ? doc.pages.indexOf(before) : -1;
    doc.pages.splice(at < 0 ? doc.pages.length : at, 0, file);
  } else {
    scan.docs.splice(scan.docs.indexOf(from) + 1, 0, scanNewDoc([file]));
  }
  scan.docs = scan.docs.filter(d => d.pages.length);
  scheduleRenderScans();
}

// One document per page: the first page keeps the document.
function scanSplit(doc) {
  const [first, ...rest] = doc.pages;
  doc.pages = [first];
  scan.docs.splice(scan.docs.indexOf(doc) + 1, 0, ...rest.map(page => scanNewDoc([page])));
  scheduleRenderScans();
}

// ----------------------------------------------------------------------------
// drag and drop: a page row is dragged onto a page (it goes before it), a
// document's head (it goes to its end), or the zone for a document of its
// own. The page isn't rendered again during a drag, which would end it.

function scanDragStart(e, file) {
  scan.dragging = file;
  e.dataTransfer.effectAllowed = 'move';
  e.dataTransfer.setData('text/plain', file);
  e.currentTarget.classList.add('is-dragged');
  e.currentTarget.closest('.import__docs')?.classList.add('is-dragging');
}

function scanDragEnd() {
  clearDropMark();
  if (!scan) return;
  scan.dragging = null;
  scheduleRenderScans();
}

// Make `el` a drop target: `onDrop(file)` gets the dragged page.
function scanDropTarget(el, onDrop) {
  dropTarget(el, {
    over: () => (scan?.dragging ? { el } : null),
    drop: () => {
      const file = scan.dragging;
      scan.dragging = null;
      onDrop(file);
    },
    effect: 'move',
  });
}

// ----------------------------------------------------------------------------
// rendering

const scheduleRenderScans = oncePerFrame(renderScans);

function renderScans() {
  if (!scan || scan.dragging || store.getState().vault_op !== 'scans') return;
  const watch = watches.get(scan.watch);
  if (!watch) return;
  const page = accEl('div', 'page');
  page.append(...renderScanHead(watch), renderScanFiles());
  if (scan.docs.length) page.append(renderScanDocs());
  replaceVaultPage(page);
}

function renderScanHead(watch) {
  const head = accEl('div', 'page-head');
  const patterns = watch.patterns.length ? watch.patterns.join(' ') : 'no file types';
  head.append(accEl('span', 'page-title', 'Import scans'),
    accEl('span', 'mono mono--small scans__where', `${watch.folder} \u00b7 ${patterns}`));
  const together = (watch.collect ?? 'together') === 'together';
  const blurb = together
    ? 'Images taken within two minutes of each other are the pages of one document.'
    : 'Each image is a document of its own; drag pages together to join them.';
  return [head, accEl('div', 'page-text',
    `${blurb} Only the files you read are sent for OCR. Nothing is written until you import.`)];
}

// The files not read yet: each with a Read button, or where its read stands.
function renderScanFiles() {
  const part = accEl('section', 'scans__part');
  const files = [...scan.files.values()];
  const unread = scanUnread();
  const inVault = files.filter(f => f.in_vault).length;

  const head = accEl('div', 'section-head', 'Files in folder');
  const note = scan.listed
    ? [`${unread.length} to read`, inVault && `${inVault} in the vault`,
      `listed ${TIME_FORMAT.format(scan.listed)}`]
      .filter(Boolean).join(' \u00b7 ')
    : 'listing\u2026';
  const again = accEl('button', 'tool');
  again.append(iconEl(SCAN_LIST_ICON));
  again.type = 'button';
  again.title = 'List the folder again';
  again.disabled = scan.listing;
  again.addEventListener('click', scanList);
  head.append(accEl('span', 'section-head__note', note), again);
  part.append(head);
  if (scan.listError) part.append(accEl('div', 'error-note', scan.listError));

  const ai = scanSetup();
  if (unread.length) {
    const list = accEl('div', 'scans__files');
    const columns = accEl('div', 'list-row scans__file scans__file--head label');
    columns.append(accEl('span', '', 'File'), accEl('span', '', 'Taken'), accEl('span', '', 'Size'),
      accEl('span'));
    list.append(columns);
    const current = scanCurrent(unread);
    for (const f of unread) list.append(...renderScanFile(f, ai, f === current));
    const box = accEl('div', 'scans__unread');
    box.append(list, renderScanFigure(current));
    part.append(box);
  } else if (scan.listed) {
    part.append(accEl('div', 'page-text scans__empty', files.length
      ? 'Every file here is read or in the vault.'
      : 'No file in this folder matches the watch.'));
  }

  const foot = accEl('div', 'scans__foot');
  const readable = unread.filter(f => !scanBusy(f));
  const readAll = accEl('button', 'btn btn--primary');
  readAll.type = 'button';
  readAll.append(iconEl(SCAN_READ_ICON),
    readable.length === 1 ? 'Read 1 file' : `Read all ${readable.length} files`);
  readAll.disabled = !readable.length || !!ai.problem;
  readAll.addEventListener('click', () => scanRead(readable.map(f => f.file)));
  foot.append(readAll);
  if (files.some(scanBusy)) {
    const stop = accEl('button', 'btn', 'Stop reading');
    stop.type = 'button';
    stop.addEventListener('click', scanStop);
    foot.append(stop);
  }
  const by = accEl('span', 'note');
  by.append('Read by ', aiModelPicker(scanReadModel(), scanPickModel));
  foot.append(by);
  if (ai.problem) foot.append(...aiMissingHint(ai, 'OCR'));
  part.append(foot);
  if (scan.error) part.append(accEl('div', 'error-note', scan.error));
  return part;
}

function renderScanFile(f, ai, current) {
  const row = accEl('div', 'list-row scans__file');
  if (current) row.setAttribute('aria-current', 'true');
  row.addEventListener('click', () => {
    scan.current = f.file;
    scheduleRenderScans();
  });
  const name = accEl('span', 'mono mono--small import__file', f.file);
  name.title = f.file;
  let status;
  if (scanBusy(f)) {
    status = accEl('span', 'meta scans__status',
      f.status === 'reading' ? `reading \u00b7 ${f.words} words` : 'queued');
  } else {
    status = accEl('button', 'icon-btn icon-btn--quiet scans__read');
    status.append(iconEl(SCAN_READ_ICON));
    status.type = 'button';
    status.title = f.status === 'failed' ? 'Read again' : 'Read';
    status.disabled = !!ai.problem;
    status.addEventListener('click', () => scanRead([f.file]));
  }
  row.append(name, accEl('span', 'meta', pageDate(f.taken, true)),
    accEl('span', 'meta', fileSize(f.size)), status);
  return f.error ? [row, accEl('div', 'error-note', f.error)] : [row];
}

// The file selected in "Files in folder", large.
function renderScanFigure(f) {
  const figure = accEl('figure', 'scans__preview');
  const name = accEl('span', 'mono mono--small import__file', f.file);
  name.title = f.file;
  figure.append(scanPreviewBox('span', 'scan-preview scan-preview--large', f, f.file), name,
    accEl('span', 'meta', `taken ${pageDate(f.taken, true)}`));
  return figure;
}

// The documents of the read pages, then Import.
function renderScanDocs() {
  const part = accEl('section', 'scans__part');
  const pages = scan.docs.reduce((n, doc) => n + doc.pages.length, 0);
  const head = accEl('div', 'section-head', 'Ready for import');
  head.append(accEl('span', 'section-head__note',
    `${plural(scan.docs.length, 'document')} \u00b7 ${plural(pages, 'page')}`));

  const select = accEl('div', 'select-bar');
  for (const [label, checked] of [['All', true], ['None', false]]) {
    const button = accEl('button', '', label);
    button.type = 'button';
    button.addEventListener('click', () => {
      for (const doc of scan.docs) doc.checked = checked;
      scheduleRenderScans();
    });
    select.append(button);
  }
  select.append(accEl('span', 'select-bar__note', 'drag pages to arrange them'));

  const list = accEl('div', 'import__docs');
  scan.docs.forEach((doc, index) => list.append(renderScanDoc(doc, index)));
  const zone = accEl('div', 'label scans__newdoc', 'Drop here to make it a document of its own');
  scanDropTarget(zone, file => scanMove(file, null));
  list.append(zone);

  const foot = accEl('div', 'scans__foot');
  const checked = scan.docs.filter(doc => doc.checked);
  const importAll = accEl('button', 'btn btn--primary');
  importAll.type = 'button';
  importAll.append(iconEl(IMPORT_ICON), checked.length === scan.docs.length
    ? `Import all ${plural(checked.length, 'document')}`
    : `Import ${plural(checked.length, 'document')}`);
  importAll.disabled = !checked.length || scan.importing;
  importAll.addEventListener('click', () => scanImport(checked));
  foot.append(importAll, accEl('span', 'meta', scan.importing ? 'importing\u2026' : 'Ctrl+Enter'));
  part.append(head, select, list, foot);
  return part;
}

function renderScanDoc(doc, index) {
  const row = accEl('div', 'list-row import__doc');
  const check = accEl('button', 'check', doc.checked ? '\u2713' : '');
  check.type = 'button';
  check.setAttribute('aria-pressed', String(doc.checked));
  check.title = doc.checked ? 'Leave it out of Import all' : 'Include it in Import all';
  check.addEventListener('click', () => {
    doc.checked = !doc.checked;
    scheduleRenderScans();
  });

  const head = accEl('div', 'import__doc-head');
  scanDropTarget(head, file => scanMove(file, doc));
  const titleBox = accEl('div', 'import__doc-title');
  const title = accEl('input', 'import__title');
  title.value = scanDocTitle(doc);
  title.title = 'Proposed title \u00b7 edit it before you import';
  title.spellcheck = false;
  title.dataset.focusKey = `scan-title-${doc.key}`;
  title.addEventListener('input', () => {
    doc.title = title.value;
    doc.edited = true;
  });
  const first = scanFile(doc.pages[0]);
  titleBox.append(title, accEl('span', 'meta',
    `${plural(doc.pages.length, 'page')} \u00b7 ${pageDate(first.taken)}`));

  const n = doc.pages.length;
  const busy = doc.pages.some(page => scanBusy(scanFile(page)));
  const acts = accEl('div', 'import__acts');
  acts.append(
    scanButton(SCAN_READ_ICON, 'Read', n === 1 ? 'Read the page again' : `Read the ${n} pages again`,
      busy || !!scanSetup().problem, () => scanRead(doc.pages, true)),
    scanButton(SCAN_SPLIT_ICON, 'Split',
      n > 1 ? `Split into ${n} documents, one per page` : 'One page, nothing to split',
      n < 2, () => scanSplit(doc)),
    scanButton(IMPORT_ICON, 'Import', `Import as \u201c${scanDocTitle(doc)}\u201d`,
      busy || scan.importing, () => scanImport([doc])));
  head.append(titleBox, acts);

  const main = accEl('div', 'import__doc-main');
  main.append(head);
  if (doc.error) main.append(accEl('div', 'error-note', doc.error));
  const pages = accEl('div', 'import__pages');
  doc.pages.forEach((file, i) => pages.append(renderScanPage(doc, index, file, i)));
  const body = accEl('div', 'import__doc-body');
  body.append(renderScanStrip(doc), pages);
  main.append(body);
  for (const f of doc.pages.map(scanFile).filter(f => f.error)) {
    main.append(accEl('div', 'error-note', `${f.file}: ${f.error}`));
  }
  row.append(check, main);
  return row;
}

// A page of the document `doc` (the `index`th): its number, file and words,
// and the buttons that move a first or last page to the document before or
// after. The row is dragged to move it anywhere.
function renderScanPage(doc, index, file, i) {
  const f = scanFile(file);
  const row = accEl('div', 'list-row import__page');
  if (file === scanDocCurrent(doc)) row.setAttribute('aria-current', 'true');
  row.addEventListener('click', () => {
    doc.current = file;
    scheduleRenderScans();
  });
  row.draggable = true;
  row.title = `${file} \u00b7 taken ${pageDate(f.taken, true)} \u00b7 drag it to move it`;
  row.addEventListener('dragstart', e => scanDragStart(e, file));
  row.addEventListener('dragend', scanDragEnd);
  scanDropTarget(row, dragged => scanMove(dragged, doc, file));

  let status = plural(f.read.words, 'word');
  if (f.status === 'reading') status = `reading \u00b7 ${f.words} words`;
  else if (f.status === 'queued') status = 'queued';
  else if (f.status === 'failed') status = `failed \u00b7 ${status}`;
  const name = accEl('span', 'mono mono--small import__file', file);

  const prev = scan.docs[index - 1];
  const next = scan.docs[index + 1];
  const first = i === 0;
  const last = i === doc.pages.length - 1;
  const before = scanPageButton(SCAN_BEFORE_ICON, !first
    ? 'Only the first page can join the document before'
    : prev ? 'Merge with the document before' : 'No document before this one',
  !(first && prev), () => scanMove(file, prev));
  const after = scanPageButton(SCAN_AFTER_ICON, !last
    ? 'Only the last page can join the document after'
    : next ? 'Merge with the document after' : 'No document after this one',
  !(last && next), () => scanMove(file, next, next.pages[0]));
  row.append(accEl('span', 'mono mono--small import__page-no', String(i + 1)), name,
    accEl('span', 'meta', status), before, after);
  return row;
}

// The document's current page: the one chosen, else its first.
function scanDocCurrent(doc) {
  return doc.pages.includes(doc.current) ? doc.current : doc.pages[0];
}

// The thumbnails of up to three pages around the current one, with arrows
// to the others; a click makes a page current, a double-click opens it.
function renderScanStrip(doc) {
  const strip = accEl('div', 'scan-strip');
  const current = scanDocCurrent(doc);
  const at = doc.pages.indexOf(current);
  const n = doc.pages.length;
  const start = Math.min(Math.max(0, at - 1), Math.max(0, n - SCAN_STRIP));
  const shown = doc.pages.slice(start, start + SCAN_STRIP);
  const pages = accEl('div', 'scan-strip__pages');
  shown.forEach((file, i) => {
    const no = start + i + 1;
    const thumb = scanPreviewBox('button', 'scan-preview', scanFile(file), `Page ${no} \u00b7 ${file}`);
    thumb.type = 'button';
    if (file === current) thumb.setAttribute('aria-current', 'true');
    thumb.append(accEl('span', 'scan-preview__no', String(no)));
    thumb.addEventListener('click', () => {
      doc.current = file;
      scheduleRenderScans();
    });
    pages.append(thumb);
  });
  strip.append(pages);
  if (n > SCAN_STRIP) {
    const step = (icon, title, to) => {
      const button = scanPageButton(icon, title, to < 0 || to >= n, () => {
        doc.current = doc.pages[to];
        scheduleRenderScans();
      });
      return button;
    };
    const nav = accEl('div', 'scan-strip__nav');
    nav.append(step(SCAN_BEFORE_ICON, 'Page before', at - 1),
      accEl('span', 'meta', `${start + 1}\u2013${start + shown.length} of ${n}`),
      step(SCAN_AFTER_ICON, 'Page after', at + 1));
    strip.append(nav);
  }
  return strip;
}

function scanButton(icon, label, title, disabled, onClick) {
  const button = accEl('button', 'btn');
  button.type = 'button';
  button.title = title;
  button.disabled = disabled;
  button.append(iconEl(icon), label);
  button.addEventListener('click', onClick);
  return button;
}

function scanPageButton(icon, title, disabled, onClick) {
  const button = accEl('button', 'icon-btn icon-btn--quiet');
  button.append(iconEl(icon));
  button.type = 'button';
  button.title = title;
  button.disabled = disabled;
  button.addEventListener('click', onClick);
  return button;
}

// Re-render while shown when the op, the watch (its folder, patterns,
// Collect) or the AI accounts (the OCR model) change. A removed watch takes
// its page along.
store.subscribe(
  [state => state.vault_op, state => state.vault_expanded, state => state.watches_rev, state => state.accounts_rev],
  ([vault_op]) => {
    if (scan && !watches.has(scan.watch)) {
      scan = null;
      if (vault_op === 'scans') closeVaultArea();
      return;
    }
    scheduleRenderScans();
  });
