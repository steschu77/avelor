// Workspace -> Folders (import.md#the-folders-page;
// api.md#watches): the open vault's folder watches, each reading one folder on this
// PC. The page follows Workspace -> Accounts: a card per watch, a menu of
// the watch types, and a new watch's pane that asks for its folder first.
// Once the host has found the folder, the watch is saved with its type's
// file types, and its full pane shows. Import, here and on the watch's
// source in the sidebar, opens the import page of its type (images:
// features/import/scans.js, through the `watch.import` action). The
// sidebar follows `watches_rev` (app.js).

import { accEl } from '../../core/dom.js';
import { oncePerFrame } from '../../core/dom.js';
import { titled } from '../../core/dom.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import { pageDate } from '../../core/format.js';
import { registerActions, runAction } from '../../core/actions.js';
import { t } from '../../core/text.js';
import { store } from '../../model/state.js';
import { watches } from '../../model/entities.js';
import { showMenu } from '../../ui/menu.js';
import { armedButton } from '../../ui/confirm.js';
import { iconEl } from '../../ui/icons.js';
import { IMPORT_ICON } from '../../ui/icons.js';
import {
  addTile, detailFooter, detailHead, detailPane, draftKind, itemCard, listDetailPage, listSection,
  notice, pageList, replaceVaultPage, sentDrafts, setPageFocus,
} from '../../ui/page.js';
import { fieldEditKey, fieldEdits, renderEditField, renderPatternField } from '../../ui/field.js';
import { accSeg } from '../../ui/segmented.js';
import { AI_ANALYSIS_OFF, aiPickerWithHint } from './models.js';

// The reading step's key in a watch's `models` (the host's `READ_STEP`).
export const WATCH_READ_STEP = 'read';

// The watch types (`type` in the watch file): how they show, what a new one
// suggests, how a file is read (`read`, always the first step). A new
// watch suggests the user's pictures, on Linux under `~`.
const WATCH_TYPES = [
  {
    type: 'images', command: name => t('watch.import_scans', { name }), also: t('watch.scans_also'),
    name: t('watch.type.images'), hint: 'OCR', kind: t('watch.kind.images'), icon: '#i-image',
    files: t('watch.files.images'),
    folder: navigator.platform.startsWith('Win') ? '%USERPROFILE%\\Pictures' : '~/Pictures',
    patterns: ['*.jpg', '*.jpeg', '*.png', '*.webp'],
    read: {
      name: t('watch.read.image_ocr'), ocr: true, collect: true,
      note: t('watch.read.images_note'),
    },
  },
];

let selectedWatch = null; // a watch id, 'new:<type>' for a new watch's pane, or null
const watchDrafts = sentDrafts(); // watch id -> the changes sent, shown until done
let watchError = null; // { watch, message } from the last failed op
const knownWatches = new Set(); // the ids of the watches the page knows

export function watchType(type) {
  return WATCH_TYPES.find(entry => entry.type === type);
}

// The watch's settings, with what was sent but isn't confirmed yet.
export function watchSettings(id) {
  return { ...watches.get(id), ...watchDrafts.get(id) };
}

registerActions(() => [...watches.values()].filter(watch => watch.enabled && watch.folder).map(watch => {
  const type = watchType(watch.type);
  return {
    id: `import.${watch.id}`, group: 'get', title: type.command(watch.name), icon: IMPORT_ICON,
    also: type.also, page: true,
    run: () => runAction('watch.import', watch.id),
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
  if (watches.size) list.append(listSection(t('watch.section.watches')));
  for (const watch of watches.values()) list.append(renderWatchCard(watch));
  list.append(listSection(t('watch.section.add')), renderWatchAdd());
  return list;
}

function renderWatchCard(watch) {
  const type = watchType(watch.type);
  const settings = watchSettings(watch.id);
  const patterns = settings.patterns.length ? settings.patterns.join(' ') : t('watch.no_file_types');
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
    label: t('watch.add_watch'),
    hint: t('watch.add_watch_hint'),
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
    t('watch.none_yet')));
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

function renderNewWatchPane(type) {
  const id = `new:${type.type}`;
  const pane = detailPane();
  pane.append(
    detailHead(t('watch.new_head', { kind: type.kind }), { icon: type.icon }),
    accEl('div', 'pane-title', type.name),
    accEl('div', 'page-text',
      t('watch.new_note', { patterns: type.patterns.join(' '), files: type.files })));
  const part = accEl('div', 'watch__part');
  const [grid, row] = watchFormGrid();
  row(t('watch.folder'), ...renderWatchFolder(id, null, type.folder));
  part.append(accEl('div', 'section-head', t('watch.folder')), grid);
  pane.append(part, accEl('div', 'page-text', t('watch.set_once_saved')));
  return pane;
}

function renderWatchPane(watch) {
  const id = watch.id;
  const type = watchType(watch.type);
  const settings = watchSettings(id);
  const pane = detailPane();

  const importButton = accEl('button', 'btn');
  importButton.type = 'button';
  importButton.title = t('watch.import_files');
  importButton.append(iconEl(IMPORT_ICON), t('watch.import'));
  importButton.addEventListener('click', () => runAction('watch.import', id));
  pane.append(detailHead(t('watch.head', { kind: type.kind, date: pageDate(watch.added) }),
    { icon: type.icon, button: importButton }));

  const [name, nameNote] = renderEditField(id, 'name', {
    saved: settings.name,
    value: settings.name,
    textClass: 'pane-title edit-field__text',
    placeholder: t('watch.name_empty'),
    editTitle: t('panel.rename'),
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

  const remove = accEl('button', 'btn btn--ghost', t('watch.remove_dots'));
  remove.type = 'button';
  armedButton(remove, t('watch.remove_again'), () => watchRemove(id), `remove_watch ${id}`);
  pane.append(detailFooter(t('watch.remove_note'), remove));
  return pane;
}

// Where to look and which files to take, and whether the watch is a source.
function renderWatchFolderPart(id, settings) {
  const part = accEl('div', 'watch__part');
  const [grid, row] = watchFormGrid();
  row(t('watch.folder'), ...renderWatchFolder(id, settings.folder));
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
    ...(type.read.ocr ? watchModel(id, settings, WATCH_READ_STEP, 'OCR') : {}),
    options: type.read.collect ? renderWatchCollect(id, settings) : null,
  }), renderWatchAnalysis(id, settings), renderWatchKeepOriginal(id, settings));
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
function renderWatchAnalysis(id, settings) {
  const analysis = { ...AI_ANALYSIS_OFF, ...settings.analysis };
  const on = analysis.level !== 'off';
  const set = next => watchSet(id, { analysis: next });
  const step = {
    name: t('watch.step.analysis'), label: t('watch.label.analyse'), on,
    onToggle: () => set({ ...analysis, level: on ? 'off' : 'lite' }),
  };
  if (!on) {
    return renderWatchStep({
      ...step, note: t('watch.analysis_off.images'),
    });
  }
  const { picker, hint } = aiPickerWithHint(analysis.model,
    model => set({ ...analysis, model }), t('feature.scan_analysis'));
  const options = accEl('dl', 'props step__opts');
  const value = accEl('dd');
  value.append(accSeg([['lite', t('watch.level.lite')], ['medium', t('watch.level.medium')]], analysis.level,
    level => set({ ...analysis, level })));
  options.append(accEl('dt', 'label', t('watch.level')), value);
  const note = analysis.level === 'medium' ? t('watch.analysis_note_medium') : t('watch.analysis_note_lite');
  return renderWatchStep({ ...step, note, where: picker, hint, options });
}

// Whether a document of one image keeps the image beside it
// (`keep_original`).
function renderWatchKeepOriginal(id, settings) {
  const on = !!settings.keep_original;
  return renderWatchStep({
    name: t('watch.step.keep'), label: t('watch.label.keep'), on,
    onToggle: () => watchSet(id, { keep_original: !on }),
    note: on
      ? t('watch.keep_on.images')
      : t('watch.keep_off.images'),
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

// Re-render while visible when the op, the vault, the watches, or the AI
// accounts (the reading step's model) change.
store.subscribe(
  [state => state.vault_op, state => state.vault_path, state => state.watches_rev, state => state.accounts_rev],
  () => scheduleRenderFolders());
