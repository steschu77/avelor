// Import scans (import.md#import-scans;
// api.md#scans): the import of an images folder watch, opened by the Import
// button of its pane (features/workspace/folders.js,
// through the `watch.import` action), and rendered
// into the expanded vault area while its op is "scans". The host lists the
// watch's files. On the page, Esc closes it (`registerVaultPage`).

import { accEl, oncePerFrame } from '../../core/dom.js';
import { rpcCall } from '../../core/rpc.js';
import { TIME_FORMAT, pageDate, parseDate } from '../../core/format.js';
import { registerAction } from '../../core/actions.js';
import { store } from '../../model/state.js';
import { watches } from '../../model/entities.js';
import { iconEl } from '../../ui/icons.js';
import {
  closeVaultArea, openVaultOp, registerVaultPage, replaceVaultPage,
} from '../../ui/page.js';

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
      // file -> { file, size, taken, in_vault }
      files: new Map(),
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
// host answers

// The files as listed (`scan_list`'s answer) replace the ones shown.
function onScanFiles(listing) {
  scan.listing = false;
  scan.listError = null;
  scan.listed = parseDate(listing.listed);
  const files = new Map();
  for (const f of listing.files) {
    files.set(f.file, {
      file: f.file,
      size: f.size,
      taken: parseDate(f.taken),
      in_vault: f.in_vault,
    });
  }
  scan.files = files;
  scheduleRenderScans();
}

// ----------------------------------------------------------------------------
// files

// The files not in the vault, by when they were taken.
function scanUnread() {
  return [...scan.files.values()].filter(f => !f.in_vault).sort(scanByTaken);
}

function scanByTaken(a, b) {
  return a.taken - b.taken || a.file.localeCompare(b.file);
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
  return [head];
}

// The files not read yet.
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

  if (unread.length) {
    const list = accEl('div', 'scans__files');
    const columns = accEl('div', 'list-row scans__file scans__file--head label');
    columns.append(accEl('span', '', 'File'), accEl('span', '', 'Taken'), accEl('span', '', 'Size'));
    list.append(columns);
    for (const f of unread) list.append(renderScanFile(f));
    part.append(list);
  } else if (scan.listed) {
    part.append(accEl('div', 'page-text scans__empty', files.length
      ? 'Every file here is in the vault.'
      : 'No file in this folder matches the watch.'));
  }
  return part;
}

function renderScanFile(f) {
  const row = accEl('div', 'list-row scans__file');
  const name = accEl('span', 'mono mono--small import__file', f.file);
  name.title = f.file;
  row.append(name, accEl('span', 'meta', pageDate(f.taken, true)),
    accEl('span', 'meta', scanSize(f.size)));
  return row;
}

// "812 KB", "2.4 MB".
function scanSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

// Re-render while shown when the op or the watch (its folder, patterns)
// change. A removed watch takes its page along.
store.subscribe(
  [state => state.vault_op, state => state.vault_expanded, state => state.watches_rev],
  ([vault_op]) => {
    if (scan && !watches.has(scan.watch)) {
      scan = null;
      if (vault_op === 'scans') closeVaultArea();
      return;
    }
    scheduleRenderScans();
  });
