// Closing the app (saving.md#closing-the-app):
// before the window goes, the host asks (`close {id}`), and the page tells
// the host it is ready (`close_ready`).

import { rpcCall, rpcOn } from '../core/rpc.js';

rpcOn('close', ({ id }) => {
  rpcCall('close_ready', { id, ready: true });
});
