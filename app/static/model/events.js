// The host's events into the model (api.md#events): the entities as they
// are on disk, and the vault being opened. Each handler writes the Maps
// (model/entities.js) and bumps the store's counters; the renders follow
// the counters (app.js), the model calls none of them. Only this module
// writes the Maps.

import { rpcOn } from '../core/rpc.js';
import { parseDate } from '../core/format.js';
import { store } from './state.js';
import { entities } from './entities.js';
import { projects } from './entities.js';

// ----------------------------------------------------------------------------
function onProject(id, meta) {
  const { title: name, starred, archived } = meta;
  projects.set(id, { id, name, starred, archived });
  store.setState({ projects_rev: store.getState().projects_rev + 1 });
}

// ----------------------------------------------------------------------------
// The entity events (api.md#events): an entity as it is on disk
// now, one shape for every kind.
rpcOn('entity', meta => {
  if (meta.kind === 'project') onProject(meta.id, meta);
  else if (meta.kind !== 'thread') onDocMeta(meta.id, meta);
});

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
// A vault opens (at the start, or after a switch): the page forgets the one
// shown before, so the scan fills an empty page (vault.md#opening).
function forgetVault() {
  entities.clear();
  projects.clear();

  const { projects_rev, entities_rev } = store.getState();
  store.setState({
    selected_project: null,
    projects_rev: projects_rev + 1, entities_rev: entities_rev + 1,
  });
}

// The vault being opened: its scan starts (the vault's name and file
// count).
rpcOn('vault', ({ state, name, total }) => {
  if (state !== 'scanning') return;
  forgetVault();
  store.setState({ vault_name: name, files_total: total });
});
