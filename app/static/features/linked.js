// The Linked tab of the panel's context column (panel.md#links):
// the links of what the panel shows (`contextTarget`) -- the entity it
// links to and those that link to it -- and a search over the titles in
// the vault panel's filter to link it to another. The card's Link button
// opens it (`openLinkedTab`). A target's part is built once, so the search
// keeps its focus and its text; the lists are built again as the entities
// or the filter change. Links are written with `setLinkLocal`.

import { $, accEl, oncePerFrame } from '../core/dom.js';
import { DATE_FORMAT } from '../core/format.js';
import { store } from '../model/state.js';
import {
  entities, matchesFilter, threadMatchesFilter, threads,
} from '../model/entities.js';
import { setLinkLocal } from '../model/structure.js';
import { iconEl } from '../ui/icons.js';
import { selectById } from './selection.js';
import {
  contextTarget, registerContextTab, shownContextTab,
} from './context.js';
import { linkTarget, linksOf } from './panel/panel.js';

const LINKED_RESULTS = 20; // the search shows at most this many

const linkedTabEl = $('context-linked');
let linkedShown = null; // { target, query, els } of the target shown

registerContextTab({ id: 'linked', title: 'Linked', el: linkedTabEl, show: focus => renderLinkedTab(focus) });

store.subscribe(contextTarget, () => {
  if (shownContextTab() === 'linked') renderLinkedTab();
});

// A link written, an entity changed, another filter in the vault panel: the
// lists follow.
const scheduleLinkedLists = oncePerFrame(() => {
  if (shownContextTab() === 'linked') renderLinkedLists();
});
store.subscribe([
  state => state.entities_rev, state => state.threads_rev, state => state.selected_project,
  state => state.selected_labels,
], scheduleLinkedLists);

// Whether the entity or thread `id` can be written: its link set.
function linkedWritable(id) {
  return !!(entities.get(id) ?? threads.get(id))?.writable;
}

// Show the links of what the panel shows; the same target's part stays as
// it is (and keeps the focus), only its lists are built again. `focus`:
// the search takes it.
function renderLinkedTab(focus = false) {
  const target = contextTarget(store.getState());
  if (linkedShown?.target !== target) buildLinkedTab(target);
  renderLinkedLists();
  if (focus) linkedShown.els.search?.focus({ preventScroll: true });
}

// The part of `target` ('' for nothing shown): "Links to", "Linked from",
// and the search when its link can be set.
function buildLinkedTab(target) {
  linkedShown = { target, query: '', els: {} };
  if (!target) {
    linkedTabEl.replaceChildren(accEl('div', 'note', 'Select a document to see its links.'));
    return;
  }
  const els = linkedShown.els;
  els.to = accEl('div');
  els.from = accEl('div');
  const parts = [
    accEl('div', 'section-head', 'Links to'), els.to,
    accEl('div', 'section-head linked__head', 'Linked from'), els.from,
  ];
  if (linkedWritable(target)) {
    const head = accEl('div', 'section-head linked__head', 'Link to\u2026');
    head.append(accEl('span', 'section-head__note', 'replaces the link'));
    els.search = accEl('input', 'input linked__search');
    els.search.type = 'search';
    els.search.placeholder = 'Search the titles';
    els.search.addEventListener('input', () => {
      linkedShown.query = els.search.value;
      renderLinkedResults();
    });
    els.search.addEventListener('keydown', e => {
      if (e.key !== 'Enter') return;
      e.preventDefault();
      const first = linkedResults(target, linkedShown.query)[0];
      if (first) setLinkLocal(target, first);
    });
    els.results = accEl('div');
    parts.push(head, els.search, els.results);
  }
  linkedTabEl.replaceChildren(...parts);
}

// The lists of the target shown: its links, both ways, and the results.
function renderLinkedLists() {
  const target = linkedShown?.target;
  if (!target) return;
  const { to, from, items } = linksOf(target);
  const unlink = id => (linkedWritable(id) ? () => setLinkLocal(id, null) : null);
  const none = () => accEl('div', 'meta linked__none', 'none');

  linkedShown.els.to.replaceChildren(to ? linkedRow(to, { onRemove: unlink(target) }) : none());
  const rows = [
    ...from.map(id => linkedRow(id, { onRemove: unlink(id) })),
    ...items.map(item => linkedRow(item.doc_id, { label: item.title || null })),
  ];
  linkedShown.els.from.replaceChildren(...(rows.length ? rows : [none()]));
  renderLinkedResults();
}

// The search's results: a pick links the target to it.
function renderLinkedResults() {
  const { target, query, els } = linkedShown;
  if (!els.results) return;
  const rows = linkedResults(target, query).map(id => linkedRow(id, {
    title: 'Link to this',
    onPick: () => setLinkLocal(target, id),
  }));
  els.results.replaceChildren(...(rows.length ? rows : [accEl('div', 'meta linked__none', 'No match in the current filter')]));
}

// The entities and threads in the vault panel's filter (project, labels:
// `matchesFilter`; a thread while one of its members is) whose
// title holds `query` (any case), newest first, at most LINKED_RESULTS --
// but `target` and what it links to.
function linkedResults(target, query) {
  const q = query.trim().toLowerCase();
  const current = (entities.get(target) ?? threads.get(target))?.linked_to;
  const time = id => linkTarget(id)?.date?.getTime() ?? 0;
  const state = store.getState();
  return [
    ...[...entities.values()].filter(doc => matchesFilter(doc, state)),
    ...[...threads.values()].filter(thread => threadMatchesFilter(thread.id, state)),
  ]
    .map(doc => doc.id)
    .filter(id => id !== target && id !== current)
    .filter(id => !q || (linkTarget(id)?.title ?? '').toLowerCase().includes(q))
    .sort((a, b) => time(b) - time(a))
    .slice(0, LINKED_RESULTS);
}

// One row: the icon, the title -- `label` instead, for a transaction item --
// and the date of `id`. A click on the title selects it, or runs `onPick`
// (titled `title`); an x calls `onRemove` (none without it).
function linkedRow(id, { label = null, title = 'Show it', onPick = null, onRemove = null } = {}) {
  const target = linkTarget(id);
  const row = accEl('div', 'list-row linked__row');
  const name = accEl('button', 'row-title linked__name', label ?? (target?.title || 'Untitled'));
  name.type = 'button';
  name.title = title;
  name.addEventListener('click', onPick ?? (() => selectById(id)));
  const date = accEl('span', 'meta', target?.date ? DATE_FORMAT.format(target.date) : '');
  row.append(iconEl(target?.icon ?? '', 'linked__icon'), name, date);
  if (onRemove) {
    const remove = accEl('button', 'tool');
    remove.type = 'button';
    remove.append(iconEl('#i-dismiss'));
    remove.title = 'Remove the link';
    remove.addEventListener('click', onRemove);
    row.append(remove);
  }
  return row;
}
