// The documents' content in the editors (saving.md): the content cache,
// and the conflicts the conflict dialog asks about (features/conflict.js).

import { rpcCall } from '../core/rpc.js';
import { createCache } from '../core/cache.js';
import { store } from './state.js';
import { commitExistingSelection } from './entities.js';

const docContent = createCache();

// ----------------------------------------------------------------------------
// Show `doc_id`'s content in `root` whenever it arrives, and load it.
// Returns the unsubscribe function.
export function bindDocContent(doc_id, root) {
  const unsubscribe = docContent.subscribe(doc_id, ({ html }) => {
    root.innerHTML = html;
  });
  docContent.request(doc_id, () => loadDocContent(doc_id));
  return unsubscribe;
}

// Load `doc_id`'s content, once the host has looked at its file: the
// content as it is then, or -- the file gone -- the panel without it, or
// the content with the conflict over it. A failed load can be asked again.
function loadDocContent(doc_id) {
  rpcCall('load_doc_content', { id: doc_id }).then(({ source, html, gone, conflict }) => {
    if (gone) {
      docContent.update(doc_id, { source: '', html: '' });
      commitExistingSelection(store.getState());
      return;
    }
    onDocContent(doc_id, source, html);
    if (conflict) showConflict(doc_id, conflict.changed);
  }, e => {
    docContent.failed(doc_id);
    throw e;
  });
}

// ----------------------------------------------------------------------------
// `onDocContent` is part of the document lifecycle -- see saving.md#messages.
// `source` is the markdown body, `html` its editor rendering
export function onDocContent(doc_id, source, html) {
  docContent.update(doc_id, { source, html });
}

// ----------------------------------------------------------------------------
// `id`'s file changed outside the app at `changed` (ISO 8601; empty: it
// was deleted): a structural edit was refused. A conflict already waiting
// is updated in place.
export function showConflict(id, content, changed) {
  const { conflicts } = store.getState();
  const waiting = conflicts.find(c => c.id === id);
  const conflict = { id, changed };
  store.setState({
    conflicts: waiting
      ? conflicts.map(c => (c === waiting ? conflict : c))
      : [...conflicts, conflict],
  });
}

// The conflict over `doc_id` is answered (or the entity is gone).
export function endConflict(doc_id) {
  const { conflicts } = store.getState();
  if (conflicts.some(c => c.id === doc_id)) {
    store.setState({ conflicts: conflicts.filter(c => c.id !== doc_id) });
  }
}

// ----------------------------------------------------------------------------
// `id` left the vault (`entity_deleted`, model/events.js): its conflict is
// forgotten.
export function forgetDocument(id) {
  endConflict(id);
}

// Another vault opens (`forgetVault`, model/events.js). An unanswered
// conflict was about the old vault's files.
export function forgetDocuments() {
  store.setState({ conflicts: [] });
  docContent.clear();
}
