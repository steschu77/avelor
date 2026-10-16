// The host's events into the model (api.md#events): the entities as they
// are on disk, those that left the vault, their content sent unasked, the
// vault being opened, and the window active again. Each
// handler writes the Maps (model/entities.js) and the documents' state
// (model/documents.js) and bumps the store's counters; the renders follow
// the counters (app.js), the model calls none of them. Only this module
// writes the Maps.

import { rpcOn } from '../core/rpc.js';
import { parseDate } from '../core/format.js';
import { store } from './state.js';
import { entities } from './entities.js';
import { projects } from './entities.js';
import { threads } from './entities.js';
import { forgetDocument, forgetDocuments } from './documents.js';
import { onDocContent } from './documents.js';
import { onActivated } from './documents.js';

// ----------------------------------------------------------------------------
function onProject(id, meta) {
  const { title: name, path, starred, archived, writable, deletable } = meta;
  projects.set(id, { id, name, path, starred, archived, writable, deletable });
  store.setState({ projects_rev: store.getState().projects_rev + 1 });
}

// ----------------------------------------------------------------------------
// A thread as it is now: its title shows in the cards' threads lines,
// refreshed on `entities_rev`, so one renamed shows without rebuilding the
// cards.
function onThread(thread_id, meta) {
  threads.set(thread_id, { id: thread_id, ...meta });
  store.setState({ entities_rev: store.getState().entities_rev + 1 });
}

// ----------------------------------------------------------------------------
// The entity events (api.md#events): an entity as it is on disk
// now, one shape for every kind; one that left the vault; a body sent
// unasked (`load_doc_content` answers with the same `source` and `html`).
// One new after the scan is one more file; the scan's are counted by its
// start.
rpcOn('entity', meta => {
  const known = entities.has(meta.id) || threads.has(meta.id) || projects.has(meta.id);
  const { vault_state, files_total } = store.getState();
  if (!known && vault_state === 'open') store.setState({ files_total: files_total + 1 });
  if (meta.kind === 'project') onProject(meta.id, meta);
  else if (meta.kind === 'thread') onThread(meta.id, meta);
  else onDocMeta(meta.id, meta);
});
rpcOn('entity_deleted', ({ id }) => onEntityDeleted(id));
rpcOn('content', ({ id, source, html }) => onDocContent(id, source, html));

// ----------------------------------------------------------------------------
function onDocMeta(doc_id, meta) {
  entities.set(doc_id, {
    ...meta,
    doc_id,
    date: parseDate(meta.date),
  });
  store.setState({ entities_rev: store.getState().entities_rev + 1 });
}

// ----------------------------------------------------------------------------
// An edit moved `id` to `.trash` (panel.md#structural-edits), or its file is
// gone: forget it everywhere. Its card goes with the panel rebuild the
// edit's answer triggers (`structureCall`); a deleted project that was
// the filter leaves it, back to all documents.
function onEntityDeleted(id) {
  entities.delete(id);
  threads.delete(id);
  const project = projects.delete(id);
  forgetDocument(id);
  const { entities_rev, files_total, projects_rev, selected_project } = store.getState();
  store.setState({
    entities_rev: entities_rev + 1,
    files_total: Math.max(0, files_total - 1),
    ...(project && { projects_rev: projects_rev + 1 }),
    ...(selected_project === id && { selected_project: null }),
  });
}

// ----------------------------------------------------------------------------
// A vault opens (at the start, or after a switch): the page forgets the one
// shown before, so the scan fills an empty page (vault.md#opening).
function forgetVault() {
  forgetDocuments();

  entities.clear();
  projects.clear();
  threads.clear();

  const { projects_rev, threads_rev, entities_rev } = store.getState();
  store.setState({
    selected_entity: null, selected_thread: null,
    selected_project: null, selected_labels: [],
    projects_rev: projects_rev + 1, threads_rev: threads_rev + 1, entities_rev: entities_rev + 1,
  });
}

// The vault being opened: its scan starts (the vault's name and file
// count) and ends.
rpcOn('vault', ({ state, name, total }) => {
  if (state !== 'scanning') return store.setState({ vault_state: state });
  forgetVault();
  store.setState({ vault_name: name, files_total: total, vault_state: 'scanning' });
});
rpcOn('activated', onActivated);
