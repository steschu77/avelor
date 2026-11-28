// Import scans (import.md#import-scans;
// api.md#scans): the import of a folder watch, images or PDFs (one
// document each, read from their text or their scans), opened by the Import
// button of its source (features/sidebar.js) or of its pane (features/workspace/folders.js,
// through the `watch.import` action), and rendered
// into the expanded vault area while its op is "scans". The host lists the
// watch's files, reads the ones asked for with the model the watch picked
// for OCR (its `models`' `read`, picked here or in its pane) as a job, which Stop ends
// (`cancel_job`) and the account's spend limit stops, and writes the
// documents into `Scans/`. The page groups an images watch's read pages
// into documents, which the user arranges (drag and drop, merge, split),
// and titles them before importing them. The files already in the vault
// are hidden unless asked for; read again, they replace their document's
// text, after a second click. While the watch's analysis has no model that
// works, nothing is read or imported, as Fetch email doesn't fetch. The images are shown as previews: the host
// renders them (`scan_previews`), and a double-click opens one in its
// default app (`scan_open`). On the page, Esc closes it and Ctrl+Enter
// imports the ticked documents (`registerVaultPage`).

import { accEl, oncePerFrame } from '../../core/dom.js';
import { pressable } from '../../core/dom.js';
import { titled } from '../../core/dom.js';
import { rpcCall } from '../../core/rpc.js';
import { rpcOn } from '../../core/rpc.js';
import { TIME_FORMAT, compareText, pageDate, parseDate } from '../../core/format.js';
import { fileSize } from '../../core/format.js';
import { registerAction, shortcutText } from '../../core/actions.js';
import { parts, t, textOf } from '../../core/text.js';
import { store } from '../../model/state.js';
import { entities, watches } from '../../model/entities.js';
import { armedButton } from '../../ui/confirm.js';
import { iconEl } from '../../ui/icons.js';
import { penButton } from '../../ui/field.js';
import { IMPORT_ICON } from '../../ui/icons.js';
import { clearDropMark, dropTarget } from '../../ui/dropzone.js';
import {
  closeVaultArea, openVaultOp, registerVaultPage, replaceVaultPage,
} from '../../ui/page.js';
import {
  aiMissingHint, aiModelOf, aiModelPicker,
} from '../workspace/models.js';
import {
  WATCH_READ_STEP, watchSetModel, watchSettings, watchType,
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
function openScanImport(watch_id) {
  if (scan?.watch !== watch_id) {
    scan = {
      watch: watch_id,
      listing: false, // while `scan_list` is being answered
      listed: null, // when the files were listed
      listError: null,
      // file -> { file, size, taken, in_vault, read: { words, title, encrypted } | null,
      // status: 'queued' | 'reading' | 'failed' | null, words, error, again }:
      // `again` while a file in the vault is read again, to replace its text
      files: new Map(),
      showInVault: false, // the files in the vault listed too
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
  openVaultOp('scans', scanType()?.page.title ?? t('import.scans_title'));
  scanList();
}

registerVaultPage('scans', {
  close: closeVaultArea,
  submit: () => {
    if (scan) scanImport(scan.docs.filter(doc => doc.checked));
  },
});
registerAction({ id: 'watch.import', title: t('scans.import_from_folder'), run: openScanImport });

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
      read: before?.again ? before.read : f.read,
      again: !!before?.again && !!f.in_vault,
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
    const read = { words: page.words, title: page.title, encrypted: !!page.encrypted };
    Object.assign(f, { read, status: null, error: null });
    scanArrange();
  } else if (page.status === 'failed') {
    Object.assign(f, { status: 'failed', error: textOf(page.error) });
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
      if (f) Object.assign(f, { in_vault: id, again: false });
    }
  }
  for (const { files, error } of reply.failed) {
    const doc = scan.docs.find(d =>
      d.pages.length === files.length && d.pages.every((page, i) => page === files[i]));
    if (doc) doc.error = textOf(error);
  }
  scanArrange();
  scheduleRenderScans();
}

// ----------------------------------------------------------------------------
// files and documents

// The type of the watch imported from (`watchType`).
function scanType() {
  return watchType(watches.get(scan.watch)?.type);
}

function scanFile(file) {
  return scan.files.get(file);
}

// A page that belongs in a document: read, and not in the vault yet, or
// read again to replace its document's text.
function scanReady(f) {
  return !!f?.read && (!f.in_vault || !!f.again);
}

// Being read, or waiting for it.
function scanBusy(f) {
  return f?.status === 'queued' || f?.status === 'reading';
}

// The files to read, by when they were taken: those not read and not in the
// vault, and with `showInVault` those in the vault not read again.
function scanUnread() {
  const toRead = f => (f.in_vault ? scan.showInVault && !(f.again && f.read) : !f.read);
  return [...scan.files.values()].filter(toRead).sort(scanByTaken);
}

// The document in the vault `doc` replaces: its pages', read again.
function scanDocReplaces(doc) {
  return scanFile(doc.pages[0])?.in_vault ?? null;
}

// Whether the watch's analysis is on without a model that works: then
// nothing is read or imported (the host refuses both).
function scanAnalysisBlocked() {
  const analysis = watchSettings(scan.watch).analysis;
  return !!analysis && analysis.level !== 'off' && !!aiModelOf(analysis.model).problem;
}

// Whether reading waits: for a model that works, where the watch needs one
// to read, or for the analysis's.
function scanReadBlocked() {
  return (!!scanSetup().problem && !scanType()?.read.optional) || scanAnalysisBlocked();
}

// The file shown large: the one chosen, else the first of `unread`.
function scanCurrent(unread = scanUnread()) {
  return unread.find(f => f.file === scan.current) ?? unread[0];
}

function scanByTaken(a, b) {
  return a.taken - b.taken || compareText(a.file, b.file);
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

// Place the read page `f`: with an images watch's Collect "together", into
// the document that has a page taken within two minutes of it, before its
// first page taken later; otherwise into a document of its own. Documents
// go by the time of their first page.
function scanPlace(f) {
  const together = !!scanType()?.read.collect
    && (watches.get(scan.watch)?.collect ?? 'together') === 'together';
  // A page read again joins only the pages of its own document.
  const near = page => scanFile(page).in_vault === f.in_vault
    && Math.abs(scanFile(page).taken - f.taken) <= SCAN_GROUP_MS;
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
  const replaces = scanDocReplaces(doc);
  if (replaces) return entities.get(replaces)?.title ?? scanStem(doc.pages[0]);
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
// the documents' pages, then the other files not in the vault. A PDF has
// none yet.
function scanRequestPreviews() {
  if (!scanType()?.read.previews) return;
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
  box.title = `${title} \u00b7 ${none ? t('scans.no_preview') : t('scans.double_click')}`;
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

// Read `files` with the OCR model; `again` reads files read before too, as
// do files in the vault, read again to replace their documents' text.
function scanRead(files, again = false) {
  if (scanReadBlocked() || !files.length) return;
  const s = scan;
  s.error = null;
  for (const f of files.map(scanFile).filter(f => f?.in_vault)) {
    Object.assign(f, { again: true, read: null });
    again = true;
  }
  scanArrange();
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
  if (!ready.length || scan.importing || scanAnalysisBlocked()) return;
  for (const doc of ready) doc.error = null;
  const s = scan;
  s.error = null;
  s.importing = true;
  // A title only as edited: the host proposes the one shown, and the
  // analysis keeps a title given.
  const args = {
    watch: s.watch,
    docs: ready.map(doc => ({
      title: doc.edited ? doc.title : '', files: doc.pages, replace: scanDocReplaces(doc) ?? undefined,
    })),
  };
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
  const type = scanType();
  const head = accEl('div', 'page-head');
  const patterns = watch.patterns.length ? watch.patterns.join(' ') : t('watch.no_file_types');
  head.append(accEl('span', 'page-title', type.page.title),
    accEl('span', 'mono mono--small scans__where', `${watch.folder} \u00b7 ${patterns}`));
  const together = (watch.collect ?? 'together') === 'together';
  const blurb = type.page.blurb ?? (together ? t('scans.blurb_together') : t('scans.blurb_single'));
  return [head, accEl('div', 'page-text', `${blurb} ${t('scans.nothing_written')}`)];
}

// The files not read yet: each with a Read button, or where its read stands.
function renderScanFiles() {
  const part = accEl('section', 'scans__part');
  const files = [...scan.files.values()];
  const unread = scanUnread();
  const inVault = files.filter(f => f.in_vault).length;
  const fresh = unread.filter(f => !f.in_vault).length;

  const head = accEl('div', 'section-head', t('scans.files_in_folder'));
  const note = scan.listed
    ? [t('scans.to_read', { n: fresh }), inVault && t('email.in_vault_n', { n: inVault }),
      t('scans.listed_at', { time: TIME_FORMAT.format(scan.listed) })]
      .filter(Boolean).join(' \u00b7 ')
    : t('scans.listing');
  const again = accEl('button', 'tool');
  again.append(iconEl(SCAN_LIST_ICON));
  again.type = 'button';
  again.title = t('scans.list_again');
  again.disabled = scan.listing;
  again.addEventListener('click', scanList);
  head.append(accEl('span', 'section-head__note', note));
  if (inVault) head.append(renderScanShowInVault(inVault));
  head.append(again);
  part.append(head);
  if (scan.listError) part.append(accEl('div', 'error-note', scan.listError));

  const ai = scanSetup();
  const blocked = scanReadBlocked();
  if (unread.length) {
    const list = accEl('div', 'scans__files');
    const columns = accEl('div', 'list-row scans__file scans__file--head label');
    columns.append(accEl('span', '', t('props.file')), accEl('span', '', scanType().page.date), accEl('span', '', t('scans.size')),
      accEl('span'));
    list.append(columns);
    const current = scanCurrent(unread);
    for (const f of unread) list.append(...renderScanFile(f, blocked, f === current));
    const box = accEl('div', 'scans__unread');
    box.append(list, renderScanFigure(current));
    part.append(box);
  } else if (scan.listed) {
    part.append(accEl('div', 'page-text scans__empty', files.length
      ? t('scans.all_read')
      : t('scans.none_match')));
  }

  const foot = accEl('div', 'scans__foot');
  const readable = unread.filter(f => !scanBusy(f));
  const readAll = accEl('button', 'btn btn--primary');
  readAll.type = 'button';
  readAll.append(iconEl(SCAN_READ_ICON),
    readable.length === 1 ? t('scans.read_one') : t('scans.read_all', { n: readable.length }));
  readAll.disabled = !readable.length || blocked;
  readAll.addEventListener('click', () => scanRead(readable.map(f => f.file)));
  foot.append(readAll);
  if (files.some(scanBusy)) {
    const stop = accEl('button', 'btn', t('scans.stop_reading'));
    stop.type = 'button';
    stop.addEventListener('click', scanStop);
    foot.append(stop);
  }
  const by = accEl('span', 'note');
  by.append(...parts('scans.read_by', { picker: aiModelPicker(scanReadModel(), scanPickModel) }));
  foot.append(by);
  if (ai.problem) foot.append(...aiMissingHint(ai, scanType()?.read.what ?? t('feature.ocr')));
  if (scanAnalysisBlocked()) foot.append(renderScanAnalysisNote());
  part.append(foot);
  if (scan.error) part.append(accEl('div', 'error-note', scan.error));
  return part;
}

// The check that lists the `n` files in the vault too, to read them again.
function renderScanShowInVault(n) {
  const show = accEl('label', 'scans__show');
  const check = accEl('button', 'check', scan.showInVault ? '\u2713' : '');
  check.type = 'button';
  check.setAttribute('aria-pressed', String(scan.showInVault));
  titled(check, scan.showInVault ? t('scans.hide_in_vault') : t('scans.list_in_vault'));
  check.addEventListener('click', () => {
    scan.showInVault = !scan.showInVault;
    scheduleRenderScans();
  });
  show.append(check, accEl('span', 'label', t('scans.show_in_vault', { n })));
  return show;
}

// Why nothing is read or imported while the analysis waits for a model.
function renderScanAnalysisNote() {
  return accEl('div', 'note scans__warning', t('scans.analysis_blocked'));
}

function renderScanFile(f, blocked, current) {
  const row = pressable(accEl('div', 'list-row scans__file'));
  row.dataset.focusKey = `scan-file:${f.file}`;
  if (f.in_vault) row.classList.add('scans__file--in-vault');
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
      f.status === 'reading' ? t('scans.reading_words', { n: f.words }) : t('scans.queued'));
  } else {
    status = accEl('button', 'icon-btn icon-btn--quiet scans__read');
    status.append(iconEl(SCAN_READ_ICON));
    status.type = 'button';
    status.title = f.in_vault
      ? t('scans.read_again_in_vault')
      : f.status === 'failed' ? t('scans.read_again') : t('watch.label.read');
    status.disabled = blocked;
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
    accEl('span', 'meta', scanType().page.dated(pageDate(f.taken, true))));
  return figure;
}

// The documents of the read pages, then Import.
function renderScanDocs() {
  const part = accEl('section', 'scans__part');
  const pages = scan.docs.reduce((n, doc) => n + doc.pages.length, 0);
  const head = accEl('div', 'section-head', t('scans.ready'));
  head.append(accEl('span', 'section-head__note',
    `${t('types.count_documents', { n: scan.docs.length })} \u00b7 ${t('scans.pages', { n: pages })}`));

  const arranges = !!scanType()?.read.collect;
  const select = accEl('div', 'select-bar');
  for (const [label, checked] of [[t('email.select.all'), true], [t('email.select.none'), false]]) {
    const button = accEl('button', '', label);
    button.type = 'button';
    button.addEventListener('click', () => {
      for (const doc of scan.docs) doc.checked = checked;
      scheduleRenderScans();
    });
    select.append(button);
  }
  if (arranges) select.append(accEl('span', 'select-bar__note', t('scans.drag_to_arrange')));

  const list = accEl('div', 'import__docs');
  scan.docs.forEach((doc, index) => list.append(renderScanDoc(doc, index, arranges)));
  if (arranges) {
    const zone = accEl('div', 'label scans__newdoc', t('scans.drop_new_doc'));
    scanDropTarget(zone, file => scanMove(file, null));
    list.append(zone);
  }

  const foot = accEl('div', 'scans__foot');
  const checked = scan.docs.filter(doc => doc.checked);
  const replacing = checked.filter(scanDocReplaces);
  const importAll = accEl('button', 'btn btn--primary');
  importAll.type = 'button';
  importAll.append(iconEl(IMPORT_ICON), checked.length === scan.docs.length
    ? t('scans.import_all', { n: checked.length })
    : t('scans.import_n', { n: checked.length }));
  importAll.disabled = !checked.length || scan.importing || scanAnalysisBlocked();
  // Replacing texts takes a second click; another set to replace asks again.
  if (replacing.length) {
    importAll.classList.add('btn--danger');
    armedButton(importAll, t('scans.replace_again', { n: replacing.length }),
      () => scanImport(checked), `scan_import ${replacing.map(doc => doc.key).join(' ')}`);
  } else {
    importAll.addEventListener('click', () => scanImport(checked));
  }
  foot.append(importAll, accEl('span', 'meta', scan.importing ? t('scans.importing') : shortcutText('Ctrl+Enter')));
  if (replacing.length) {
    foot.append(accEl('div', 'note scans__warning', t('scans.replacing', { n: replacing.length })));
  }
  if (scanAnalysisBlocked()) foot.append(renderScanAnalysisNote());
  part.append(head, select, list, foot);
  return part;
}

// A document to import (the `index`th); one of a watch whose pages are
// arranged (`arranges`) has Split, and its pages move.
function renderScanDoc(doc, index, arranges) {
  const row = accEl('div', 'list-row import__doc');
  const replaces = scanDocReplaces(doc);
  if (replaces) row.classList.add('import__doc--replace');
  const check = accEl('button', 'check', doc.checked ? '\u2713' : '');
  check.type = 'button';
  check.setAttribute('aria-pressed', String(doc.checked));
  titled(check, doc.checked ? t('scans.leave_out') : t('scans.include'));
  check.addEventListener('click', () => {
    doc.checked = !doc.checked;
    scheduleRenderScans();
  });

  const head = accEl('div', 'import__doc-head');
  if (arranges) scanDropTarget(head, file => scanMove(file, doc));
  const titleBox = accEl('div', 'import__doc-title');
  const titleLine = accEl('div', 'import__title-line');
  const title = accEl('input', 'import__title');
  title.value = scanDocTitle(doc);
  title.title = t('scans.proposed_title');
  title.spellcheck = false;
  title.dataset.focusKey = `scan-title-${doc.key}`;
  title.addEventListener('input', () => {
    doc.title = title.value;
    doc.edited = true;
  });
  const first = scanFile(doc.pages[0]);
  const size = !arranges ? scanWords(first) : t('scans.pages', { n: doc.pages.length });
  const replacedTitle = replaces && entities.get(replaces)?.title;
  const replaced = !replaces ? '' : ` \u00b7 ${replacedTitle ? t('scans.replaces', { title: replacedTitle }) : t('scans.replaces_its')}`;
  // its pen right after it (ui.md#the-pen), as a click into it does
  titleLine.append(title, penButton(t('scans.proposed_title'), () => {
    title.focus();
    title.select();
  }));
  titleBox.append(titleLine, accEl('span', 'meta', `${size} \u00b7 ${pageDate(first.taken)}${replaced}`));

  const n = doc.pages.length;
  const busy = doc.pages.some(page => scanBusy(scanFile(page)));
  const blocked = scanReadBlocked();
  const acts = accEl('div', 'import__acts');
  acts.append(scanButton(SCAN_READ_ICON, t('watch.label.read'),
    t('scans.read_pages_again', { n }),
    busy || blocked, () => scanRead(doc.pages, true)));
  if (arranges) {
    acts.append(scanButton(SCAN_SPLIT_ICON, t('scans.split'),
      n > 1 ? t('scans.split_into', { n }) : t('scans.nothing_to_split'),
      n < 2, () => scanSplit(doc)));
  }
  const importOne = scanButton(IMPORT_ICON, t('watch.import'), t('scans.import_as', { title: scanDocTitle(doc) }),
    busy || scan.importing || scanAnalysisBlocked(), replaces ? null : () => scanImport([doc]));
  if (replaces) {
    importOne.classList.add('btn--danger');
    armedButton(importOne, t('scans.replace_one_again'), () => scanImport([doc]),
      `scan_import ${doc.key}`);
  }
  acts.append(importOne);
  head.append(titleBox, acts);

  const main = accEl('div', 'import__doc-main');
  main.append(head);
  if (doc.error) main.append(accEl('div', 'error-note', doc.error));
  const pages = accEl('div', 'import__pages');
  doc.pages.forEach((file, i) => pages.append(renderScanPage(doc, index, file, i, arranges)));
  const body = accEl('div', 'import__doc-body');
  body.append(renderScanStrip(doc), pages);
  main.append(body);
  for (const f of doc.pages.map(scanFile).filter(f => f.error)) {
    main.append(accEl('div', 'error-note', `${f.file}: ${f.error}`));
  }
  row.append(check, main);
  return row;
}

// A page of the document `doc` (the `index`th): its number, file and words;
// where pages are arranged (`arranges`), the buttons that move a first or
// last page to the document before or after, and the row is dragged to move
// it anywhere.
function renderScanPage(doc, index, file, i, arranges) {
  const f = scanFile(file);
  const row = pressable(accEl('div', 'list-row import__page'));
  row.dataset.focusKey = `scan-page:${file}`;
  if (file === scanDocCurrent(doc)) row.setAttribute('aria-current', 'true');
  row.addEventListener('click', () => {
    doc.current = file;
    scheduleRenderScans();
  });
  row.title = `${file} \u00b7 ${scanType().page.dated(pageDate(f.taken, true))}`;
  if (arranges) {
    row.draggable = true;
    row.title += ` \u00b7 ${t('scans.drag_to_move')}`;
    row.addEventListener('dragstart', e => scanDragStart(e, file));
    row.addEventListener('dragend', scanDragEnd);
    scanDropTarget(row, dragged => scanMove(dragged, doc, file));
  }

  let status = scanWords(f);
  if (f.status === 'reading') status = t('scans.reading_words', { n: f.words });
  else if (f.status === 'queued') status = t('scans.queued');
  else if (f.status === 'failed') status = t('scans.failed_with', { status });
  const name = accEl('span', 'mono mono--small import__file', file);
  row.append(accEl('span', 'mono mono--small import__page-no', String(i + 1)), name,
    accEl('span', 'meta', status));
  if (!arranges) return row;

  const prev = scan.docs[index - 1];
  const next = scan.docs[index + 1];
  const first = i === 0;
  const last = i === doc.pages.length - 1;
  const before = scanPageButton(SCAN_BEFORE_ICON, !first
    ? t('scans.only_first_joins')
    : prev ? t('scans.merge_before') : t('scans.no_doc_before'),
  !(first && prev), () => scanMove(file, prev));
  const after = scanPageButton(SCAN_AFTER_ICON, !last
    ? t('scans.only_last_joins')
    : next ? t('scans.merge_after') : t('scans.no_doc_after'),
  !(last && next), () => scanMove(file, next, next.pages[0]));
  row.append(before, after);
  return row;
}

// What the read file `f` read to: its words, or that it is encrypted.
function scanWords(f) {
  return f.read.encrypted ? t('scans.encrypted') : t('scans.words', { n: f.read.words });
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
    const thumb = scanPreviewBox('button', 'scan-preview', scanFile(file), t('scans.page_file', { no, file }));
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
    nav.append(step(SCAN_BEFORE_ICON, t('scans.page_before'), at - 1),
      accEl('span', 'meta', t('email.page_of', { from: start + 1, to: start + shown.length, n })),
      step(SCAN_AFTER_ICON, t('scans.page_after'), at + 1));
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
  // Without `onClick`, the caller arms it (`armedButton`).
  if (onClick) button.addEventListener('click', onClick);
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
