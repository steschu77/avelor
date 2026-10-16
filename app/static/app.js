// The page's entry (architecture.md#page): it tells the host the page
// listens (`ready`). Nothing imports it.
//
// Its imports run the page's modules (development.md#the-bundled-page): each
// module that acts when it loads -- subscribes to the store, listens to the
// host or the DOM -- is imported here, in the order it acts. A module that only
// declares names runs when another first imports it.

import { rpcCall } from './core/rpc.js';
import { store } from './model/state.js';
import { isDirty } from './model/documents.js';
import './model/events.js';
import './ui/menu.js';
import './ui/statusbar.js';
import { msgControllers } from './features/panel/panel.js';
import './features/close.js';
import './features/jobs.js';
import './features/header.js';
import './features/conflict.js';
import './features/layout.js';
import { scheduleRenderSidebar } from './features/sidebar.js';
import { initDocumentsLens, scheduleRenderDocuments } from './features/lenses/documents.js';
import { revealInDocuments } from './features/lenses/documents.js';
import { getDocRows } from './features/lenses/documents.js';
import { restoreSelection } from './features/selection.js';

initDocumentsLens();
restoreSelection(revealInDocuments);

// The lenses and the sidebar follow the model (model/events.js): the
// entities as they are now, the projects the sidebar lists, the threads
// and projects the Documents lens groups by (`commitThreadChange`).
store.subscribe(state => state.entities_rev, () => {
  scheduleRenderDocuments();
  scheduleRenderSidebar();
});
store.subscribe(state => state.projects_rev, () => scheduleRenderSidebar());
store.subscribe([state => state.threads_rev, state => state.projects_rev], () => scheduleRenderDocuments());

function updateDirtyIndicator(doc_id) {
  for (const row of getDocRows(doc_id)) row.querySelector('.dirty-dot').hidden = !isDirty(doc_id);
  const controller = msgControllers.get(doc_id);
  if (controller) controller.dirtyDot.hidden = !isDirty(doc_id);
}

// The dots of the documents with unsaved edits, in the table's rows and on
// the cards, follow `dirty_docs` (`setDirty`, model/documents.js); a row or
// a card built later shows its own (`isDirty`).
let dirtyShown = new Set();
store.subscribe(state => state.dirty_docs, dirty_docs => {
  for (const doc_id of dirtyShown) if (!dirty_docs.has(doc_id)) updateDirtyIndicator(doc_id);
  for (const doc_id of dirty_docs) if (!dirtyShown.has(doc_id)) updateDirtyIndicator(doc_id);
  dirtyShown = dirty_docs;
});

// The filter changed: re-render the filtered lenses and the sidebar's
// highlights.
store.subscribe(
  [state => state.selected_project, state => state.selected_labels],
  () => {
    scheduleRenderDocuments();
    scheduleRenderSidebar();
  });

// The lens's type selection (features/lenses/types.js): a change renders
// the lens again.
store.subscribe(({ prefs }) => prefs.doc_types, scheduleRenderDocuments);

// The page listens.
rpcCall('ready');
