// The panel's context column, expanded or half-open (index.html,
// .panel__context, `panel_size`): tabs about what the panel shows --
// Conversation (features/assistant/assistant.js), Task (features/new.js) and
// Linked (features/linked.js). One tab shows at a time, the one picked last
// while the app runs. A tab registers itself, in the order the strip shows
// them, which is the order app.js imports their files in; a card's buttons
// open them.

import { $, accEl } from '../core/dom.js';
import { store } from '../model/state.js';
import { expandPanel, panelMode, selectDocument } from './selection.js';
import { dayShows } from './selection.js';

const contextTabsEl = $('context-tabs');
const contextTabs = []; // { id, title, el, show, button }, in the strip's order
let contextTab = null; // the id of the tab shown

export function shownContextTab() {
  return contextTab;
}

// `el`: the tab's part of the column; `show(focus)`: renders it as it is
// shown -- `focus` when the user asked for it, so its first field can take
// the focus, with `preventScroll`: the column may still be opening, 0px
// wide at the window's edge, and a plain focus would scroll the whole page
// sideways to it. The first tab registered is shown at first.
export function registerContextTab(tab) {
  const button = accEl('button');
  button.type = 'button';
  button.setAttribute('role', 'tab');
  button.textContent = tab.title;
  button.addEventListener('click', () => selectContextTab(tab.id, true));
  contextTabs.push({ ...tab, button });
  contextTabsEl.append(button);
  tab.el.setAttribute('role', 'tabpanel');
  selectContextTab(contextTab ?? tab.id);
}

// What the Task and Linked tabs are about: the selected card (a thread's
// or a day's focused one, the document shown), else the thread shown; ''
// for nothing.
export function contextTarget(state) {
  const mode = panelMode(state);
  if (mode === 'day') return dayShows(state.selected_entity) ? state.selected_entity : '';
  if (mode !== 'document' && mode !== 'thread') return '';
  return state.selected_entity ?? state.selected_thread ?? '';
}

// Show the tab `id`; `focus`: the user asked for it.
export function selectContextTab(id, focus = false) {
  contextTab = id;
  for (const tab of contextTabs) {
    const shown = tab.id === id;
    tab.button.setAttribute('aria-selected', String(shown));
    tab.el.hidden = !shown;
    if (shown) tab.show?.(focus);
  }
}

// ----------------------------------------------------------------------------
// A card's Task button: the Task tab for the card, in the expanded panel.
export function openTaskTab(doc_id) {
  selectDocument(doc_id);
  expandPanel();
  selectContextTab('task', true);
}

// A card's Link button: the Linked tab for the card, in the expanded panel,
// with the search focused.
export function openLinkedTab(doc_id) {
  selectDocument(doc_id);
  expandPanel();
  selectContextTab('linked', true);
}

// A card's chat button: the Conversation tab beside the card, the panel
// half-open (`panel_size: 'chat'`; at full width it stays so), the input
// focused. Pressed while the chat is open, it goes back to the normal panel.
export function toggleCardChat(doc_id) {
  if (store.getState().panel_size === 'chat') {
    store.setState({ panel_size: 'normal' });
    return;
  }
  selectDocument(doc_id);
  if (store.getState().panel_size !== 'expanded') store.setState({ panel_size: 'chat' });
  selectContextTab('conversation', true);
}

// ----------------------------------------------------------------------------
// The note that `id` was made -- `text`, and Open, which shows it (`hint`
// its tooltip): the Task tab's, and an assistant reply's saved as a
// document (features/assistant/assistant.js).
export function createdNote(text, id, hint) {
  const note = accEl('div', 'note new__created', text);
  const open = accEl('button', 'tool', 'Open');
  open.type = 'button';
  open.title = hint;
  open.addEventListener('click', () => selectDocument(id));
  note.append(open);
  return note;
}
