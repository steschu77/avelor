// The vault panel's box, Filter by words (ui.md#words): the lenses keep
// what holds every word, as the host finds it (`search`, on the channel
// `search`), and the rest drops out (`matchesFilter`). The answer comes a
// little after the typing; until then the lenses keep the last one. It is
// asked again while the entities change, so an edit's new words count.

import { $ } from '../core/dom.js';
import { registerAction } from '../core/actions.js';
import { rpcCall } from '../core/rpc.js';
import { NO_WORDS, showError, store } from '../model/state.js';
import { selectFilter } from './selection.js';

// At most one search this often, with the words as they are then.
const SEARCH_EVERY_MS = 120;

const boxEl = $('words-box');
const inputEl = $('words-input');

let searchTimer = null;

inputEl.addEventListener('input', () => {
  const words = inputEl.value;
  selectFilter(words.trim() ? { words } : { words, word_filter: null });
});

// Esc empties the box, a second one leaves it: the box's own, not the open
// vault page's or the panel's (core/actions.js skips a key handled).
inputEl.addEventListener('keydown', ev => {
  if (ev.key !== 'Escape') return;
  ev.preventDefault();
  if (inputEl.value) selectFilter({ ...NO_WORDS });
  else inputEl.blur();
});

$('words-clear').addEventListener('click', () => {
  selectFilter({ ...NO_WORDS });
  inputEl.focus();
});

// Ctrl+S goes to the box, but not from an editor or a field, where it is
// the habit of saving.
registerAction({
  id: 'words.focus',
  title: 'Filter by words',
  shortcut: 'Ctrl+S',
  when: () => {
    const focused = document.activeElement;
    return !focused?.isContentEditable && !focused?.matches('input, textarea, select');
  },
  run: () => {
    inputEl.focus();
    inputEl.select();
  },
});

// The box shows the words, marked while they filter.
store.subscribe(state => state.words, words => {
  if (inputEl.value !== words) inputEl.value = words;
  boxEl.classList.toggle('is-on', Boolean(words.trim()));
  searchSoon();
});
store.subscribe([state => state.entities_rev, state => state.saved_rev], searchSoon);

function searchSoon() {
  if (!store.getState().words.trim()) {
    clearTimeout(searchTimer);
    searchTimer = null;
    return;
  }
  searchTimer ??= setTimeout(search, SEARCH_EVERY_MS);
}

// Ask for the words as they are; the answer counts while they still are.
function search() {
  searchTimer = null;
  const { words: text, vault_state } = store.getState();
  if (!text.trim() || vault_state === 'none') return;
  rpcCall('search', { text }, { channel: 'search' }).then(({ terms, hits }) => {
    if (store.getState().words !== text) return;
    const byId = new Map(hits.map(({ id, excerpt, where }) => [id, { excerpt, where }]));
    store.setState({ word_filter: { text, terms, hits: byId } });
  }, e => {
    if (e.code !== 'superseded' && e.code !== 'cancelled') showError(e.message);
  });
}
