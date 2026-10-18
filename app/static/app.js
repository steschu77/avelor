// The page's entry (architecture.md#page): it starts the lenses, wires the
// renders to the model (the lenses, the sidebar, the dirty dots), closes a
// page about the old vault when another opens, and tells the host the page
// listens (`ready`). Nothing imports it.
//
// Its imports run the page's modules (development.md#the-bundled-page): each
// module that acts when it loads -- registers an action, a page, a tab or a
// lens, subscribes to the store, listens to the host or the DOM -- is
// imported here, in the order it acts: the menu's Esc before the vault pages'
// (core/actions.js), the context tabs as the strip shows them
// (features/context.js). A module that only declares names runs when
// another first imports it.

import { rpcCall } from './core/rpc.js';
import { rpcOn } from './core/rpc.js';
import { store } from './model/state.js';
import { isDirty } from './model/documents.js';
import './model/events.js';
import './ui/menu.js';
import { shownVaultPage } from './ui/page.js';
import './ui/statusbar.js';
import { invalidateLenses, restoreLens } from './features/lenses/registry.js';
import { msgControllers } from './features/panel/panel.js';
import './features/dashboard/dashboard.js';
import './features/close.js';
import './features/jobs.js';
import './features/header.js';
import './features/conflict.js';
import './features/import/mail.js';
import './features/workspace/accounts.js';
import './features/workspace/trash.js';
import './features/assistant/assistant.js';
import './features/new.js';
import './features/linked.js';
import './features/layout.js';
import { scheduleRenderSidebar } from './features/sidebar.js';
import { initDocumentsLens } from './features/lenses/documents.js';
import { revealInDocuments } from './features/lenses/documents.js';
import { getDocRows } from './features/lenses/documents.js';
import { restoreSelection } from './features/selection.js';
import './features/lenses/timeline.js';
import { initCalendarLens } from './features/lenses/calendar.js';

initDocumentsLens();
initCalendarLens();
restoreLens();
restoreSelection(revealInDocuments);

// The lenses and the sidebar follow the model (model/events.js): the
// entities as they are now, the projects and accounts the
// sidebar lists, the threads and projects the Documents lens groups by
// (`commitThreadChange`), and the projects the Timeline lens's rows name.
store.subscribe(state => state.entities_rev, () => {
  invalidateLenses();
  scheduleRenderSidebar();
});
store.subscribe([state => state.projects_rev, state => state.accounts_rev],
  () => scheduleRenderSidebar());
store.subscribe([state => state.threads_rev, state => state.projects_rev], () => invalidateLenses('documents', 'timeline'));

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
  [state => state.selected_project, state => state.selected_source, state => state.selected_labels],
  () => {
    invalidateLenses();
    scheduleRenderSidebar();
  });

// Another vault opens (`forgetVault`, model/events.js): a page about the
// old one (New, Fetch mail) closes.
rpcOn('vault', ({ state }) => {
  if (state === 'scanning') shownVaultPage()?.close();
});

// The page listens: the host opens the vault of the last session, and
// says whether this build has Google.
rpcCall('ready').then(({ google }) => store.setState({ google }));
