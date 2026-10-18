// The page's store (architecture.md#page-state).

import { $ } from '../core/dom.js';
import { createStore } from '../core/store.js';
import { rpcCall } from '../core/rpc.js';
import { dayStart } from '../core/format.js';

export const store = createStore({
  vault_name: '', // the open vault's folder name and path (the `vault` event)
  vault_path: '',
  vault_state: 'none', // 'none' | 'scanning' | 'open' (the `vault` event)
  google: true, // whether this build has Google (`ready`)
  // the status bar's notices, newest last (ui/statusbar.js): { key, text,
  // level: 'error' }; always replaced
  notices: [],
  // files changed outside the app, oldest first, each asked about in the
  // conflict dialog (features/conflict.js): { id, content, changed }; always replaced
  conflicts: [],
  // the ids of the documents with unsaved edits (`setDirty`,
  // model/documents.js): a Set, always replaced, never mutated
  dirty_docs: new Set(),
  // the page's preferences, strings by key: the host fills them into the
  // page (#prefs, page.rs); `savePrefs` changes them; always replaced
  prefs: JSON.parse($('prefs')?.textContent || '{}'),
  // *_rev: bumped on every change on projects, threads, accounts, entities,
  // the vault's settings
  projects_rev: 0,
  threads_rev: 0,
  accounts_rev: 0,
  entities_rev: 0,
  settings_rev: 0,
  selected_project: null, // project filter: one, NO_PROJECT (in none) or null (all), see matchesFilter
  selected_source: null, // source filter: one or none (all), keyed as `sourceKeyOf`
  vault_op: null, // the Workspace op shown in the expanded vault area ("accounts", "mail", "new", "trash")
  vault_expanded: false, // the vault area is open
  // the panel's width: 'closed', 'normal', 'chat' (the normal panel with the
  // context column beside it, `toggleCardChat`) or 'expanded' (full width)
  panel_size: 'normal',
  selected_labels: [], // label (tag) filter: a doc must carry all of them; always replaced, never mutated
  selected_entity: null,
  selected_thread: null, // the thread shown in the panel, see renderPanel
  selected_day: dayStart(new Date()), // the day the calendar's panel shows (features/lenses/calendar.js)
  selected_lens: 'dashboard', // the lens shown; the one shown last comes back on start (`restoreLens`)
  files_total: 0,
  // the Documents lens's order; `kind` is the last tiebreak, no column sorts by it
  sort_state: [
    { key: 'date', dir: 'desc' },
    { key: 'title', dir: 'asc' },
    { key: 'kind', dir: 'asc' },
  ],
});

// ----------------------------------------------------------------------------
// Merge `patch` into the page's preferences and send them all to the host
// (`set_prefs`), which stores them as given and fills them into the page on
// the next start. Values are strings.
export function savePrefs(patch) {
  const prefs = { ...store.getState().prefs, ...patch };
  store.setState({ prefs });
  rpcCall('set_prefs', { prefs });
}

// ----------------------------------------------------------------------------
// Show `notice`, replacing the one with the same key.
function setNotice(notice) {
  const notices = store.getState().notices.filter(n => n.key !== notice.key);
  store.setState({ notices: [...notices, notice] });
}

// An error the user should see: a failed save, a link to an entity not in
// the vault. It stays until it is dismissed.
export function showError(text) {
  setNotice({ key: text, text, level: 'error' });
}
