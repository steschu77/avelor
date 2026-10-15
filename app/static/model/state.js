// The page's store (architecture.md#page-state).

import { $ } from '../core/dom.js';
import { createStore } from '../core/store.js';
import { rpcCall } from '../core/rpc.js';

export const store = createStore({
  vault_name: '', // the open vault's folder name (the `vault` event)
  // the status bar's notices, newest last (ui/statusbar.js): { key, text,
  // level: 'error' }; always replaced
  notices: [],
  // the page's preferences, strings by key: the host fills them into the
  // page (#prefs, page.rs); `savePrefs` changes them; always replaced
  prefs: JSON.parse($('prefs')?.textContent || '{}'),
  // *_rev: bumped on every change on entities
  entities_rev: 0,
  // the panel's width: 'closed', 'normal' or 'expanded' (full width)
  panel_size: 'normal',
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

// An error the user should see: a failed save. It stays until it is
// dismissed.
export function showError(text) {
  setNotice({ key: text, text, level: 'error' });
}
