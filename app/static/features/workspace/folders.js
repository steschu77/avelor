// Workspace -> Folders (import.md#the-folders-page;
// api.md#watches): the open vault's folder watches, each reading one folder on this
// PC. The page follows Workspace -> Accounts: a card per watch, a menu of
// the watch types, and a new watch's pane that asks for its folder first.
// Once the host has found the folder, the watch is saved with its type's
// file types, and its full pane shows.

import { accEl } from '../../core/dom.js';
import { oncePerFrame } from '../../core/dom.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import { pageDate } from '../../core/format.js';
import { store } from '../../model/state.js';
import { watches } from '../../model/entities.js';
import { showMenu } from '../../ui/menu.js';
import { armedButton } from '../../ui/confirm.js';
import {
  addTile, detailFooter, detailHead, detailPane, draftKind, itemCard, listDetailPage, listSection,
  notice, pageList, replaceVaultPage, sentDrafts, setPageFocus,
} from '../../ui/page.js';
import { fieldEditKey, fieldEdits, renderEditField } from '../../ui/field.js';
import { accSeg } from '../../ui/segmented.js';
import {
  aiAnyOffered, aiMissingHint, aiModelOf, aiModelPicker,
} from './models.js';

// The reading step's key in a watch's `models` (the host's `READ_STEP`).
export const WATCH_READ_STEP = 'read';

// The watch types (`type` in the watch file): how they show, what a new one
// suggests, how a file is read (`read`, always the first step). A new
// watch suggests the user's pictures, on Linux under `~`.
const WATCH_TYPES = [
  {
    type: 'images', name: 'Images', hint: 'OCR', kind: 'Images \u00b7 OCR', icon: '#i-image',
    folder: navigator.platform.startsWith('Win') ? '%USERPROFILE%\\Pictures' : '~/Pictures',
    patterns: ['*.jpg', '*.jpeg', '*.png', '*.webp'],
    read: {
      name: 'Image OCR', ocr: true, collect: true,
      note: 'The OCR model reads the text in each image. Images taken together become the pages of one document.',
    },
  },
];

let selectedWatch = null; // a watch id, 'new:<type>' for a new watch's pane, or null
const watchDrafts = sentDrafts(); // watch id -> the changes sent, shown until done
let watchError = null; // { watch, message } from the last failed op
const knownWatches = new Set(); // the ids of the watches the page knows

function watchType(type) {
  return WATCH_TYPES.find(t => t.type === type);
}

// The watch's settings, with what was sent but isn't confirmed yet.
function watchSettings(id) {
  return { ...watches.get(id), ...watchDrafts.get(id) };
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

// Show a new watch's pane, its folder ready for input.
function watchStartDraft(type) {
  const draft = `new:${type}`;
  fieldEdits.delete(fieldEditKey(draft, 'folder'));
  selectedWatch = draft;
  watchError = null;
  setPageFocus(fieldEditKey(draft, 'folder'));
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
// `watch` event confirms them. A refusal shows in the watch's pane.
function watchSet(watch, change, onError = null) {
  watchError = null;
  watchSetCall(watch, change, change).catch(e => {
    watchError = { watch, message: watchErrorText(e) };
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
  if (watches.size) list.append(listSection('Folder watches'));
  for (const watch of watches.values()) list.append(renderWatchCard(watch));
  list.append(listSection('Add'), renderWatchAdd());
  return list;
}

function renderWatchCard(watch) {
  const type = watchType(watch.type);
  const settings = watchSettings(watch.id);
  const patterns = settings.patterns.length ? settings.patterns.join(' ') : 'no file types';
  return itemCard({
    icon: type.icon,
    kind: type.kind,
    status: settings.enabled ? '' : 'off',
    title: settings.name,
    sub: `${settings.folder} \u00b7 ${patterns}`,
    cut: true,
    current: watch.id === selectedWatch,
    onSelect: () => {
      selectedWatch = watch.id;
      scheduleRenderFolders();
    },
  });
}

// "Watch a folder": a menu of the types; choosing one opens its new pane.
function renderWatchAdd() {
  return addTile({
    label: 'Watch a folder',
    hint: 'type \u25be',
    current: !!draftKind(selectedWatch),
    onClick: tile => {
      const items = WATCH_TYPES.map(type => ({
        label: type.name,
        hint: type.hint,
        run: () => watchStartDraft(type.type),
      }));
      showMenu(tile, items, { matchWidth: true });
    },
  });
}

// The selected watch's pane, a new watch's, or a hint while there is none.
function renderWatchDetail() {
  if (!store.getState().vault_path) return detailPane();
  const type = watchType(draftKind(selectedWatch));
  if (type) return renderNewWatchPane(type);
  const watch = watches.get(selectedWatch);
  if (watch) return renderWatchPane(watch);
  const pane = detailPane();
  pane.append(accEl('div', 'page-text',
    'No folder is watched yet. Choose Watch a folder, then the kind of files it holds.'));
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
// input until the host has found it, and Browse to pick one instead. Its
// note (being checked, or the error) goes on a line of its own.
function renderWatchFolder(id, saved, suggestion) {
  const key = fieldEditKey(id, 'folder');
  const [field, note] = renderEditField(id, 'folder', {
    saved,
    required: true,
    mono: true,
    value: saved ?? suggestion,
    placeholder: suggestion ?? '',
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

function renderNewWatchPane(type) {
  const id = `new:${type.type}`;
  const pane = detailPane();
  pane.append(
    detailHead(`New folder watch \u00b7 ${type.kind}`, { icon: type.icon }),
    accEl('div', 'pane-title', type.name),
    accEl('div', 'page-text',
      `The folder is looked for before the watch is saved. It then takes ${type.patterns.join(' ')} and reads them as set for ${type.name.toLowerCase()} files; both can be changed then.`));
  const part = accEl('div', 'watch__part');
  const [grid, row] = watchFormGrid();
  row('Folder', ...renderWatchFolder(id, null, type.folder));
  part.append(accEl('div', 'section-head', 'Folder'), grid);
  pane.append(part, accEl('div', 'page-text', 'File types and the import process can be set once the watch is saved.'));
  return pane;
}

function renderWatchPane(watch) {
  const id = watch.id;
  const type = watchType(watch.type);
  const settings = watchSettings(id);
  const pane = detailPane();

  pane.append(detailHead(`Folder watch \u00b7 ${type.kind} \u00b7 added ${pageDate(watch.added)}`,
    { icon: type.icon }));

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
  pane.append(name);
  if (nameNote) pane.append(nameNote);

  pane.append(renderWatchFolderPart(id, settings), renderWatchSteps(id, type, settings));
  if (watchError?.watch === id) {
    pane.append(notice('error', watchError.message));
  }

  const remove = accEl('button', 'btn btn--ghost', 'Remove\u2026');
  remove.type = 'button';
  armedButton(remove, 'Click again to remove', () => watchRemove(id), `remove_watch ${id}`);
  pane.append(detailFooter('Removing the watch keeps everything it imported.', remove));
  return pane;
}

// Where to look and which files to take.
function renderWatchFolderPart(id, settings) {
  const part = accEl('div', 'watch__part');
  const [grid, row] = watchFormGrid();
  row('Folder', ...renderWatchFolder(id, settings.folder));
  row('Subfolders',
    accEl('span', 'note', settings.subfolders ? 'Files in subfolders too' : 'Only files directly in this folder'),
    accSeg([[true, 'Include'], [false, 'Ignore']], settings.subfolders,
      subfolders => watchSet(id, { subfolders })));
  row('File types', ...renderWatchPatterns(id, settings.patterns));
  part.append(accEl('div', 'section-head', 'Folder'), grid);
  return part;
}

// A chip per pattern (click to remove it), then an input that adds what is
// typed on Enter (several separated by commas).
function renderWatchPatterns(id, patterns) {
  const chips = patterns.map(pattern => {
    const chip = accEl('button', 'chip chip--action chip--mono', `${pattern} \u00d7`);
    chip.type = 'button';
    chip.title = `Remove ${pattern}`;
    chip.addEventListener('click', () => watchSet(id, { patterns: patterns.filter(p => p !== pattern) }));
    return chip;
  });

  const key = fieldEditKey(id, 'pattern');
  const input = accEl('input', 'form-input mono mono--small');
  input.type = 'text';
  input.placeholder = patterns.length ? 'add\u2026' : 'add a pattern, like *.pdf';
  input.spellcheck = false;
  input.autocomplete = 'off';
  input.dataset.focusKey = key;
  input.value = fieldEdits.get(key)?.value ?? '';
  input.addEventListener('input', () => fieldEdits.set(key, { value: input.value }));
  input.addEventListener('keydown', e => {
    if (e.key !== 'Enter') return;
    e.preventDefault();
    const added = input.value.split(/[,;]/).map(p => p.trim()).filter(p => p && !patterns.includes(p));
    fieldEdits.delete(key);
    setPageFocus(key);
    if (added.length) watchSet(id, { patterns: [...patterns, ...new Set(added)] });
    else scheduleRenderFolders();
  });
  return [...chips, input];
}

// The import process: the type's read step, which always runs; a step
// that sends the file to an AI shows the model it runs with, picked there
// (accounts.md#model-choice).
function renderWatchSteps(id, type, settings) {
  const part = accEl('div', 'watch__part');
  const head = accEl('div', 'section-head', 'Import process');
  head.append(accEl('span', 'section-head__note', 'for each file'));

  const steps = accEl('div', 'steps');
  steps.append(renderWatchStep({
    name: type.read.name, label: 'Read', note: type.read.note,
    ...(type.read.ocr ? watchModel(id, settings, WATCH_READ_STEP, 'OCR') : {}),
    options: type.read.collect ? renderWatchCollect(id, settings) : null,
  }));
  part.append(head, steps);
  return part;
}

// The model `step` runs with (`WATCH_READ_STEP` for reading): its picker,
// and what keeps the picked model from it (`aiMissingHint`) -- not for a
// plain "pick a model", which the picker says itself.
function watchModel(id, settings, step, what) {
  const choice = settings.models?.[step] ?? null;
  const ai = aiModelOf(choice);
  const plainPick = ai.problem === 'pick' && aiAnyOffered();
  let hint = null;
  if (ai.problem && !plainPick) {
    hint = notice('hint', ...aiMissingHint(ai, what));
  }
  return { where: aiModelPicker(choice, pick => watchSetModel(id, step, pick)), hint };
}

// Pick `step`'s model. The whole map is sent, so the watch shows the other
// steps' picks while this one is unconfirmed (`watchSettings`).
function watchSetModel(id, step, choice, onError = null) {
  const models = { ...watchSettings(id).models, [step]: choice };
  watchSet(id, { models }, onError);
}

// One step: its check, which always runs, its title and kind, its note,
// options and what keeps its model from it (`hint`), and the picker of the
// model it runs with (`where`), if it sends the file to an AI.
function renderWatchStep({ name, label, note, where, hint, options }) {
  const row = accEl('div', 'step');
  const check = accEl('button', 'check', '\u2713');
  check.type = 'button';
  check.setAttribute('aria-pressed', 'true');
  check.setAttribute('aria-disabled', 'true');
  check.title = 'Always runs';
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

// Re-render while visible when the op, the vault, the watches, or the AI
// accounts (the reading step's model) change.
store.subscribe(
  [state => state.vault_op, state => state.vault_path, state => state.watches_rev, state => state.accounts_rev],
  () => scheduleRenderFolders());
