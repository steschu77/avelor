// The documents (saving.md): the conflicts the conflict dialog asks about
// (features/conflict.js).

import { store } from './state.js';

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
}
