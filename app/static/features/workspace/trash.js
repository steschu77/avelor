// Workspace -> Trash: the files a delete moved to the vault's `.trash`
// (vault.md#trash), rendered into the expanded vault area while its
// op is "trash". Each can be restored -- it comes back where it was, and is
// selected -- or deleted for good; Empty trash deletes them all. The list
// is asked for (`list_trash`) when the page opens, after each of its own
// commands, and when an entity leaves the vault while it is shown. A
// failure shows in the host's words.

import { accEl, oncePerFrame } from '../../core/dom.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import { DATE_FORMAT, parseDate, plural } from '../../core/format.js';
import { showError, store } from '../../model/state.js';
import { getDocIcon, iconEl } from '../../ui/icons.js';
import { armedButton } from '../../ui/confirm.js';
import { replaceVaultPage } from '../../ui/page.js';
import { selectById } from '../selection.js';

let trashItems = null; // the last `list_trash` answer, null before it

const scheduleLoadTrash = oncePerFrame(loadTrash);

function loadTrash() {
  if (store.getState().vault_op !== 'trash') return;
  rpcCall('list_trash', {}, { channel: 'trash' }).then(({ items }) => {
    trashItems = items;
    renderTrash();
  }, e => {
    if (e.code !== 'superseded') showError(`Could not read the trash: ${e.message}`);
  });
}

function renderTrash() {
  if (store.getState().vault_op !== 'trash') return;
  const page = accEl('div', 'page trash');
  const items = trashItems ?? [];

  const head = accEl('div', 'page-head');
  head.append(accEl('div', 'page-title', 'Trash'));
  if (trashItems) head.append(accEl('span', 'meta', plural(items.length, 'file')));
  if (items.length) {
    const empty = accEl('button', 'btn push', 'Empty trash');
    empty.type = 'button';
    empty.title = 'Delete every file in the trash for good';
    armedButton(empty, 'Click again to empty the trash', emptyTrash);
    head.append(empty);
  }
  page.append(head);

  if (trashItems && !items.length) {
    page.append(accEl('div', 'page-text', 'The trash is empty.'));
  } else if (items.length) {
    const list = accEl('div', 'trash__list');
    list.append(...items.map(trashRow));
    page.append(list);
  }
  replaceVaultPage(page);
}

// One file: its icon, title, folder and date (or what is wrong with it),
// then Restore -- not for a file that couldn't be read -- and Delete.
function trashRow(item) {
  const row = accEl('div', 'list-row trash__row');
  const name = item.title || item.path.split(/[\\/]/).pop();
  const icon = iconEl(item.kind ? getDocIcon(item.kind, item.genre) : '', 'table__icon');
  const title = accEl('span', 'row-title', name);
  title.title = name;
  const folder = accEl('span', 'mono truncate-start', item.path);
  folder.title = item.path;
  const date = parseDate(item.date);
  const when = accEl('span', 'meta truncate-start', item.error ?? (date ? DATE_FORMAT.format(date) : '\u2014'));
  if (item.error) when.title = item.error;

  const restore = accEl('button', 'tool');
  restore.type = 'button';
  restore.append(iconEl('#i-arrow-undo'));
  restore.title = item.kind ? 'Restore where it was' : 'Can\'t be read, so it can\'t be restored';
  restore.setAttribute('aria-disabled', String(!item.kind));
  restore.addEventListener('click', () => {
    if (restore.getAttribute('aria-disabled') !== 'true') restoreTrashed(item);
  });

  const remove = accEl('button', 'tool tool--danger');
  remove.type = 'button';
  remove.append(iconEl('#i-delete'));
  remove.title = 'Delete for good';
  armedButton(remove, 'Click again to delete for good', () => deleteTrashed(item));

  row.append(icon, title, folder, when, restore, remove);
  return row;
}

// Put `item` back into the vault and select it; the host pushes the entity
// before it answers.
function restoreTrashed(item) {
  rpcCall('restore_entity', { path: item.path }).then(({ id }) => {
    selectById(id);
    loadTrash();
  }, e => {
    showError(e.message);
    loadTrash();
  });
}

function deleteTrashed(item) {
  rpcCall('delete_trashed', { path: item.path }).then(loadTrash, e => {
    showError(e.message);
    loadTrash();
  });
}

function emptyTrash() {
  rpcCall('empty_trash').then(loadTrash, e => {
    showError(e.message);
    loadTrash();
  });
}

// Opening the page lists the trash again; what was listed before shows
// meanwhile, unless it was another vault's.
let trashVaultPath = null;
store.subscribe([state => state.vault_op, state => state.vault_expanded, state => state.vault_path],
  ([vault_op, vault_expanded, vault_path]) => {
    if (vault_path !== trashVaultPath) {
      trashVaultPath = vault_path;
      trashItems = null;
    }
    if (vault_op !== 'trash' || !vault_expanded) return;
    renderTrash();
    loadTrash();
  });

// A delete elsewhere (or a dissolved thread) adds to the trash.
rpcOn('entity_deleted', scheduleLoadTrash);
