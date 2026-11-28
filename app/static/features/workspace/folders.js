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
import { titled } from '../../core/dom.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import { money, pageDate } from '../../core/format.js';
import { registerActions, runAction } from '../../core/actions.js';
import { t } from '../../core/text.js';
import { store } from '../../model/state.js';
import { entities, parseContact, watches } from '../../model/entities.js';
import { ACCOUNT_TYPES, ledger, statementsOf } from '../../model/bank.js';
import { setBankAccountLocal, structureCall } from '../../model/structure.js';
import { showMenu } from '../../ui/menu.js';
import { armedButton } from '../../ui/confirm.js';
import { iconEl } from '../../ui/icons.js';
import { IMPORT_ICON, KIND_ICONS } from '../../ui/icons.js';
import {
  addTile, detailFooter, detailHead, detailPane, draftKind, itemCard, listDetailPage, listSection,
  notice, openVaultOp, pageList, replaceVaultPage, sentDrafts, setPageFocus, stat, stats,
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
// page's title, blurb and a file's date (an image's taken, a PDF's date, the
// earlier of its created and modified times either way). Account statements have none of these: each file
// goes into the watch's bank account as it is (import.md#statements). A new
// watch suggests the user's pictures or documents, on Linux under `~`.
const WATCH_HOME = navigator.platform.startsWith('Win') ? '%USERPROFILE%\\' : '~/';
const WATCH_TYPES = [
  {
    type: 'images', command: name => t('watch.import_scans', { name }), also: t('watch.scans_also'),
    name: t('watch.type.images'), hint: 'OCR', kind: t('watch.kind.images'), icon: '#i-image',
    files: t('watch.files.images'),
    folder: `${WATCH_HOME}Pictures`,
    patterns: ['*.jpg', '*.jpeg', '*.png', '*.webp'],
    read: {
      name: t('watch.read.image_ocr'), ocr: true, collect: true, previews: true, what: t('feature.ocr'), used: t('feature.ocr'),
      note: t('watch.read.images_note'),
    },
    analysis: {
      what: t('feature.scan_analysis'),
      off: t('watch.analysis_off.images'),
    },
    keep: {
      on: t('watch.keep_on.images'),
      off: t('watch.keep_off.images'),
    },
    page: {
      title: t('import.scans_title'),
      date: t('scans.taken'),
      dated: date => t('scans.taken_at', { date }),
    },
  },
  {
    type: 'pdf', command: name => t('watch.import_pdfs', { name }),
    name: 'PDF', hint: t('watch.hint.pdf'), kind: t('watch.kind.pdf'), icon: '#i-document-pdf',
    files: t('watch.files.pdf'),
    folder: `${WATCH_HOME}Documents`,
    patterns: ['*.pdf'],
    read: {
      name: t('watch.read.pdf'), ocr: true, optional: true, what: t('feature.scanned_pages'), used: t('accounts.step.reading'),
      note: t('watch.read.pdf_note'),
    },
    analysis: {
      what: t('feature.pdf_analysis'),
      off: t('watch.analysis_off.pdf'),
    },
    keep: {
      on: t('watch.keep_on.pdf'),
      off: t('watch.keep_off.pdf'),
    },
    page: {
      title: t('import.pdfs_title'),
      blurb: t('import.pdfs_blurb'),
      date: t('scans.date'),
      dated: date => t('scans.date_at', { date }),
    },
  },
  {
    type: 'statements', command: name => t('watch.import_new_statements', { name }), also: t('watch.statements_also'),
    name: t('watch.type.statements'), hint: 'CSV', kind: t('watch.type.statements'),
    icon: KIND_ICONS['bank-account'].icon,
    folder: `${WATCH_HOME}Documents`,
    patterns: ['*.csv'],
  },
];

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
    account: null, title: '', number: '', type: null, bank: '', folder: '',
    saving: false, error: null, folderError: null,
  };
}

export function watchType(type) {
  return WATCH_TYPES.find(entry => entry.type === type);
}

// The watch's settings, with what was sent but isn't confirmed yet.
export function watchSettings(id) {
  return { ...watches.get(id), ...watchDrafts.get(id) };
}

// A watch's name: a statements watch's is its bank account's title.
export function watchName(watch) {
  if (watch.type !== 'statements') return watch.name;
  return entities.get(watch.account)?.title || t('kind.bank_account');
}

// Import, here and on the watch's source: a statements watch's new files
// go into its account as a job (`statements_import`, its summary in the jobs
// panel); the others open the import page of their type (`watch.import`).
// A refusal shows in the status bar.
export function importWatch(id) {
  if (watches.get(id)?.type === 'statements') rpcCall('statements_import', { watch: id });
  else runAction('watch.import', id);
}

registerActions(() => [...watches.values()].filter(watch => watch.enabled && watch.folder).map(watch => {
  const type = watchType(watch.type);
  return {
    id: `import.${watch.id}`, group: 'get', title: type.command(watchName(watch)), icon: IMPORT_ICON,
    also: type.also, page: watch.type !== 'statements',
    run: () => importWatch(watch.id),
  };
}));

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
  return e.code === 'not_found' ? t('watch.gone') : e.message;
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

// The Folders page with a new bank account's pane: the Bank lens's empty
// state.
export function addBankAccount() {
  watchStartDraft('statements');
  openVaultOp('folders');
}

// The Folders page with the watch `id`'s pane: its source's Settings...
// (features/sidebar.js).
export function openWatch(id) {
  selectedWatch = id;
  watchError = null;
  openVaultOp('folders');
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
  rpcCall('pick_folder', { title: t('watch.pick_title') }).then(({ path }) => {
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
// a new one's name, number and type given.
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
      title: d.title.trim(), number: d.number.trim(), type: d.type, bank: d.bank.trim(),
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

// ----------------------------------------------------------------------------
// rendering

const scheduleRenderFolders = oncePerFrame(renderFolders);

function renderFolders() {
  if (store.getState().vault_op !== 'folders') return;
  replaceVaultPage(listDetailPage(renderWatchList(), renderWatchDetail()));
}

function renderWatchList() {
  const list = pageList(t('vault_op.folders'), t('watch.intro'));
  if (!store.getState().vault_path) {
    list.append(accEl('div', 'page-text', t('watch.no_vault')));
    return list;
  }
  const all = [...watches.values()];
  for (const [name, part] of [
    [t('watch.section.watches'), all.filter(watch => watch.type !== 'statements')],
    [t('watch.section.accounts'), all.filter(watch => watch.type === 'statements')],
  ]) {
    if (part.length) list.append(listSection(name), ...part.map(renderWatchCard));
  }
  list.append(listSection(t('watch.section.add')), renderWatchAdd(), renderAccountAdd());
  return list;
}

// A watch's card; a statements watch's names its account, by the end of
// its number.
function renderWatchCard(watch) {
  const type = watchType(watch.type);
  const settings = watchSettings(watch.id);
  const patterns = settings.patterns.length ? settings.patterns.join(' ') : t('watch.no_file_types');
  const number = entities.get(watch.account)?.['bank-account']?.number;
  const status = settings.enabled ? (number ? `\u2026${number.slice(-4)}` : '') : t('watch.status_off');
  return itemCard({
    icon: type.icon,
    kind: type.kind,
    status,
    title: watchName(settings),
    sub: `${settings.folder || t('watch.folder_unset')} \u00b7 ${patterns}`,
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
    label: t('watch.add_watch'),
    hint: t('watch.add_watch_hint'),
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
    label: t('bank.add_account'),
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
  pane.append(accEl('div', 'page-text', t('watch.none_yet')));
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

// The folder of `id` (a watch, or 'new:<type>'): text with its pen, or an
// input until the host has found it (starting with `value`), and Browse to
// pick one instead. Its note (being checked, or the error) goes on a line
// of its own.
function renderWatchFolder(id, saved, { value = saved, placeholder = value } = {}) {
  const key = fieldEditKey(id, 'folder');
  const [field, note] = renderEditField(id, 'folder', {
    saved,
    required: true,
    checked: true,
    mono: true,
    value: value ?? '',
    placeholder: placeholder ?? '',
    editTitle: t('watch.change_folder'),
    onSave: folder => watchSaveFolder(id, folder),
    rerender: scheduleRenderFolders,
  });
  field.classList.add('watch__folder');
  const browse = accEl('button', 'btn', t('watch.browse'));
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
    detailHead(t('watch.new_head', { kind: type.kind }), { icon: type.icon }),
    accEl('div', 'pane-title', type.name),
    accEl('div', 'page-text', t('watch.new_note', { patterns: type.patterns.join(' '), files: type.files })));
  const part = accEl('div', 'watch__part');
  const [grid, row] = watchFormGrid();
  row(t('watch.folder'), ...renderWatchFolder(id, null, { value: type.folder }));
  part.append(accEl('div', 'section-head', t('watch.folder')), grid);
  pane.append(part, accEl('div', 'page-text', t('watch.set_once_saved')));
  return pane;
}

// A new bank account's pane (`accountDraft`): the account -- a new one's
// name, number and type, which it needs, and its bank; or one of the
// vault's no watch has --, then the folder of its statements, which can
// wait. Add account saves both (`addAccount`).
function renderNewAccountPane(type) {
  const id = `new:${type.type}`;
  const d = accountDraft;
  const free = freeAccounts();
  const picked = pickedAccount();
  const pane = detailPane();
  pane.append(
    detailHead(t('watch.new_account_head', { kind: type.kind }), { icon: type.icon }),
    accEl('div', 'pane-title', picked ? picked.title || t('untitled') : t('kind.bank_account')),
    accEl('div', 'page-text', t('watch.new_account_note')));

  const add = accEl('button', 'btn btn--primary', t('watch.add_account'));
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
  if (free.length) row(t('props.account'), renderAccountPick(picked, free));
  if (picked) {
    const fields = picked['bank-account'] ?? {};
    const type = ACCOUNT_TYPES.find(([value]) => value === fields.type)?.[1];
    row(t('props.bank'), accEl('span', '', fields.bank || '\u2014'));
    row(t('props.number'), accEl('span', 'mono', fields.number || '\u2014'));
    row(t('props.type'), accEl('span', '', type ?? '\u2014'));
  } else {
    row(t('docs.name'), input('title', t('watch.name_placeholder')));
    row(t('props.number'), input('number', t('watch.number_placeholder'), true),
      accEl('span', 'note watch__note', t('watch.number_note')));
    row(t('props.type'), accSeg(ACCOUNT_TYPES, d.type, type => {
      d.type = type;
      scheduleRenderFolders();
    }, d.saving));
    row(t('props.bank'), input('bank', t('watch.optional')));
  }
  account.append(accEl('div', 'section-head', t('props.account')), grid);

  const statements = accEl('div', 'watch__part');
  const head = accEl('div', 'section-head', t('watch.statements'));
  head.append(accEl('span', 'section-head__note', t('watch.can_wait')));
  const [folderGrid, folderRow] = watchFormGrid();
  const folder = input('folder', type.folder, true);
  const browse = accEl('button', 'btn', t('watch.browse'));
  browse.type = 'button';
  browse.disabled = d.saving;
  browse.addEventListener('click', () => {
    rpcCall('pick_folder', { title: t('watch.pick_statements_title') }).then(({ path }) => {
      if (!path) return;
      Object.assign(d, { folder: path, folderError: null });
      scheduleRenderFolders();
    });
  });
  folder.append(browse);
  folderRow(t('watch.folder'), folder, ...(d.folderError ? [accEl('span', 'error-note watch__note', d.folderError)] : []));
  folderRow(t('watch.format'), accEl('span', '', 'comdirect (CSV)'), accEl('span', 'note watch__note',
    t('watch.takes_statements', { patterns: type.patterns.join(' ') })));
  statements.append(head, folderGrid);

  pane.append(account, statements);
  if (d.error) pane.append(notice('error', d.error));
  pane.append(detailFooter(t('watch.change_once_added'), add));
  return pane;
}

// The new bank account's pick: a new account, or one of the vault's that
// no watch has (`free`).
function renderAccountPick(picked, free) {
  const pick = accEl('button', 'tool', picked ? picked.title || t('untitled') : t('watch.new_account'));
  pick.type = 'button';
  pick.setAttribute('aria-haspopup', 'menu');
  pick.disabled = accountDraft.saving;
  pick.append(iconEl('#i-chevron-down'));
  pick.addEventListener('click', () => {
    const choose = account => () => {
      Object.assign(accountDraft, { account, error: null });
      scheduleRenderFolders();
    };
    const items = [{ label: t('watch.new_account'), hint: '', run: choose(null) }, ...free.map(doc => ({
      label: doc.title || t('untitled'),
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
  button.title = folder ? title : t('watch.choose_folder_first');
  button.disabled = !folder;
  button.append(iconEl(IMPORT_ICON), t('watch.import'));
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
    ? importButton(id, t('watch.import_statements'), !!settings.folder)
    : importButton(id, t('watch.import_files'));
  pane.append(detailHead(t('watch.head', { kind: type.kind, date: pageDate(watch.added) }),
    { icon: type.icon, button }));
  pane.append(...(statements ? renderStatementsParts(id, watch) : renderWatchName(id, settings)),
    renderWatchFolderPart(id, settings),
    statements ? renderStatementSteps(id, settings) : renderWatchSteps(id, type, settings));
  if (watchError?.watch === id) {
    pane.append(notice('error', watchError.message));
  }

  const remove = accEl('button', 'btn btn--ghost', t('watch.remove_dots'));
  remove.type = 'button';
  armedButton(remove, t('watch.remove_again'), () => watchRemove(id), `remove_watch ${id}`);
  pane.append(detailFooter(statements ? t('watch.remove_note_statements') : t('watch.remove_note'), remove));
  return pane;
}

// A watch's name, its folder's while it has none.
function renderWatchName(id, settings) {
  const [name, nameNote] = renderEditField(id, 'name', {
    saved: settings.name,
    value: settings.name,
    textClass: 'pane-title edit-field__text',
    placeholder: t('watch.name_empty'),
    editTitle: t('panel.rename'),
    byClick: true,
    onSave: value => {
      fieldEdits.delete(fieldEditKey(id, 'name'));
      watchSet(id, { name: value });
    },
    rerender: scheduleRenderFolders,
  });
  return nameNote ? [name, nameNote] : [name];
}

// A statements watch's bank account (import.md#the-folders-page): what it
// holds, its name (the watch's), bank, number and type, its currency, holder
// and file as they are. Import writes the new files into the account at
// once, once the folder is chosen.
function renderStatementsParts(id, watch) {
  const account = entities.get(watch.account);
  if (!account) {
    return [accEl('div', 'pane-title', t('kind.bank_account')),
      notice('error', t('watch.account_gone_note'))];
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
    editTitle: t('watch.rename_account'),
    byClick: true,
    onSave: title => {
      fieldEdits.delete(key);
      structureCall('set_title', { id: account.id, title });
      scheduleRenderFolders();
    },
    rerender: scheduleRenderFolders,
  });
  return note ? [name, note] : [name];
}

// What the account holds: its balance and what is pending, its transactions,
// and its newest statement.
function renderAccountStats(account) {
  const fields = account['bank-account'] ?? {};
  const statements = statementsOf(account.id);
  const book = ledger(account);
  const pending = book.pending.length;
  const newest = statements.reduce((a, b) => (a && a.date >= b.date ? a : b), null);
  const balance = book.balance != null ? money(book.balance, fields.currency) : '\u2014';
  return stats(
    stat(t('props.balance'), balance, pending ? t('watch.transactions_pending', { n: pending }) : null),
    stat(t('props.transactions'), String(book.entries.length),
      statements.length ? t('watch.from_statements', { n: statements.length }) : t('watch.none_imported')),
    stat(t('watch.newest_statement'), newest?.title || t('watch.none'), newest?.source?.import?.origin ?? null));
}

// The account's fields: bank, number, type and holder changed here
// (`set_bank_account`), currency and file as they are.
function renderAccountPart(id, account) {
  const fields = account['bank-account'] ?? {};
  const edit = (field, saved, title, mono = false, shown = saved) => {
    const key = fieldEditKey(id, field);
    const [wrap, note] = renderEditField(id, field, {
      saved: shown ?? null,
      empty: '\u2014',
      value: shown ?? '',
      mono,
      editTitle: title,
      onSave: value => {
        fieldEdits.delete(key);
        setBankAccountLocal(account.id, { [field]: value });
        scheduleRenderFolders();
      },
      rerender: scheduleRenderFolders,
    });
    wrap.classList.add('watch__folder');
    return note ? [wrap, note] : [wrap];
  };
  const part = accEl('div', 'watch__part');
  const head = accEl('div', 'section-head', t('props.account'));
  head.append(accEl('span', 'section-head__note', t('watch.name_is_source')));
  const [grid, row] = watchFormGrid();
  row(t('props.bank'), ...edit('bank', fields.bank, t('watch.change_bank')));
  row(t('props.number'), ...edit('number', fields.number, t('watch.change_number'), true),
    accEl('span', 'note watch__note', t('watch.number_note')));
  row(t('props.type'), accSeg(ACCOUNT_TYPES, fields.type ?? null, type => setBankAccountLocal(account.id, { type })));
  row(t('watch.currency'), fields.currency
    ? accEl('span', '', fields.currency) : accEl('span', 'note', t('watch.currency_from_file')));
  row(t('props.holder'), ...edit('holder', fields.holder, t('watch.change_holder'), false,
    fields.holder ? parseContact(fields.holder).name : null));
  const path = accEl('span', 'meta truncate-start', account.path);
  path.title = account.path;
  row(t('accounts.in_vault'), path);
  part.append(head, grid);
  return part;
}

// The steps a statement takes (import.md#statements), each always but
// Pair transfers, switched by its check (`pair_transfers`).
function renderStatementSteps(id, settings) {
  const pair = settings.pair_transfers !== false;
  const part = accEl('div', 'watch__part');
  const head = accEl('div', 'section-head', t('watch.import_process'));
  head.append(accEl('span', 'section-head__note', t('watch.for_each_file')));
  const steps = accEl('div', 'steps');
  steps.append(
    renderWatchStep({
      name: t('watch.step.read_rows'), label: t('watch.label.read'),
      note: t('watch.step.read_rows_note'),
    }),
    renderWatchStep({
      name: t('watch.step.skip'), label: t('watch.label.rule'),
      note: t('watch.step.skip_note'),
    }),
    renderWatchStep({
      name: t('watch.step.pair'), label: t('watch.label.rule'), on: pair,
      onToggle: () => watchSet(id, { pair_transfers: !pair }),
      note: pair ? t('watch.step.pair_on') : t('watch.step.pair_off'),
    }),
    renderWatchStep({
      name: t('watch.step.balance'), label: t('watch.label.rule'),
      note: t('watch.step.balance_note'),
    }));
  part.append(head, steps);
  return part;
}

// Where to look and which files to take, and whether the watch is a source.
// A statements watch's folder may not be chosen yet: then its field is open.
function renderWatchFolderPart(id, settings) {
  const part = accEl('div', 'watch__part');
  const [grid, row] = watchFormGrid();
  row(t('watch.folder'), ...renderWatchFolder(id, settings.folder || null,
    { value: settings.folder, placeholder: watchType(settings.type)?.folder }));
  row(t('watch.subfolders'),
    accEl('span', 'note', settings.subfolders ? t('watch.subfolders_in') : t('watch.subfolders_out')),
    accSeg([[true, t('watch.include')], [false, t('watch.ignore')]], settings.subfolders,
      subfolders => watchSet(id, { subfolders })));
  row(t('watch.file_types'), ...renderPatternField(id, settings.patterns,
    patterns => watchSet(id, { patterns }), scheduleRenderFolders));
  row(t('watch.source'),
    accEl('span', 'note', settings.enabled ? t('watch.source_on') : t('watch.source_off')),
    accSeg([[true, t('watch.on')], [false, t('watch.off')]], settings.enabled, enabled => watchSet(id, { enabled })));
  part.append(accEl('div', 'section-head', t('watch.folder')), grid);
  return part;
}

// The import process: the type's read step, which always runs, then the
// analysis and Keep original, each switched by its check
// (import.md#the-folders-page); a step that sends the file to an AI shows
// the model it runs with, picked there (accounts.md#model-choice).
function renderWatchSteps(id, type, settings) {
  const part = accEl('div', 'watch__part');
  const head = accEl('div', 'section-head', t('watch.import_process'));
  head.append(accEl('span', 'section-head__note', t('watch.for_each_file')));

  const steps = accEl('div', 'steps');
  steps.append(renderWatchStep({
    name: type.read.name, label: t('watch.label.read'), note: type.read.note,
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

// The analysis of the documents imported (`analysis`, as an email filter's):
// its check switches it on at Lite, or off, keeping the model; while on,
// its level and the model it runs with.
function renderWatchAnalysis(id, type, settings) {
  const analysis = { ...AI_ANALYSIS_OFF, ...settings.analysis };
  const on = analysis.level !== 'off';
  const set = next => watchSet(id, { analysis: next });
  const step = {
    name: t('watch.step.analysis'), label: t('watch.label.analyse'), on,
    onToggle: () => set({ ...analysis, level: on ? 'off' : 'lite' }),
  };
  if (!on) {
    return renderWatchStep({ ...step, note: type.analysis.off });
  }
  const { picker, hint } = aiPickerWithHint(analysis.model,
    model => set({ ...analysis, model }), type.analysis.what);
  const options = accEl('dl', 'props step__opts');
  const value = accEl('dd');
  value.append(accSeg([['lite', t('watch.level.lite')], ['medium', t('watch.level.medium')]], analysis.level,
    level => set({ ...analysis, level })));
  options.append(accEl('dt', 'label', t('watch.level')), value);
  const note = analysis.level === 'medium' ? t('watch.analysis_note_medium') : t('watch.analysis_note_lite');
  return renderWatchStep({ ...step, note, where: picker, hint, options });
}

// Whether a document of one file keeps the file beside it
// (`keep_original`).
function renderWatchKeepOriginal(id, type, settings) {
  const on = !!settings.keep_original;
  return renderWatchStep({
    name: t('watch.step.keep'), label: t('watch.label.keep'), on,
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
    titled(check, on ? t('watch.turn_off') : t('watch.turn_on'));
    check.addEventListener('click', onToggle);
  } else {
    check.setAttribute('aria-disabled', 'true');
    titled(check, t('watch.always_runs'));
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
  value.append(accSeg([['together', t('watch.collect.together')], ['single', t('watch.collect.single')]],
    settings.collect ?? 'together', collect => watchSet(id, { collect })));
  options.append(accEl('dt', 'label', t('watch.collect')), value);
  return options;
}

// Re-render while visible when the op, the vault, the watches, the AI
// accounts (the reading step's model), or the entities (a statements
// watch's account) change.
store.subscribe(
  [state => state.vault_op, state => state.vault_path, state => state.watches_rev, state => state.accounts_rev,
    state => state.entities_rev],
  () => scheduleRenderFolders());
