// Import scans (import.md#import-scans;
// api.md#scans): the import of an images folder watch, opened by the Import
// button of its pane (features/workspace/folders.js,
// through the `watch.import` action), and rendered
// into the expanded vault area while its op is "scans". The host lists the
// watch's files, reads the ones asked for with the model the watch picked
// for OCR (its `models`' `read`, picked here or in its pane) as a job, which Stop ends
// (`cancel_job`) and the account's spend limit stops. The files are shown
// as previews: the host renders them (`scan_previews`), and a double-click
// opens one in its default app (`scan_open`). On the page, Esc closes it
// (`registerVaultPage`).

import { accEl, oncePerFrame } from '../../core/dom.js';
import { rpcCall } from '../../core/rpc.js';
import { rpcOn } from '../../core/rpc.js';
import { TIME_FORMAT, pageDate, parseDate } from '../../core/format.js';
import { registerAction } from '../../core/actions.js';
import { store } from '../../model/state.js';
import { watches } from '../../model/entities.js';
import { iconEl } from '../../ui/icons.js';
import {
  closeVaultArea, openVaultOp, registerVaultPage, replaceVaultPage,
} from '../../ui/page.js';
import {
  aiMissingHint, aiModelOf, aiModelPicker,
} from '../workspace/models.js';
import {
  WATCH_READ_STEP, watchSetModel, watchSettings,
} from '../workspace/folders.js';

const SCAN_READ_ICON = '#i-book-open';
const SCAN_LIST_ICON = '#i-arrow-sync';

let scan = null; // the page's state, see openScanImport

// ----------------------------------------------------------------------------
// opening and closing

// Open the page for `watch_id` and list its files.
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
      job: null, // the job reading this watch's files, which Stop ends
      error: null, // a refused read or open
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
  } else if (page.status === 'failed') {
    Object.assign(f, { status: 'failed', error: page.error });
  } else if (page.status === 'stopped') {
    f.status = null;
  } else {
    Object.assign(f, { status: page.status, words: page.words ?? 0, error: null });
  }
  scheduleRenderScans();
});

// ----------------------------------------------------------------------------
// files

function scanFile(file) {
  return scan.files.get(file);
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

// ----------------------------------------------------------------------------
// previews

// The key of `f`'s preview: a changed file needs a new one.
function scanPreviewKey(f) {
  return `${f.file}|${f.size}|${f.modified}`;
}

// Ask for the previews the page lacks: the file shown large first, then
// the other files not in the vault.
function scanRequestPreviews() {
  const order = [scanCurrent()?.file, ...scan.files.keys()];
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

// Read `files` with the OCR model.
function scanRead(files, again = false) {
  const ai = scanSetup();
  if (ai.problem || !files.length) return;
  const s = scan;
  s.error = null;
  const args = { watch: s.watch, files };
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

// ----------------------------------------------------------------------------
// rendering

const scheduleRenderScans = oncePerFrame(renderScans);

function renderScans() {
  if (!scan || store.getState().vault_op !== 'scans') return;
  const watch = watches.get(scan.watch);
  if (!watch) return;
  const page = accEl('div', 'page');
  page.append(...renderScanHead(watch), renderScanFiles());
  replaceVaultPage(page);
}

function renderScanHead(watch) {
  const head = accEl('div', 'page-head');
  const patterns = watch.patterns.length ? watch.patterns.join(' ') : 'no file types';
  head.append(accEl('span', 'page-title', 'Import scans'),
    accEl('span', 'mono mono--small scans__where', `${watch.folder} \u00b7 ${patterns}`));
  return [head, accEl('div', 'page-text', 'Only the files you read are sent for OCR.')];
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
    accEl('span', 'meta', scanSize(f.size)), status);
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

// "812 KB", "2.4 MB".
function scanSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

// Re-render while shown when the op, the watch (its folder, patterns) or
// the AI accounts (the OCR model) change. A removed watch takes its page
// along.
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
