// The structural edits (panel.md#saving-structure, api.md#structure):
// titles and reloads, each a host round trip. The sidebar and the project
// header (features/sidebar.js, features/header.js) send them.

import { rpcCall } from '../core/rpc.js';
import { endConflict, showConflict } from './documents.js';

// ----------------------------------------------------------------------------
// Structural edits are applied and saved by the host (panel.md#saving-structure),
// never here: each is a typed command (`structureCall`). The host pushes
// what it wrote -- `entity` for every written entity -- before it
// answers, so once the answer arrives, `entities` and `threads` already
// show what is on disk, success or not. A conflict (a file changed outside
// the app, nothing written) opens the conflict dialog. A failure shows in
// the status bar, in the host's words (core/rpc.js).
export function structureCall(cmd, args, select = null) {
  return rpcCall(cmd, args).then(reply => {
    if (reply?.conflict) showConflict(reply.conflict.id, false, reply.conflict.changed);
  });
}

// ----------------------------------------------------------------------------
// Reload: read `id`'s file again. It ends like a structural edit
// (`structureCall`), and with its answer the conflict over `id` ends.
export function reloadEntity(id) {
  structureCall('reload_entity', { id }).then(() => endConflict(id));
}
