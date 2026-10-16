// The documents' content in the editors (saving.md): the content cache,
// the dirty state, the saves and autosave, and the conflicts the conflict
// dialog asks about (features/conflict.js). The editors are the panel's
// cards (features/panel/panel.js); they register their roots in
// `editorRoots`.

import { oncePerFrame } from '../core/dom.js';
import { rpcCall } from '../core/rpc.js';
import { createCache } from '../core/cache.js';
import { serializeMarkdown } from '../editor/markdown.js';
import { store } from './state.js';
import { showError } from './state.js';
import { commitExistingSelection } from './entities.js';

const docContent = createCache();

// Save path -- see saving.md. The editor's DOM is serialized to
// markdown here (`serializeMarkdown`, markdown.js); `save_doc_content`
// carries it, and the host answers with the saved source and its
// re-rendered HTML.
//
// Dirty state is content-based (saving.md#dirty-state): `baseline` holds, per
// document, the markdown the editor's content last matched on disk -- the
// serialization of the DOM as loaded, never the host's `source`, so a
// non-canonical file isn't rewritten just for being opened -- and a
// document is dirty exactly when its current serialization differs.
// `editSeq` is only the stale-reply guard (saving.md#staleness).
//
// A save the host refused because the file changed outside the app
// (`{conflict}`, saving.md#held-saves) puts the document in `conflicted`
// until the user answers in the conflict dialog: no save of it is sent, and its
// latest markdown is kept there for Overwrite, even once its card is gone.
// Reload marks it `reloading` until the file's content arrives.
const editSeq = new Map(); // doc_id -> generation of the latest edit
const baseline = new Map(); // doc_id -> markdown last known to match the file
const sentMarkdown = new Map(); // doc_id -> markdown of the latest save sent
const conflicted = new Map(); // doc_id -> markdown to overwrite the file with
const reloading = new Set(); // doc_ids whose file is being read again

export function isDirty(doc_id) {
  return store.getState().dirty_docs.has(doc_id);
}

// The store's `dirty_docs` changes only when `doc_id`'s state flips; the
// dots in the table's rows and on the cards follow it (app.js).
function setDirty(doc_id, dirty) {
  const { dirty_docs } = store.getState();
  if (dirty_docs.has(doc_id) === dirty) return;
  const next = new Set(dirty_docs);
  if (dirty) next.add(doc_id); else next.delete(doc_id);
  store.setState({ dirty_docs: next });
}

// A registry of "editor root elements currently loaded", keyed by doc_id,
// so refreshDirty/onEdited/trySave can find the right DOM to serialize
// (the panel registers/unregisters its own controllers' roots here).
export const editorRoots = new Map(); // doc_id -> root element

// Whether `doc_id` is loaded into an editor that is in edit mode.
function isEditing(doc_id) {
  return editorRoots.get(doc_id)?.getAttribute('contenteditable') === 'true';
}

// Show `doc_id`'s content in `editor` (on `root`) whenever it arrives, and
// load it. The baseline is the DOM as loaded (saving.md#dirty-state).
// Returns the unsubscribe function.
export function bindDocContent(doc_id, editor, root) {
  const unsubscribe = docContent.subscribe(doc_id, ({ html }) => {
    editor.setHTML(html);
    baseline.set(doc_id, serializeMarkdown(root));
    refreshDirty(doc_id);
  });
  docContent.request(doc_id, () => loadDocContent(doc_id));
  return unsubscribe;
}

// Load `doc_id`'s content, once the host has looked at its file: the
// content as it is then, or -- the file gone -- the panel without it, or
// the content with the conflict over it (the app's version is shown, so
// Use ours can write it back). A failed load can be asked again.
function loadDocContent(doc_id) {
  rpcCall('load_doc_content', { id: doc_id }).then(({ source, html, gone, conflict }) => {
    if (gone) {
      docContent.update(doc_id, { source: '', html: '' });
      commitExistingSelection(store.getState());
      return;
    }
    onDocContent(doc_id, source, html);
    if (conflict) onDocConflict(doc_id, conflict.changed);
  }, e => {
    docContent.failed(doc_id);
    throw e;
  });
}

// Re-derive a loaded document's dirty state from its DOM. Docs with no
// registered root (not currently loaded into any editor) keep whatever
// state their last save reply left.
function refreshDirty(doc_id) {
  const root = editorRoots.get(doc_id);
  if (!root) return;
  setDirty(doc_id, serializeMarkdown(root) !== (baseline.get(doc_id) ?? ''));
}

// One serialization per edited document and animation frame no matter how
// many edit events a keystroke produces (same shape as the editor's own
// toolbar refresh).
const pendingDirty = new Set();

const scheduleRefreshDirty = oncePerFrame(() => {
  for (const doc_id of pendingDirty) refreshDirty(doc_id);
  pendingDirty.clear();
});

// For autosave (below): per document, when it was last edited, and when its
// first edit not autosaved yet was made.
const editedAt = new Map(); // doc_id -> time of its latest edit
const unsavedSince = new Map(); // doc_id -> time of its first unsaved edit

export function onEdited(doc_id) {
  if (!doc_id) return;
  editSeq.set(doc_id, (editSeq.get(doc_id) ?? 0) + 1);
  pendingDirty.add(doc_id);
  scheduleRefreshDirty();
  const now = Date.now();
  editedAt.set(doc_id, now);
  if (!unsavedSince.has(doc_id)) unsavedSince.set(doc_id, now);
}

export function trySave(doc_id) {
  const root = editorRoots.get(doc_id);
  if (!doc_id || !root) return;
  const markdown = serializeMarkdown(root);
  if (conflicted.has(doc_id)) {
    conflicted.set(doc_id, markdown); // waits for Reload or Overwrite
    return;
  }
  if (markdown === (baseline.get(doc_id) ?? '')) {
    setDirty(doc_id, false);
    return;
  }
  sentMarkdown.set(doc_id, markdown);
  saveDocContent(doc_id, markdown, false);
}

// The saves sent and not answered yet; closing the app waits for them
// (features/close.js).
export const savesInFlight = new Set();

// Send `markdown` as `doc_id`'s content; `overwrite` writes it over a file
// changed outside the app (Use ours). The edit generation it is sent at
// stays with the call, so a reply overtaken by newer edits is known as
// stale (saving.md#staleness). A failure shows in the status bar, in the
// host's words.
function saveDocContent(doc_id, markdown, overwrite) {
  const seq = editSeq.get(doc_id) ?? 0;
  const saved = rpcCall('save_doc_content', { id: doc_id, markdown, overwrite }).then(({ source, html, conflict }) => {
    if (conflict) onDocConflict(doc_id, conflict.changed);
    else onDocSaved(doc_id, seq, source, html);
  }, e => {
    if (e.code !== 'cancelled') showError(e.message);
  }).finally(() => savesInFlight.delete(saved));
  savesInFlight.add(saved);
}

// The host refused a save of `doc_id`, or found its file changed after the
// app's own save: it changed outside the app at `changed` (empty: it was
// deleted). It stays dirty, and the conflict dialog asks what to do.
function onDocConflict(doc_id, changed) {
  const root = editorRoots.get(doc_id);
  const markdown = root ? serializeMarkdown(root) : sentMarkdown.get(doc_id);
  conflicted.set(doc_id, markdown);
  setDirty(doc_id, true);
  showConflict(doc_id, markdown !== undefined, changed);
}

// Overwrite: save `doc_id` over its changed file -- the editor's content
// if it is loaded, else what was kept when the save was refused. The
// conflict ends with the save's answer (`onDocSaved`).
export function overwriteDocContent(doc_id) {
  const root = editorRoots.get(doc_id);
  const markdown = root ? serializeMarkdown(root) : conflicted.get(doc_id);
  if (markdown === undefined) return;
  saveDocContent(doc_id, markdown, true);
}

// Reload (`reloadEntity`, model/structure.js): `doc_id`'s file is read
// again, and its saves are held back until the content arrives
// (`onDocContent`).
export function startReload(doc_id) {
  reloading.add(doc_id);
}

// The conflict over `doc_id` is answered (or the entity is gone).
function endConflict(doc_id) {
  conflicted.delete(doc_id);
  reloading.delete(doc_id);
  clearConflict(doc_id);
}

// Autosave (saving.md#triggers): a flat poll over the documents edited
// since their last autosave. One is saved once its typing has paused for a
// second, and at the latest 4 seconds after its first unsaved edit while the
// typing goes on: the file is never far behind, which is all Windows ending
// the session can count on (saving.md#session-end). A document no longer
// in edit mode was saved by leaving it. `trySave` only posts when the
// markdown changed.
const SAVE_PAUSE_MS = 1000;
const SAVE_LATEST_MS = 4000;
setInterval(() => {
  const now = Date.now();
  for (const [doc_id, since] of unsavedSince) {
    if (!isEditing(doc_id)) {
      unsavedSince.delete(doc_id);
      continue;
    }
    const paused = now - editedAt.get(doc_id) >= SAVE_PAUSE_MS;
    if (!paused && now - since < SAVE_LATEST_MS) continue;
    unsavedSince.delete(doc_id);
    trySave(doc_id);
  }
}, 250);

// A save of `doc_id`, sent at edit generation `seq`, went through: the file
// holds `source` now.
function onDocSaved(doc_id, seq, source, html) {
  endConflict(doc_id); // an Overwrite went through
  const current = seq === (editSeq.get(doc_id) ?? 0);
  // `source` is what the file now holds, stale reply or not. Only a loaded
  // document has a DOM to compare against it.
  baseline.set(doc_id, source);
  const root = editorRoots.get(doc_id);
  if (root) refreshDirty(doc_id);
  else if (current) setDirty(doc_id, false);
  // A stale reply's html is never shown (saving.md#staleness), and a live
  // document is never replaced while being edited (saving.md#live-document): both this and the
  // cache subscription in `bindDocContent` go through a full `setHTML()`.
  if (!current) return;
  if (isEditing(doc_id)) return;
  docContent.update(doc_id, { source, html });
}

// ----------------------------------------------------------------------------
// `onDocContent` is part of the document lifecycle -- see saving.md#messages.
// `source` is the markdown body, `html` its editor rendering
export function onDocContent(doc_id, source, html) {
  // The file's content after a Reload: the unsaved changes are dropped,
  // and an in-flight reply is stale (saving.md#held-saves).
  if (reloading.has(doc_id)) {
    endConflict(doc_id);
    editSeq.set(doc_id, (editSeq.get(doc_id) ?? 0) + 1);
    setDirty(doc_id, false);
  }
  docContent.update(doc_id, { source, html });
}

// ----------------------------------------------------------------------------
// `id`'s file changed outside the app at `changed` (ISO 8601; empty: it
// was deleted). `content`: a save of its body was refused, so Use ours is
// offered; otherwise a structural edit was. A conflict already waiting is
// updated in place and stays a content conflict.
export function showConflict(id, content, changed) {
  const { conflicts } = store.getState();
  const waiting = conflicts.find(c => c.id === id);
  const conflict = { id, content: content || (waiting?.content ?? false), changed };
  store.setState({
    conflicts: waiting
      ? conflicts.map(c => (c === waiting ? conflict : c))
      : [...conflicts, conflict],
  });
}

function clearConflict(id) {
  const { conflicts } = store.getState();
  if (conflicts.some(c => c.id === id)) {
    store.setState({ conflicts: conflicts.filter(c => c.id !== id) });
  }
}

// A vault switch: the old vault's files are no longer the question.
function clearConflicts() {
  store.setState({ conflicts: [] });
}

// ----------------------------------------------------------------------------
// `id` left the vault (`entity_deleted`, model/events.js): its edits and
// its conflict are forgotten.
export function forgetDocument(id) {
  editSeq.delete(id);
  baseline.delete(id);
  setDirty(id, false);
  endConflict(id);
  sentMarkdown.delete(id);
}

// Another vault opens (`forgetVault`, model/events.js). No editor of the
// old vault saves any more: its edits were saved before the switch
// (`switchVault`, features/workspace/vault.js), and the open vault is another one now. The
// cards torn down after this find no root to
// save (`trySave`). An unanswered conflict was about the old vault's files.
export function forgetDocuments() {
  editorRoots.clear();
  for (const map of [editSeq, baseline, sentMarkdown, editedAt, unsavedSince, conflicted]) map.clear();
  for (const set of [pendingDirty, reloading]) set.clear();
  store.setState({ dirty_docs: new Set() });
  clearConflicts();
  docContent.clear();
}
