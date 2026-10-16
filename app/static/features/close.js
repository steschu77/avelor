// Closing the app (saving.md#closing-the-app):
// before the window goes, the host asks (`close {id}`). Every editor leaves
// edit mode -- the card being edited, the project description -- saving as
// it does, and once every save sent is answered the page tells the host
// whether it is ready (`close_ready`). Not while a conflict waits: its
// dialog is open (features/conflict.js), the window stays, and closing again once
// it is answered goes on. A save that failed otherwise doesn't hold the
// close; the status bar said so.

import { rpcCall, rpcOn } from '../core/rpc.js';
import { store } from '../model/state.js';
import { savesInFlight } from '../model/documents.js';
import { leaveEditModes } from './header.js';

rpcOn('close', ({ id }) => {
  leaveEditModes();
  Promise.allSettled([...savesInFlight]).then(() => {
    const ready = store.getState().conflicts.length === 0;
    rpcCall('close_ready', { id, ready });
  });
});
