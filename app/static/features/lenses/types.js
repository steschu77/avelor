// The type selection of a lens that lists entities (ui.md#documents,
// ui.md#timeline): a bar of type toggles, each with its count in the vault
// panel's filter -- the lens's own filter, which the vault panel's counts
// and the other lenses don't follow. A lens makes its bar with `typeBar`;
// the types shown and the bar's fold are its preferences. The lens head's
// Types button folds the bar of the lens shown.

import { $, accEl } from '../../core/dom.js';
import { plural } from '../../core/format.js';
import { savePrefs, store } from '../../model/state.js';
import { invalidateLenses } from './registry.js';

// The types, each a toggle: an entity's by its kind, a document's notes
// (the genre `note`) apart.
const DOC_TYPES = [
  { id: 'documents', name: 'Documents', one: 'document', many: 'documents' },
  { id: 'mail', name: 'Mail', one: 'mail', many: 'mails' },
  { id: 'notes', name: 'Notes', one: 'note', many: 'notes' },
  { id: 'chats', name: 'Chats', one: 'chat', many: 'chats' },
  { id: 'bank', name: 'Bank', one: 'bank item', many: 'bank items' },
  { id: 'lists', name: 'Lists', one: 'list', many: 'lists' },
  { id: 'records', name: 'Records', one: 'record', many: 'records' },
  { id: 'contacts', name: 'Contacts', one: 'contact', many: 'contacts' },
  { id: 'tasks', name: 'Tasks', one: 'task', many: 'tasks' },
  { id: 'events', name: 'Events', one: 'event', many: 'events' },
];
const KIND_TYPES = {
  message: 'documents', email: 'mail', conversation: 'chats', 'bank-account': 'bank',
  transactions: 'bank', list: 'lists', record: 'records', contact: 'contacts', task: 'tasks',
  event: 'events', calendar: 'events',
};

export function typeOf(doc) {
  if (doc.kind === 'document') return doc.genre === 'note' ? 'notes' : 'documents';
  return KIND_TYPES[doc.kind] ?? 'documents';
}

// "1 chat, 10 bank items and 5 tasks", of the types hidden that the filter
// has (`counts`: type id -> entities).
export function hiddenText(shown, counts) {
  const parts = DOC_TYPES.filter(type => !shown.has(type.id) && counts.get(type.id))
    .map(type => plural(counts.get(type.id), type.one, type.many));
  return parts.length > 1 ? `${parts.slice(0, -1).join(', ')} and ${parts.at(-1)}` : parts[0] ?? '';
}

// ----------------------------------------------------------------------------
// The lens head's Types button, shown under a lens with a bar.
const typesBtn = $('types-btn');
const bars = new Map(); // lens id -> { el, barPref }

function showTypesBtn(selected_lens) {
  const bar = bars.get(selected_lens);
  typesBtn.hidden = !bar;
  if (bar) typesBtn.setAttribute('aria-expanded', String(!bar.el.hidden));
}
store.subscribe(state => state.selected_lens, showTypesBtn);

typesBtn.addEventListener('click', () => {
  const bar = bars.get(store.getState().selected_lens);
  if (bar) savePrefs({ [bar.barPref]: bar.el.hidden ? 'open' : 'closed' });
});

// The bar of the lens `lens` in `el`: the types shown are the preference
// `pref` (their ids joined by commas; an id this page doesn't know is
// dropped; `shownAtFirst` while it is missing), the bar's fold `barPref`
// (`open` / `closed`). A change renders the lens again. `render(shown,
// counts)` sets the toggles; `shown()` the types shown; `show(type_id)` and
// `showAll()` switch types on.
export function typeBar({ lens, el, pref, barPref, shownAtFirst = DOC_TYPES.map(type => type.id) }) {
  function shown() {
    const saved = store.getState().prefs[pref];
    const ids = saved === undefined ? shownAtFirst : saved.split(',');
    return new Set(ids.filter(id => DOC_TYPES.some(type => type.id === id)));
  }

  function save(types) {
    savePrefs({ [pref]: DOC_TYPES.filter(type => types.has(type.id)).map(type => type.id).join(',') });
  }

  // the toggles, built once
  const toggles = new Map(DOC_TYPES.map(type => {
    const btn = accEl('button', 'type-toggle');
    btn.type = 'button';
    const count = accEl('span', 'type-toggle__n');
    btn.append(type.name, count);
    btn.addEventListener('click', () => {
      const types = shown();
      if (!types.delete(type.id)) types.add(type.id);
      save(types);
    });
    el.append(btn);
    return [type.id, btn];
  }));

  function render(types, counts) {
    for (const type of DOC_TYPES) {
      const btn = toggles.get(type.id);
      const on = types.has(type.id);
      btn.setAttribute('aria-pressed', String(on));
      btn.classList.toggle('is-empty', !counts.get(type.id));
      btn.title = `${on ? 'Hide' : 'Show'} ${type.many}`;
      btn.lastChild.textContent = counts.get(type.id) ?? 0;
    }
  }

  bars.set(lens, { el, barPref });
  store.subscribe(({ prefs }) => prefs[barPref], value => {
    el.hidden = value === 'closed';
    showTypesBtn(store.getState().selected_lens);
  });
  store.subscribe(({ prefs }) => prefs[pref], () => invalidateLenses(lens));

  return {
    shown,
    show(type_id) {
      const types = shown();
      if (!types.has(type_id)) save(types.add(type_id));
    },
    showAll: () => save(new Set(DOC_TYPES.map(type => type.id))),
    render,
  };
}
