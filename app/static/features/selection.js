// The selection: what the panel shows -- an entity, a
// thread -- all in the store; every feature selects through it.

import { setAria } from '../core/dom.js';
import { savePrefs, store } from '../model/state.js';
import { entities, threadOf, threads } from '../model/entities.js';

// Selects an entity, in the thread `thread_id` if it is one of its members
// (its row under that thread in the Documents lens). Otherwise the shown
// thread is kept if the entity is one of its members (an entity may be in
// several threads), or it becomes the entity's first thread, or none.
export function selectDocument(doc_id, thread_id) {
  const selection = selectionOf(doc_id, thread_id);
  openPanel();
  store.setState(selection);
}

// The selection of the entity `selected_entity`, see `selectDocument`.
function selectionOf(selected_entity, thread_id) {
  const current = thread_id ?? store.getState().selected_thread;
  const threadsOfDoc = entities.get(selected_entity).threads ?? [];
  const selected_thread = threadsOfDoc.includes(current) ? current : (threadOf(selected_entity) ?? null);
  return { selected_entity, selected_thread };
}

// Mark what shows the selected entity in `containerEl` -- its elements
// with that `data-id` -- as `aria-selected`: now, whenever the selection
// changes, and through the function returned, which a render calls once it
// has drawn.
export function markSelection(containerEl) {
  const mark = () => {
    const { selected_entity } = store.getState();
    for (const el of containerEl.querySelectorAll('[data-id]')) {
      setAria(el, 'aria-selected', el.dataset.id === selected_entity);
    }
  };
  store.subscribe(state => state.selected_entity, mark);
  return mark;
}

// Select whatever `id` names: a thread in the panel's thread view. A
// thread's row.
export function selectById(id) {
  openPanel();
  store.setState({ selected_entity: null, selected_thread: id });
}

// The selection comes back on the next start (app.js, after the lens): it
// is remembered (the `entity` and `thread` preferences), and once the
// vault's scan has ended it is selected again, if it is still in the vault
// and nothing else was selected meanwhile; `reveal(id)` then shows it in
// the Documents lens. Until then nothing is remembered, so closing the app
// during the scan keeps it. A vault switch clears the selection, so
// another vault starts with none.
export function restoreSelection(reveal) {
  const { entity, thread } = store.getState().prefs;
  let restored = false;
  const remember = ([selected_entity, selected_thread]) => {
    if (!restored) return;
    const now = { entity: selected_entity ?? '', thread: selected_thread ?? '' };
    const { prefs } = store.getState();
    if ((prefs.entity ?? '') !== now.entity || (prefs.thread ?? '') !== now.thread) savePrefs(now);
  };
  store.subscribe([state => state.selected_entity, state => state.selected_thread], remember);
  store.subscribe(state => state.vault_state, vault_state => {
    if (restored || vault_state !== 'open') return;
    restored = true;
    const { selected_entity, selected_thread } = store.getState();
    if (selected_entity || selected_thread) {
      remember([selected_entity, selected_thread]); // selected meanwhile
    } else if (entities.has(entity)) {
      selectDocument(entity, thread || null);
      reveal(entity);
    } else if (threads.has(thread)) {
      selectById(thread);
      reveal(thread);
    }
  });
}

// Open the panel if it is closed; the layout follows `panel_size`
// (features/layout.js).
function openPanel() {
  if (store.getState().panel_size === 'closed') store.setState({ panel_size: 'normal' });
}
