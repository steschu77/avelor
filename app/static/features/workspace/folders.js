// Workspace -> Folders (import.md#the-folders-page;
// api.md#watches): the open vault's folder watches, each reading one folder on this
// PC. The page follows Workspace -> Accounts: a card per watch, a menu of
// the watch types, and a new watch's pane that asks for its folder first.
// Once the host has found the folder, the watch is saved with its type's
// file types, and its full pane shows. A bank account is added here with
// its statements watch, whose folder can wait: its pane asks for the
// account, then the folder. Import, here and on the watch's source in the
// sidebar (`importWatch`), opens the import page of its type (images and
// PDFs: features/import/scans.js, through the `watch.import` action), or
// imports a statements watch's files into its account at once. The sidebar
// follows `watches_rev` (app.js).

import { accEl } from '../../core/dom.js';
import { oncePerFrame } from '../../core/dom.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import { money, pageDate, plural } from '../../core/format.js';
import { runAction } from '../../core/actions.js';
import { store } from '../../model/state.js';
import { entities, parseContact, watches } from '../../model/entities.js';
import { structureCall } from '../../model/structure.js';
import { showMenu } from '../../ui/menu.js';
import { armedButton } from '../../ui/confirm.js';
import { iconEl } from '../../ui/icons.js';
import { IMPORT_ICON, KIND_ICONS } from '../../ui/icons.js';
import {
  addTile, detailFooter, detailHead, detailPane, draftKind, itemCard, listDetailPage, listSection,
  notice, pageList, replaceVaultPage, sentDrafts, setPageFocus, stat, stats,
} from '../../ui/page.js';
import { fieldEditKey, fieldEdits, renderEditField, renderPatternField } from '../../ui/field.js';
import { accSeg } from '../../ui/segmented.js';
import { AI_ANALYSIS_OFF, aiPickerWithHint } from './models.js';

// The reading step's key in a watch's `models` (the host's `READ_STEP`).
export const WATCH_READ_STEP = 'read';

// The watch types (`type` in the watch file): how they show, what a new one
// suggests, how a file is read (`read`, always the first step: `collect`
// for pages joined into documents, `optional` if a file reads without a
// model, `what` the model reads in a refusal's words, `used` in "Used by",
// `previews` if the files have previews), the steps' notes and the import
// page's title and blurb. Account statements have none of these: each file
// goes into the watch's bank account as it is (import.md#statements). A new
// watch suggests the user's pictures or documents, on Linux under `~`.
const WATCH_HOME = navigator.platform.startsWith('Win') ? '%USERPROFILE%\\' : '~/';
const WATCH_TYPES = [
  {
    type: 'images', name: 'Images', hint: 'OCR', kind: 'Images \u00b7 OCR', icon: '#i-image',
    folder: `${WATCH_HOME}Pictures`,
    patterns: ['*.jpg', '*.jpeg', '*.png', '*.webp'],
    read: {
      name: 'Image OCR', ocr: true, collect: true, previews: true, what: 'OCR', used: 'OCR',
      note: 'The OCR model reads the text in each image. Images taken together become the pages of one document.',
    },
    analysis: {
      what: 'the scan analysis',
      off: 'A document keeps its first page\u2019s title, dated when its file was made.',
    },
    keep: {
      on: 'The scanned image beside its document in Scans/, opened from its card; a document of several images keeps them as one PDF (JPEG, PNG).',
      off: 'Only the text read goes into the vault; the images stay in the folder.',
    },
    page: { title: 'Import scans' },
  },
  {
    type: 'pdf', name: 'PDF', hint: 'text, scans', kind: 'PDF \u00b7 text, scans', icon: '#i-document-pdf',
    folder: `${WATCH_HOME}Documents`,
    patterns: ['*.pdf'],
    read: {
      name: 'Read', ocr: true, optional: true, what: 'the scanned pages', used: 'reading',
      note: 'Takes the text a PDF has; only scanned pages go to the model, as images. An encrypted PDF is imported without its text.',
    },
    analysis: {
      what: 'the PDF analysis',
      off: 'A document keeps its first line as its title, dated when its file was made.',
    },
    keep: {
      on: 'The PDF beside its document in Scans/, opened from its card.',
      off: 'Only the text read goes into the vault; the PDFs stay in the folder. An encrypted PDF keeps its file anyway.',
    },
    page: {
      title: 'Import PDFs',
      blurb: 'Each PDF is a document of its own. A PDF\u2019s own text is read here; only scanned pages go to the model.',
    },
  },
  {
    type: 'statements', name: 'Account statements', hint: 'CSV', kind: 'Account statements',
    icon: KIND_ICONS['bank-account'].icon,
    folder: `${WATCH_HOME}Documents`,
    patterns: ['*.csv'],
  },
];

// A bank account's kinds (`bank-account.type`), in words.
const ACCOUNT_KINDS = [['checking', 'Checking'], ['savings', 'Savings'], ['credit-card', 'Credit card']];

let selectedWatch = null; // a watch id, 'new:<type>' for a new watch's pane, or null
const watchDrafts = sentDrafts(); // watch id -> the changes sent, shown until done
let watchError = null; // { watch, message } from the last failed op
const knownWatches = new Set(); // the ids of the watches the page knows
// A new bank account's pane ('new:statements', `renderNewAccountPane`).
let accountDraft = newAccountDraft();

// What the new bank account's pane holds: the vault's account picked (its
// id; null for a new one), the new one's fields, the folder, whether it is
// being added, and the last refusal (the folder's apart).
function newAccountDraft() {
  return {
    account: null, title: '', number: '', type: null, institution: '', folder: '',
    saving: false, error: null, folderError: null,
  };
}

export function watchType(type) {
  return WATCH_TYPES.find(t => t.type === type);
}

// The watch's settings, with what was sent but isn't confirmed yet.
export function watchSettings(id) {
  return { ...watches.get(id), ...watchDrafts.get(id) };
}

// A watch's name: a statements watch's is its bank account's title.
export function watchName(watch) {
  if (watch.type !== 'statements') return watch.name;
  return entities.get(watch.account)?.title || 'Bank account';
}

// Import, here and on the watch's source: a statements watch's new files
// go into its account as a job (`statements_import`, its note in the jobs
// panel); the others open the import page of their type (`watch.import`).
// A refusal shows in the status bar.
export function importWatch(id) {
  if (watches.get(id)?.type === 'statements') rpcCall('statements_import', { watch: id });
  else runAction('watch.import', id);
}

// ----------------------------------------------------------------------------
// host events: the page's own state about the watches, after the model
// wrote them (model/events.js)

// The opened vault's watches replace the ones shown.
rpcOn('watches', ({ items }) => {
  const ids = new Set(items.map(item => item.id));
  for (const id of [...knownWatches]) {
    if (!ids.has(id)) onWatchRemoved(id);
  }
  for (const item of items) onWatch(item);
});
rpcOn('watch', meta => onWatch(meta));
rpcOn('watch_removed', ({ id }) => onWatchRemoved(id));

function onWatch(meta) {
  knownWatches.add(meta.id);
  if (selectedWatch === null) selectedWatch = meta.id;
}

function onWatchRemoved(id) {
  knownWatches.delete(id);
  watchDrafts.forget(id);
  for (const key of fieldEdits.keys()) {
    if (key.startsWith(fieldEditKey(id, ''))) fieldEdits.delete(key);
  }
  if (selectedWatch === id) selectedWatch = watches.keys().next().value ?? null;
}

// A failed call's message for the page; the host words most of them.
function watchErrorText(e) {
  return e.code === 'not_found' ? "The folder watch isn't there any more." : e.message;
}

// ----------------------------------------------------------------------------
// actions

// Send `set_watch` with `change` for `watch`; what of it is `shown` at
// once (`watchSettings`) stays until the last of the watch's calls has
// settled: the host's `watch` event, which comes before the reply, has the
// settings then.
function watchSetCall(watch, change, shown = {}) {
  return watchDrafts.send(watch, shown, () => rpcCall('set_watch', { watch, ...change }))
    .finally(scheduleRenderFolders);
}

// Show a new watch's pane, its folder ready for input; a new bank
// account's, its name.
function watchStartDraft(type) {
  const draft = `new:${type}`;
  fieldEdits.delete(fieldEditKey(draft, 'folder'));
  selectedWatch = draft;
  watchError = null;
  accountDraft = newAccountDraft();
  setPageFocus(fieldEditKey(draft, type === 'statements' ? 'title' : 'folder'));
  scheduleRenderFolders();
}

// Check and store `folder` for `watch` (an id, or 'new:<type>', which
// creates the watch with its type's defaults). The field shows it as being
// checked until the host answers; then it closes, or opens again with the
// error.
function watchSaveFolder(watch, folder) {
  const key = fieldEditKey(watch, 'folder');
  fieldEdits.set(key, { value: folder, checking: true, error: null });
  watchError = null;
  const type = watchType(draftKind(watch));
  let saved;
  if (type) {
    const settings = { type: type.type, folder, patterns: type.patterns };
    if (type.read.collect) settings.collect = 'together';
    saved = rpcCall('create_watch', settings).then(({ id }) => {
      if (selectedWatch === watch) selectedWatch = id;
    });
  } else {
    saved = watchSetCall(watch, { folder });
  }
  saved.then(() => fieldEdits.delete(key), e => {
    if (!fieldEdits.has(key)) return;
    fieldEdits.set(key, { ...fieldEdits.get(key), checking: false, error: watchErrorText(e) });
    setPageFocus(key);
  }).finally(scheduleRenderFolders);
  scheduleRenderFolders();
}

// Change some settings of `watch`; they show at once, and the host's
// `watch` event confirms them. A refusal shows in the watch's pane, or goes
// to `onError` (Import scans picking its model).
function watchSet(watch, change, onError = null) {
  watchError = null;
  watchSetCall(watch, change, change).catch(e => {
    if (onError) onError(e);
    else watchError = { watch, message: watchErrorText(e) };
  });
  scheduleRenderFolders();
}

// Browse: the folder picked is checked and stored.
function watchBrowse(watch) {
  rpcCall('pick_folder', { title: 'Folder to watch' }).then(({ path }) => {
    if (path) watchSaveFolder(watch, path);
  });
}

// Remove `watch`, on the second click of its button (`armedButton`).
function watchRemove(watch) {
  rpcCall('remove_watch', { watch }).catch(e => {
    watchError = { watch, message: watchErrorText(e) };
  }).finally(scheduleRenderFolders);
}

// The vault's bank accounts no watch imports into.
function freeAccounts() {
  const taken = new Set([...watches.values()].map(watch => watch.account).filter(Boolean));
  return [...entities.values()].filter(doc => doc.kind === 'bank-account' && !taken.has(doc.id));
}

// The vault's account the new bank account's pane picked, while no watch
// has it; else null, a new account.
function pickedAccount() {
  return freeAccounts().find(doc => doc.id === accountDraft.account) ?? null;
}

// Whether the new bank account's pane can be added: an account picked, or
// a new one's name, number and kind given.
function canAddAccount() {
  const d = accountDraft;
  if (d.saving) return false;
  return !!pickedAccount() || !!(d.title.trim() && d.number.trim() && d.type);
}

// Add the bank account of the new pane with its statements watch
// (`create_watch`): the account picked, or a new one of the fields given;
// the folder if one is given. A folder that isn't there shows under the
// folder, any other refusal under the pane's parts.
function addAccount() {
  if (!canAddAccount()) return;
  const d = accountDraft;
  const type = watchType('statements');
  const args = { type: type.type, patterns: type.patterns };
  if (d.folder.trim()) args.folder = d.folder.trim();
  const picked = pickedAccount();
  if (picked) {
    args.account = picked.id;
  } else {
    args.new_account = {
      title: d.title.trim(), number: d.number.trim(), type: d.type, institution: d.institution.trim(),
    };
  }
  Object.assign(d, { saving: true, error: null, folderError: null });
  rpcCall('create_watch', args).then(({ id }) => {
    if (selectedWatch === `new:${type.type}`) selectedWatch = id;
    if (accountDraft === d) accountDraft = newAccountDraft();
  }, e => {
    Object.assign(d, { saving: false, [e.code === 'file_not_found' ? 'folderError' : 'error']: e.message });
  }).finally(scheduleRenderFolders);
  scheduleRenderFolders();
}

// Change fields of the bank account `id` (`set_bank_account`): its bank,
// number or kind.
function setBankAccount(id, change) {
  structureCall('set_bank_account', { entity: id, ...change });
}

// ----------------------------------------------------------------------------
// rendering

const scheduleRenderFolders = oncePerFrame(renderFolders);

function renderFolders() {
  if (store.getState().vault_op !== 'folders') return;
  replaceVaultPage(listDetailPage(renderWatchList(), renderWatchDetail()));
}

function renderWatchList() {
  const list = pageList('Watch Folders',
    'Folders on this PC the vault reads from. A watch says where to look, which files to take and how to read them.');
  if (!store.getState().vault_path) {
    list.append(accEl('div', 'page-text', 'Folder watches are kept in the vault: open a vault to see them.'));
    return list;
  }
  const all = [...watches.values()];
  for (const [name, part] of [
    ['Folder watches', all.filter(watch => watch.type !== 'statements')],
    ['Bank accounts', all.filter(watch => watch.type === 'statements')],
  ]) {
    if (part.length) list.append(listSection(name), ...part.map(renderWatchCard));
  }
  list.append(listSection('Add'), renderWatchAdd(), renderAccountAdd());
  return list;
}

// A watch's card; a statements watch's names its account, by the end of
// its number.
function renderWatchCard(watch) {
  const type = watchType(watch.type);
  const settings = watchSettings(watch.id);
  const patterns = settings.patterns.length ? settings.patterns.join(' ') : 'no file types';
  const number = entities.get(watch.account)?.['bank-account']?.number;
  const status = settings.enabled ? (number ? `\u2026${number.slice(-4)}` : '') : 'off';
  return itemCard({
    icon: type.icon,
    kind: type.kind,
    status,
    title: watchName(settings),
    sub: `${settings.folder || 'No folder'} \u00b7 ${patterns}`,
    cut: true,
    current: watch.id === selectedWatch,
    onSelect: () => {
      selectedWatch = watch.id;
      scheduleRenderFolders();
    },
  });
}

// "Watch a folder": a menu of the types; choosing one opens its new pane.
// A statements watch comes with its bank account (`renderAccountAdd`).
function renderWatchAdd() {
  const kind = draftKind(selectedWatch);
  return addTile({
    label: 'Watch a folder',
    hint: 'type \u25be',
    current: !!kind && kind !== 'statements',
    onClick: tile => {
      const items = WATCH_TYPES.filter(type => type.type !== 'statements').map(type => ({
        label: type.name,
        hint: type.hint,
        run: () => watchStartDraft(type.type),
      }));
      showMenu(tile, items, { matchWidth: true });
    },
  });
}

// "Add a bank account": the new bank account's pane.
function renderAccountAdd() {
  return addTile({
    label: 'Add a bank account',
    hint: 'CSV',
    current: draftKind(selectedWatch) === 'statements',
    onClick: () => watchStartDraft('statements'),
  });
}

// The selected watch's pane, a new watch's or bank account's, or a hint
// while there is none.
function renderWatchDetail() {
  if (!store.getState().vault_path) return detailPane();
  const type = watchType(draftKind(selectedWatch));
  if (type) return type.type === 'statements' ? renderNewAccountPane(type) : renderNewWatchPane(type);
  const watch = watches.get(selectedWatch);
  if (watch) return renderWatchPane(watch);
  const pane = detailPane();
  pane.append(accEl('div', 'page-text',
    'No folder is watched yet. Choose Watch a folder, then the kind of files it holds, or Add a bank account.'));
  return pane;
}

// A grid of form rows; `row(label, ...content)` adds one and returns its
// field.
export function watchFormGrid() {
  const grid = accEl('div', 'form-grid');
  const row = (label, ...content) => {
    const field = accEl('div', 'form-field');
    field.append(...content);
    grid.append(accEl('span', 'label', label), field);
    return field;
  };
  return [grid, row];
}

// The folder of `id` (a watch, or 'new:<type>'): text with a pencil, or an
// input until the host has found it (starting with `value`), and Browse to
// pick one instead. Its note (being checked, or the error) goes on a line
// of its own.
function renderWatchFolder(id, saved, { value = saved, placeholder = value } = {}) {
  const key = fieldEditKey(id, 'folder');
  const [field, note] = renderEditField(id, 'folder', {
    saved,
    required: true,
    mono: true,
    value: value ?? '',
    placeholder: placeholder ?? '',
    editTitle: 'Change folder',
    onSave: folder => watchSaveFolder(id, folder),
    rerender: scheduleRenderFolders,
  });
  field.classList.add('watch__folder');
  const browse = accEl('button', 'btn', 'Browse\u2026');
  browse.type = 'button';
  browse.disabled = !!fieldEdits.get(key)?.checking;
  browse.addEventListener('click', () => watchBrowse(id));
  field.append(browse);
  if (note) note.classList.add('watch__note');
  return note ? [field, note] : [field];
}

// A new watch's pane: its folder.
function renderNewWatchPane(type) {
  const id = `new:${type.type}`;
  const pane = detailPane();
  pane.append(
    detailHead(`New folder watch \u00b7 ${type.kind}`, { icon: type.icon }),
    accEl('div', 'pane-title', type.name),
    accEl('div', 'page-text',
      `The folder is looked for before the watch is saved. It then takes ${type.patterns.join(' ')} and reads them as set for ${type.name.toLowerCase()} files; the file types can be changed then.`));
  const part = accEl('div', 'watch__part');
  const [grid, row] = watchFormGrid();
  row('Folder', ...renderWatchFolder(id, null, { value: type.folder }));
  part.append(accEl('div', 'section-head', 'Folder'), grid);
  pane.append(part, accEl('div', 'page-text', 'File types and the import process can be set once the watch is saved.'));
  return pane;
}

// A new bank account's pane (`accountDraft`): the account -- a new one's
// name, number and kind, which it needs, and its bank; or one of the
// vault's no watch has --, then the folder of its statements, which can
// wait. Add account saves both (`addAccount`).
function renderNewAccountPane(type) {
  const id = `new:${type.type}`;
  const d = accountDraft;
  const free = freeAccounts();
  const picked = pickedAccount();
  const pane = detailPane();
  pane.append(
    detailHead(`New bank account \u00b7 ${type.kind}`, { icon: type.icon }),
    accEl('div', 'pane-title', picked ? picked.title || 'Untitled' : 'Bank account'),
    accEl('div', 'page-text',
      'The account its statements go into, and the folder they are read from. The folder can be chosen later.'));

  const add = accEl('button', 'btn btn--primary', 'Add account');
  add.type = 'button';
  add.disabled = !canAddAccount();
  add.addEventListener('click', addAccount);
  // An input of the draft's `field`, in a field as wide as the row: typing
  // keeps it there and updates Add account, Enter adds.
  const input = (field, placeholder, mono = false) => {
    const el = accEl('input', mono ? 'input input--mono edit-field__input' : 'input edit-field__input');
    el.type = 'text';
    el.value = d[field];
    el.placeholder = placeholder;
    el.spellcheck = false;
    el.autocomplete = 'off';
    el.dataset.focusKey = fieldEditKey(id, field);
    el.disabled = d.saving;
    el.addEventListener('input', () => {
      d[field] = el.value;
      add.disabled = !canAddAccount();
    });
    el.addEventListener('keydown', e => {
      if (e.key === 'Enter') {
        e.preventDefault();
        addAccount();
      }
    });
    const wrap = accEl('span', 'edit-field edit-field--edit watch__folder');
    wrap.append(el);
    return wrap;
  };

  const account = accEl('div', 'watch__part');
  const [grid, row] = watchFormGrid();
  if (free.length) row('Account', renderAccountPick(picked, free));
  if (picked) {
    const fields = picked['bank-account'] ?? {};
    const kind = ACCOUNT_KINDS.find(([value]) => value === fields.type)?.[1];
    row('Bank', accEl('span', '', fields.institution || '\u2014'));
    row('Number', accEl('span', 'mono', fields.number || '\u2014'));
    row('Kind', accEl('span', '', kind ?? '\u2014'));
  } else {
    row('Name', input('title', 'its name, the source\u2019s too'));
    row('Number', input('number', 'as the bank writes it', true), accEl('span', 'note watch__note',
      'Checked in each file that names it: a file of another account is left out and named on import.'));
    row('Kind', accSeg(ACCOUNT_KINDS, d.type, kind => {
      d.type = kind;
      scheduleRenderFolders();
    }, d.saving));
    row('Bank', input('institution', 'optional'));
  }
  account.append(accEl('div', 'section-head', 'Account'), grid);

  const statements = accEl('div', 'watch__part');
  const head = accEl('div', 'section-head', 'Statements');
  head.append(accEl('span', 'section-head__note', 'can wait'));
  const [folderGrid, folderRow] = watchFormGrid();
  const folder = input('folder', type.folder, true);
  const browse = accEl('button', 'btn', 'Browse\u2026');
  browse.type = 'button';
  browse.disabled = d.saving;
  browse.addEventListener('click', () => {
    rpcCall('pick_folder', { title: 'Folder of the statements' }).then(({ path }) => {
      if (!path) return;
      Object.assign(d, { folder: path, folderError: null });
      scheduleRenderFolders();
    });
  });
  folder.append(browse);
  folderRow('Folder', folder, ...(d.folderError ? [accEl('span', 'error-note watch__note', d.folderError)] : []));
  folderRow('Format', accEl('span', '', 'comdirect (CSV)'), accEl('span', 'note watch__note',
    `Takes ${type.patterns.join(' ')}, each file a statement of the account.`));
  statements.append(head, folderGrid);

  pane.append(account, statements);
  if (d.error) pane.append(notice('error', d.error));
  pane.append(detailFooter('File types and the import can be changed once the account is added.', add));
  return pane;
}

// The new bank account's pick: a new account, or one of the vault's that
// no watch has (`free`).
function renderAccountPick(picked, free) {
  const pick = accEl('button', 'tool', picked ? picked.title || 'Untitled' : 'A new account');
  pick.type = 'button';
  pick.setAttribute('aria-haspopup', 'menu');
  pick.disabled = accountDraft.saving;
  pick.append(iconEl('#i-chevron-down'));
  pick.addEventListener('click', () => {
    const choose = account => () => {
      Object.assign(accountDraft, { account, error: null });
      scheduleRenderFolders();
    };
    const items = [{ label: 'A new account', hint: '', run: choose(null) }, ...free.map(doc => ({
      label: doc.title || 'Untitled',
      hint: doc['bank-account']?.number ?? '',
      run: choose(doc.id),
    }))];
    showMenu(pick, items, { matchWidth: true });
  });
  return pick;
}

// The pane's Import button (`importWatch`); without a folder it waits for
// one.
function importButton(id, title, folder = true) {
  const button = accEl('button', 'btn');
  button.type = 'button';
  button.title = folder ? title : 'Choose the folder of its statements first';
  button.disabled = !folder;
  button.append(iconEl(IMPORT_ICON), 'Import');
  button.addEventListener('click', () => importWatch(id));
  return button;
}

// A watch's pane: its head with Import, its name (a statements watch: its
// bank account, `renderStatementsParts`), its folder, the steps every file
// takes, an error, and Remove.
function renderWatchPane(watch) {
  const id = watch.id;
  const type = watchType(watch.type);
  const settings = watchSettings(id);
  const statements = watch.type === 'statements';
  const pane = detailPane();
  const button = statements
    ? importButton(id, 'Import the new statements into the account', !!settings.folder)
    : importButton(id, 'Read and import the files');
  pane.append(detailHead(`Folder watch \u00b7 ${type.kind} \u00b7 added ${pageDate(watch.added)}`,
    { icon: type.icon, button }));
  pane.append(...(statements ? renderStatementsParts(id, watch) : renderWatchName(id, settings)),
    renderWatchFolderPart(id, settings),
    statements ? renderStatementSteps() : renderWatchSteps(id, type, settings));
  if (watchError?.watch === id) {
    pane.append(notice('error', watchError.message));
  }

  const remove = accEl('button', 'btn btn--ghost', 'Remove\u2026');
  remove.type = 'button';
  armedButton(remove, 'Click again to remove', () => watchRemove(id), `remove_watch ${id}`);
  const kept = statements ? 'the account and every statement it imported' : 'everything it imported';
  pane.append(detailFooter(`Removing the watch keeps ${kept}.`, remove));
  return pane;
}

// A watch's name, its folder's while it has none.
function renderWatchName(id, settings) {
  const [name, nameNote] = renderEditField(id, 'name', {
    saved: settings.name,
    value: settings.name,
    textClass: 'pane-title edit-field__text',
    placeholder: 'empty for the folder\u2019s name',
    editTitle: 'Rename',
    onSave: value => {
      fieldEdits.delete(fieldEditKey(id, 'name'));
      watchSet(id, { name: value });
    },
    rerender: scheduleRenderFolders,
  });
  return nameNote ? [name, nameNote] : [name];
}

// A statements watch's bank account (import.md#the-folders-page): what it
// holds, its name (the watch's), bank, number and kind, its currency, owner
// and file as they are. Import writes the new files into the account at
// once, once the folder is chosen.
function renderStatementsParts(id, watch) {
  const account = entities.get(watch.account);
  if (!account) {
    return [accEl('div', 'pane-title', 'Bank account'),
      notice('error', 'The watch\u2019s bank account isn\u2019t in the vault: nothing can be imported.')];
  }
  return [...renderAccountName(id, account), renderAccountStats(account), renderAccountPart(id, account)];
}

// The account's name, the watch's too: renamed with `set_title`.
function renderAccountName(id, account) {
  const key = fieldEditKey(id, 'account');
  const [name, note] = renderEditField(id, 'account', {
    saved: account.title || null,
    value: account.title,
    required: true,
    textClass: 'pane-title edit-field__text',
    editTitle: 'Rename the account',
    onSave: title => {
      fieldEdits.delete(key);
      structureCall('set_title', { id: account.id, title });
      scheduleRenderFolders();
    },
    rerender: scheduleRenderFolders,
  });
  return note ? [name, note] : [name];
}

// What the account holds: its balance and what is pending, its bookings,
// and its newest statement.
function renderAccountStats(account) {
  const fields = account['bank-account'] ?? {};
  const statements = [...entities.values()].filter(doc => doc.transactions?.account === account.id);
  const bookings = statements.flatMap(doc => doc.transactions.transactions ?? []);
  const pending = bookings.filter(booking => booking.status === 'pending').length;
  const newest = statements.reduce((a, b) => (a && a.date >= b.date ? a : b), null);
  const balance = fields.balance != null ? money(fields.balance, fields.currency) : '\u2014';
  return stats(
    stat('Balance', balance, pending ? `${plural(pending, 'booking')} pending` : null),
    stat('Bookings', String(bookings.length - pending),
      statements.length ? `from ${plural(statements.length, 'statement')}` : 'none imported yet'),
    stat('Newest statement', newest?.title || 'None', newest?.source?.import?.origin ?? null));
}

// The account's fields: bank, number and kind changed here
// (`set_bank_account`), currency, owner and file as they are.
function renderAccountPart(id, account) {
  const fields = account['bank-account'] ?? {};
  const edit = (field, saved, title, mono = false) => {
    const key = fieldEditKey(id, field);
    const [wrap, note] = renderEditField(id, field, {
      saved: saved ?? null,
      empty: '\u2014',
      value: saved ?? '',
      mono,
      editTitle: title,
      onSave: value => {
        fieldEdits.delete(key);
        setBankAccount(account.id, { [field]: value });
        scheduleRenderFolders();
      },
      rerender: scheduleRenderFolders,
    });
    wrap.classList.add('watch__folder');
    return note ? [wrap, note] : [wrap];
  };
  const part = accEl('div', 'watch__part');
  const head = accEl('div', 'section-head', 'Account');
  head.append(accEl('span', 'section-head__note', 'its name is the source\u2019s'));
  const [grid, row] = watchFormGrid();
  row('Bank', ...edit('institution', fields.institution, 'Change the bank'));
  row('Number', ...edit('number', fields.number, 'Change the number', true),
    accEl('span', 'note watch__note', 'Checked in each file that names it: a file of another account is left out and named on import.'));
  row('Kind', accSeg(ACCOUNT_KINDS, fields.type ?? null, type => setBankAccount(account.id, { type })));
  row('Currency', fields.currency
    ? accEl('span', '', fields.currency) : accEl('span', 'note', 'read from the first file'));
  if (fields.owner) row('Owner', accEl('span', '', parseContact(fields.owner).name));
  const path = accEl('span', 'meta truncate-start', account.path);
  path.title = account.path;
  row('In the vault', path);
  part.append(head, grid);
  return part;
}

// The steps a statement takes, each always (import.md#statements).
function renderStatementSteps() {
  const part = accEl('div', 'watch__part');
  const head = accEl('div', 'section-head', 'Import process');
  head.append(accEl('span', 'section-head__note', 'for each file'));
  const steps = accEl('div', 'steps');
  steps.append(
    renderWatchStep({
      name: 'Read the rows', label: 'Read',
      note: 'Each row becomes a booking of the account. The files are read in the format of comdirect\u2019s CSV export.',
    }),
    renderWatchStep({
      name: 'Skip what is there', label: 'Rule',
      note: 'A booking the account has (the same reference, or the same day, amount and purpose) is left out, so exports may overlap. The newest statement replaces the pending bookings.',
    }),
    renderWatchStep({
      name: 'Check the balance', label: 'Rule',
      note: 'The bookings are checked against the statement\u2019s balances, and its first balance against the statement before it. A gap means a missing statement and is named when you import.',
    }));
  part.append(head, steps);
  return part;
}

// Where to look and which files to take, and whether the watch is a source.
// A statements watch's folder may not be chosen yet: then its field is open.
function renderWatchFolderPart(id, settings) {
  const part = accEl('div', 'watch__part');
  const [grid, row] = watchFormGrid();
  row('Folder', ...renderWatchFolder(id, settings.folder || null,
    { value: settings.folder, placeholder: watchType(settings.type)?.folder }));
  row('Subfolders',
    accEl('span', 'note', settings.subfolders ? 'Files in subfolders too' : 'Only files directly in this folder'),
    accSeg([[true, 'Include'], [false, 'Ignore']], settings.subfolders,
      subfolders => watchSet(id, { subfolders })));
  row('File types', ...renderPatternField(id, settings.patterns,
    patterns => watchSet(id, { patterns }), scheduleRenderFolders));
  row('Source',
    accEl('span', 'note', settings.enabled ? 'Listed under Sources, with its Import button' : 'Not listed under Sources'),
    accSeg([[true, 'On'], [false, 'Off']], settings.enabled, enabled => watchSet(id, { enabled })));
  part.append(accEl('div', 'section-head', 'Folder'), grid);
  return part;
}

// The import process: the type's read step, which always runs, then the
// analysis and Keep original, each switched by its check
// (import.md#the-folders-page); a step that sends the file to an AI shows
// the model it runs with, picked there (accounts.md#model-choice).
function renderWatchSteps(id, type, settings) {
  const part = accEl('div', 'watch__part');
  const head = accEl('div', 'section-head', 'Import process');
  head.append(accEl('span', 'section-head__note', 'for each file'));

  const steps = accEl('div', 'steps');
  steps.append(renderWatchStep({
    name: type.read.name, label: 'Read', note: type.read.note,
    ...(type.read.ocr ? watchModel(id, settings, WATCH_READ_STEP, type.read.what) : {}),
    options: type.read.collect ? renderWatchCollect(id, settings) : null,
  }), renderWatchAnalysis(id, type, settings), renderWatchKeepOriginal(id, type, settings));
  part.append(head, steps);
  return part;
}

// The model `step` runs with (`WATCH_READ_STEP` for reading): its picker,
// and what keeps the picked model from it (`aiPickerWithHint`).
function watchModel(id, settings, step, what) {
  const choice = settings.models?.[step] ?? null;
  const { picker, hint } = aiPickerWithHint(choice, pick => watchSetModel(id, step, pick), what);
  return { where: picker, hint };
}

// The analysis of the documents imported (`analysis`, as a mail filter's):
// its check switches it on at Lite, or off, keeping the model; while on,
// its level and the model it runs with.
function renderWatchAnalysis(id, type, settings) {
  const analysis = { ...AI_ANALYSIS_OFF, ...settings.analysis };
  const on = analysis.level !== 'off';
  const set = next => watchSet(id, { analysis: next });
  const step = {
    name: 'Analysis', label: 'Analyse', on,
    onToggle: () => set({ ...analysis, level: on ? 'off' : 'lite' }),
  };
  if (!on) {
    return renderWatchStep({ ...step, note: type.analysis.off });
  }
  const { picker, hint } = aiPickerWithHint(analysis.model,
    model => set({ ...analysis, model }), type.analysis.what);
  const options = accEl('dl', 'props step__opts');
  const value = accEl('dd');
  value.append(accSeg([['lite', 'Lite'], ['medium', 'Medium']], analysis.level,
    level => set({ ...analysis, level })));
  options.append(accEl('dt', 'label', 'Level'), value);
  const note = 'After the import, the model gives each document a clean title, its own date, a genre, labels and projects'
    + (analysis.level === 'medium' ? ', and points to check and questions as a chat linked to it' : '')
    + '. A title typed before the import stays.';
  return renderWatchStep({ ...step, note, where: picker, hint, options });
}

// Whether a document of one file keeps the file beside it
// (`keep_original`).
function renderWatchKeepOriginal(id, type, settings) {
  const on = !!settings.keep_original;
  return renderWatchStep({
    name: 'Keep original', label: 'Keep', on,
    onToggle: () => watchSet(id, { keep_original: !on }),
    note: on ? type.keep.on : type.keep.off,
  });
}

// Pick `step`'s model. The whole map is sent, so the watch shows the other
// steps' picks while this one is unconfirmed (`watchSettings`).
export function watchSetModel(id, step, choice, onError = null) {
  const models = { ...watchSettings(id).models, [step]: choice };
  watchSet(id, { models }, onError);
}

// One step: its check -- switching it (`on`, `onToggle`), or always on --,
// its title and kind, its note, options and what keeps its model from it
// (`hint`), and the picker of the model it runs with (`where`), if it
// sends the file to an AI.
function renderWatchStep({ name, label, note, where, hint, options, on = true, onToggle = null }) {
  const row = accEl('div', 'step');
  const check = accEl('button', 'check', on ? '\u2713' : '');
  check.type = 'button';
  check.setAttribute('aria-pressed', String(on));
  if (onToggle) {
    check.title = on ? 'Turn it off' : 'Turn it on';
    check.addEventListener('click', onToggle);
  } else {
    check.setAttribute('aria-disabled', 'true');
    check.title = 'Always runs';
  }
  const body = accEl('span', 'step__body');
  const title = accEl('span', 'step__title', name);
  title.append(accEl('span', 'label', label));
  body.append(title, accEl('span', 'note', note));
  if (options) body.append(options);
  if (hint) body.append(hint);
  const model = accEl('span', 'meta step__model');
  if (where) model.append(where);
  row.append(check, body, model);
  return row;
}

// Images: whether images taken together are one document.
function renderWatchCollect(id, settings) {
  const options = accEl('dl', 'props step__opts');
  const value = accEl('dd');
  value.append(accSeg([['together', 'Taken within 2 min'], ['single', 'One per image']],
    settings.collect ?? 'together', collect => watchSet(id, { collect })));
  options.append(accEl('dt', 'label', 'Collect'), value);
  return options;
}

// Re-render while visible when the op, the vault, the watches, the AI
// accounts (the reading step's model), or the entities (a statements
// watch's account) change.
store.subscribe(
  [state => state.vault_op, state => state.vault_path, state => state.watches_rev, state => state.accounts_rev,
    state => state.entities_rev],
  () => scheduleRenderFolders());
