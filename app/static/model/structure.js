// The structural edits (panel.md#saving-structure, api.md#structure):
// projects, titles, labels, links and reloads, each a host round trip that
// ends in a published selection (`commitExistingSelection`,
// model/entities.js); joining and leaving projects, labels and links as
// requests of their own (`joinProjectLocal`, ..., `setLinkLocal`). The
// panel's buttons (features/panel/panel.js), the sidebar and the project
// header (features/sidebar.js, features/header.js) send them.

import { rpcCall } from '../core/rpc.js';
import { store } from './state.js';
import { commitExistingSelection } from './entities.js';
import { entities } from './entities.js';
import { showConflict } from './documents.js';
import { isDirty, startReload, trySave } from './documents.js';

// ----------------------------------------------------------------------------
// Structural edits are applied and saved by the host (panel.md#saving-structure),
// never here: each is a typed command (`structureCall`). The host pushes
// what it wrote -- `entity` for every written entity -- before it
// answers, so once the answer arrives, `entities` and `threads` already
// show what is on disk, success or not. `select(id)` is the selection
// afterwards, `id` the answer's (the id a reloaded file has now); after a
// failure or a conflict the selection stays. Each is applied minus
// anything that no longer exists; publishing it rebuilds the panel's cards
// (`threads_rev`). Without `select`, nothing is published: an entity's own
// fields, which the cards follow from its push (features/panel/panel.js's
// `refreshCardChips`). A conflict (a file changed outside the app, nothing
// written) opens the conflict dialog; unsaved edits of it are sent first,
// so their own conflict offers Use ours too. A failure shows in the status
// bar, in the host's words (core/rpc.js).
export function structureCall(cmd, args, select = null) {
  rpcCall(cmd, args).then(reply => {
    const conflict = reply?.conflict;
    if (conflict) {
      showConflict(conflict.id, false, conflict.changed);
      if (isDirty(conflict.id)) trySave(conflict.id);
    }
    if (select) commitExistingSelection(conflict ? store.getState() : select(reply?.id ?? null));
  }, e => {
    if (select) commitExistingSelection(store.getState());
    throw e;
  });
}

// ----------------------------------------------------------------------------
// Reload: read `id`'s file again, dropping the unsaved changes. It ends
// like a structural edit (`structureCall`), so the panel is rebuilt --
// without the entity if its file is gone, with its new id if that was
// changed by hand. Saves stay held back until the content arrives
// (`onDocContent`), so none of the old content can follow the reload and
// overwrite the file after all.
export function reloadEntity(id) {
  startReload(id);
  const { selected_entity, selected_thread } = store.getState();
  structureCall('reload_entity', { id }, now => ({
    selected_entity: selected_entity === id ? now : selected_entity,
    selected_thread,
  }));
}

// ----------------------------------------------------------------------------
// Adds `doc_id` to the project `project_id`; the selection stays.
export function joinProjectLocal(doc_id, project_id) {
  const doc = entities.get(doc_id);
  if (!doc || doc.projects?.includes(project_id)) return;

  structureCall('join_project', { id: doc_id, project: project_id }, () => store.getState());
}

// ----------------------------------------------------------------------------
// Removes `doc_id` from the project `project_id`; the selection stays.
export function leaveProjectLocal(doc_id, project_id) {
  const doc = entities.get(doc_id);
  if (!doc || !doc.projects?.includes(project_id)) return;

  structureCall('leave_project', { id: doc_id, project: project_id }, () => store.getState());
}

// ----------------------------------------------------------------------------
// Set `doc_id`'s labels to `tags`; the host writes them and pushes the
// entity back. The selection stays.
export function setTagsLocal(doc_id, tags) {
  structureCall('set_tags', { id: doc_id, tags }, () => store.getState());
}

// Link `id` to `to`, null to remove its link; the host writes it and pushes
// the entity back. The selection stays.
export function setLinkLocal(id, to) {
  structureCall('set_link', { id, to }, () => store.getState());
}
