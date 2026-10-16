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
import { scheduleRenderSidebar } from './features/sidebar.js';
import { initDocumentsLens, scheduleRenderDocuments } from './features/lenses/documents.js';
import { revealInDocuments } from './features/lenses/documents.js';
import { restoreSelection } from './features/selection.js';

initDocumentsLens();
restoreSelection(revealInDocuments);

// The lenses and the sidebar follow the model (model/events.js): the
// entities as they are now, the projects the sidebar lists.
store.subscribe(state => state.entities_rev, () => {
  scheduleRenderDocuments();
  scheduleRenderSidebar();
});
store.subscribe(state => state.projects_rev, () => {
  scheduleRenderDocuments();
  scheduleRenderSidebar();
});

// The filter changed: re-render the filtered lenses and the sidebar's
// highlights.
store.subscribe(state => state.selected_project, () => {
  scheduleRenderDocuments();
  scheduleRenderSidebar();
});

// The lens's type selection (features/lenses/types.js): a change renders
// the lens again.
store.subscribe(({ prefs }) => prefs.doc_types, scheduleRenderDocuments);

// The page listens.
rpcCall('ready');
