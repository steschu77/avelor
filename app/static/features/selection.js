// The selection (panel.md#selection): what the panel shows -- an entity, a
// thread -- and the vault panel's filter, all in the store; every feature
// selects through it.

import { setAria } from '../core/dom.js';
import { rpcCall } from '../core/rpc.js';
import { savePrefs, store } from '../model/state.js';
import { showError } from '../model/state.js';
import { entities, threadOf, threads } from '../model/entities.js';
import { projects } from '../model/entities.js';
import { closeVaultArea } from '../ui/page.js';

// Set the vault panel's filter to `patch`: a filter row's click closes
// the open vault page, even if the filter stays the same
// (features/sidebar.js).
export function selectFilter(patch) {
  closeVaultArea();
  store.setState(patch);
}

// Selects an entity, in the thread `thread_id` if it is one of its members
// (its row under that thread in the Documents lens). Otherwise the shown
// thread is kept if the entity is one of its members (an entity may be in
// several threads; clicking a card must not jump to another one), or it
// becomes the entity's first thread, or none -- renderPanel then shows it
// standalone.
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

// Open a link clicked in a preview (an editor's `onLinkClick`): a link
// into the vault, `id:<uuid>` (vault.md#links-in-content), selects its
// target; a web, mail or phone link opens in its default app, through the
// host (`open_url`).
export function openLink(href) {
  const vault = /^id:(.+)$/i.exec(href?.trim() ?? '');
  if (vault) {
    selectById(vault[1]);
    return;
  }
  if (!/^(https?:[/][/]|mailto:|tel:)/i.test(href ?? '')) return;
  rpcCall('open_url', { url: href }).catch(e => {
    if (e.code !== 'cancelled') showError(e.message);
  });
}

// Select whatever `id` names: an entity in the panel, a thread in the
// panel's thread view, a project as the filter. A link's click
// (`openLink`, the link chips), a thread's row.
export function selectById(id) {
  if (entities.has(id)) {
    selectDocument(id);
  } else if (threads.has(id)) {
    openPanel();
    store.setState({ selected_entity: null, selected_thread: id });
  } else if (projects.has(id)) {
    selectFilter({ selected_project: id });
  } else {
    showError(`The linked entity ${id} is not in the vault.`);
  }
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

// ----------------------------------------------------------------------------
// Which panel shows what, derived from store state alone:
// - 'thread': `selected_thread` exists and `selected_entity` (if any) is one
//   of its members,
// - 'document': `selected_entity` is a known entity, shown standalone,
// - 'none': nothing selected (or the selection was deleted): the empty
//   preview.
export function panelMode({ selected_entity, selected_thread }) {
  if (selected_thread && threads.has(selected_thread)
    && (!selected_entity || entities.get(selected_entity)?.threads?.includes(selected_thread))) {
    return 'thread';
  }
  if (selected_entity && entities.has(selected_entity)) {
    return 'document';
  }
  return 'none';
}
