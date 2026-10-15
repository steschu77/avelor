// The page's entry (architecture.md#page): it tells the host the page
// listens (`ready`). Nothing imports it.
//
// Its imports run the page's modules (development.md#the-bundled-page): each
// module that acts when it loads -- subscribes to the store, listens to the
// host or the DOM -- is imported here, in the order it acts. A module that only
// declares names runs when another first imports it.

import { rpcCall } from './core/rpc.js';
import { store } from './model/state.js';
import './model/events.js';
import './ui/menu.js';
import './ui/statusbar.js';
import './features/close.js';
import './features/jobs.js';
import './features/layout.js';
import './features/sidebar.js';
import { initDocumentsLens, scheduleRenderDocuments } from './features/lenses/documents.js';

initDocumentsLens();

// The lenses follow the model (model/events.js): the entities as they are
// now.
store.subscribe(state => state.entities_rev, () => scheduleRenderDocuments());

// The page listens.
rpcCall('ready');
