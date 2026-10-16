// The Documents lens (ui.md#documents): the entities in the vault panel's
// filter as rows -- under All Projects in a group per project, then those
// in no project; a thread's members under the thread's row -- each group
// and thread folding (the projects folded kept in the `doc_folded`
// preference); its own type selection (features/lenses/types.js; the
// `doc_types` and `doc_types_bar` preferences); and one entity shown on
// request (`revealInDocuments`, the selection restored on start). This
// file owns the rows, the folds and the sort-state UI, model/entities.js
// the entities and features/selection.js the selection.

import { $, accEl, oncePerFrame, placeChildren } from '../../core/dom.js';
import { setAria } from '../../core/dom.js';
import { DATE_FORMAT } from '../../core/format.js';
import { shortDate } from '../../core/format.js';
import { plural } from '../../core/format.js';
import { store } from '../../model/state.js';
import { savePrefs } from '../../model/state.js';
import { isDirty } from '../../model/documents.js';
import { entities } from '../../model/entities.js';
import { matchesFilter } from '../../model/entities.js';
import { projects, projectsInOrder, projectsOf } from '../../model/entities.js';
import { knownThreads, threads } from '../../model/entities.js';
import { getDocIcon, iconEl } from '../../ui/icons.js';
import { KIND_ICONS } from '../../ui/icons.js';
import { markSelection, selectById, selectDocument } from '../selection.js';
import { hiddenText, typeBar, typeOf } from './types.js';

const lensEl = $('lens-documents');
const rowsEl = $('doc-rows');
const countEl = $('doc-count');
const hiddenEl = $('doc-hidden');
const hiddenTextEl = $('doc-hidden-text');

// Sorting through a shared instance: `localeCompare` builds a collator per
// call.
const TEXT_ORDER = new Intl.Collator();

// ----------------------------------------------------------------------------
// The type selection (features/lenses/types.js): Documents, Mail and Notes
// shown at first.
const docTypes = typeBar({
  el: $('doc-types'), pref: 'doc_types', barPref: 'doc_types_bar',
  shownAtFirst: ['documents', 'mail', 'notes'],
});

$('doc-show-all').addEventListener('click', docTypes.showAll);

// ----------------------------------------------------------------------------
const SORT_COMPARATORS = {
  kind: (a, b) => TEXT_ORDER.compare(a.kind, b.kind),
  title: (a, b) => TEXT_ORDER.compare(a.title, b.title),
  date: (a, b) => {
    if (a.date?.getTime() === b.date?.getTime()) return 0;
    if (a.date === undefined) return 1;
    if (b.date === undefined) return -1;
    return a.date < b.date ? -1 : 1;
  },
};

// Entities and threads' items alike (`title`, `kind`, `date`).
function sortItems(items) {
  const sortState = store.getState().sort_state;
  return items.sort((a, b) => {
    for (const { key, dir } of sortState) {
      const cmp = SORT_COMPARATORS[key](a, b);
      if (cmp !== 0) return dir === 'asc' ? cmp : -cmp;
    }
    return 0;
  });
}

// The groups the rows fall into: under All Projects, every project in the
// vault panel's order, each with its entities (one in several projects is
// in each), and then the entities in no project of the vault, a group
// without a row (`key` ''); with a project or No project selected, that
// group alone.
function groupsOf(docs, selected_project) {
  if (selected_project) return [{ key: '', docs }];
  const order = projectsInOrder();
  const byProject = new Map(order.map(project => [project.id, []]));
  const none = [];
  for (const doc of docs) {
    const ids = projectsOf(doc);
    if (!ids.length) none.push(doc);
    for (const id of new Set(ids)) byProject.get(id).push(doc);
  }
  return [
    ...order.map(project => ({ key: project.id, name: project.name, project, docs: byProject.get(project.id) })),
    { key: '', docs: none },
  ].filter(group => group.docs.length);
}

// A group's items, sorted: its entities in no thread, and an item per
// thread of its members (`knownThreads`, a member in several threads in
// each), dated by its newest member here.
function itemsOf(docs) {
  const items = [];
  const byThread = new Map();
  for (const doc of docs) {
    const ids = knownThreads(doc);
    if (!ids.length) items.push(doc);
    for (const thread_id of ids) {
      let item = byThread.get(thread_id);
      if (!item) {
        item = { thread_id, title: threads.get(thread_id).title ?? '', kind: 'thread', date: undefined, members: [] };
        byThread.set(thread_id, item);
        items.push(item);
      }
      item.members.push(doc);
      if (doc.date && (!item.date || doc.date > item.date)) item.date = doc.date;
    }
  }
  for (const item of byThread.values()) sortItems(item.members);
  return sortItems(items);
}

// ----------------------------------------------------------------------------
// The folds. A project's group is folded while its id is in the
// `doc_folded` preference (the ids joined by commas; before the first
// fold, the archived projects' are), so the folds outlast a restart
// without changing the vault (vault.md#principles); a group a reveal
// opened stays open for the session, the preference unchanged. The
// threads unfolded, by id -- one unfolded shows in every group -- are kept
// while the app runs.
const revealedGroups = new Set(); // project ids
const unfoldedThreads = new Set();

function foldedProjects() {
  const saved = store.getState().prefs.doc_folded;
  if (saved === undefined) return new Set([...projects.values()].filter(p => p.archived).map(p => p.id));
  return new Set(saved.split(',').filter(Boolean));
}

function groupFolded(key) {
  return !revealedGroups.has(key) && foldedProjects().has(key);
}

// A group a reveal opened is folded again with its id already in the
// preference: then nothing is saved.
function toggleGroup(key) {
  const folded = foldedProjects();
  if (groupFolded(key)) folded.delete(key);
  else folded.add(key);
  revealedGroups.delete(key);
  const doc_folded = [...folded].join(',');
  if (doc_folded !== store.getState().prefs.doc_folded) savePrefs({ doc_folded });
  scheduleRenderDocuments();
}

function toggleThread(thread_id) {
  if (!unfoldedThreads.delete(thread_id)) unfoldedThreads.add(thread_id);
  scheduleRenderDocuments();
}

// ----------------------------------------------------------------------------
// The rows by key -- `g`roup, `t`hread or `d`ocument, with the group, the
// thread and the entity they are for -- kept across renders (also while
// filtered out or folded), so a scan only adds rows instead of rebuilding
// the lens every frame.
const rows = new Map();
const rowFor = new WeakMap(); // row element -> { group, thread_id, doc_id }
const rowDocs = new WeakMap(); // a document's row -> the entity it was filled from

const groupKey = key => `g\n${key}`;
const threadKey = (group, thread_id) => `t\n${group}\n${thread_id}`;
const docKey = (group, thread_id, doc_id) => `d\n${group}\n${thread_id ?? ''}\n${doc_id}`;

// The rows of the entity `doc_id`, wherever it shows (app.js keeps their
// dirty dots).
export function getDocRows(doc_id) {
  return [...rows.values()].filter(el => rowFor.get(el).doc_id === doc_id);
}

// Rows of what left the vault are dropped.
function pruneRows() {
  for (const [key, el] of rows) {
    const { group, thread_id, doc_id } = rowFor.get(el);
    const gone = (group && !projects.has(group))
      || (thread_id && !threads.has(thread_id)) || (doc_id && !entities.has(doc_id));
    if (gone) rows.delete(key);
  }
}

function cachedRow(key, info, create) {
  let el = rows.get(key);
  if (!el) {
    el = create();
    rows.set(key, el);
    rowFor.set(el, info);
  }
  return el;
}

// A cell of the name column, one level deeper per `depth`.
function nameCell(depth) {
  const cell = accEl('span', depth ? `docs__cell docs__cell--l${depth}` : 'docs__cell');
  return cell;
}

// A fold's chevron: a thread's is a button, a group row folds as a whole.
function foldEl(tag) {
  const fold = accEl(tag, 'docs__fold');
  if (tag === 'button') fold.type = 'button';
  fold.append(iconEl('#i-chevron-down', 'disclosure'));
  return fold;
}

// A project's group: its fold, icon, name, count and Open (the project as
// the filter). A click anywhere else folds it.
function groupRow(group) {
  const el = cachedRow(groupKey(group.key), { group: group.key }, () => {
    const row = accEl('div', 'list-row group-row');
    row.dataset.group = group.key;
    const cell = nameCell(0);
    const icon = iconEl(KIND_ICONS.project.icon, 'group-row__icon');
    const name = accEl('span', 'group-row__name');
    const count = accEl('span', 'meta');
    const open = accEl('button', 'tool label group-row__open', 'Open');
    open.type = 'button';
    cell.append(foldEl('span'), icon, name, count, open);
    row.append(cell, accEl('span'));
    return row;
  });
  const [fold, , name, count, open] = el.firstChild.children;
  const folded = groupFolded(group.key);
  fold.setAttribute('aria-expanded', String(!folded));
  name.textContent = group.name;
  count.textContent = group.docs.length;
  open.title = `Open ${group.name}`;
  return el;
}

// The projects' groups sit in a box of their own, so a group's row stays
// under the head only while the groups scroll, not over the entities in no
// project after them; its end is the separator between the two.
const projectsEl = accEl('div', 'docs__projects');

// A thread's row: its fold, icon, title, "5 of 6, 14/08 - 23/09" (its
// members shown here of all of them, and their days) and its newest
// member's date. A click shows the thread in the panel.
function threadRow(group, item, depth, total) {
  const key = threadKey(group, item.thread_id);
  const el = cachedRow(key, { group, thread_id: item.thread_id }, () => {
    const row = accEl('div', 'list-row table__row');
    row.dataset.thread = item.thread_id;
    const cell = nameCell(depth);
    const icon = iconEl(KIND_ICONS.thread.icon, 'table__icon');
    const title = accEl('span', 'row-title');
    const members = accEl('span', 'meta docs__members');
    cell.append(foldEl('button'), icon, title, members);
    const date = accEl('span', 'row-title docs__date');
    row.append(cell, date);
    return row;
  });
  const [fold, , title, members] = el.firstChild.children;
  const unfolded = unfoldedThreads.has(item.thread_id);
  fold.setAttribute('aria-expanded', String(unfolded));
  fold.title = unfolded ? 'Fold the thread' : 'Unfold the thread';
  title.textContent = item.title;
  const count = item.members.length === total ? `${total}` : `${item.members.length} of ${total}`;
  const days = memberDays(item.members);
  members.textContent = days ? `${count} \u00b7 ${days}` : count;
  el.lastChild.textContent = item.date ? DATE_FORMAT.format(item.date) : '\u2014';
  return el;
}

// The days of a thread's `members` here: "14/08 - 23/09", one day alone.
function memberDays(members) {
  const dates = members.map(doc => doc.date).filter(Boolean).sort((a, b) => a - b);
  if (!dates.length) return '';
  const [first, last] = [shortDate(dates[0]), shortDate(dates.at(-1))];
  return first === last ? first : `${first} \u2013 ${last}`;
}

// An entity's row, in the thread `thread_id` (or none): created on first
// use, refilled when `entities` holds a new object for it (`onDocMeta`
// always stores a fresh one).
function docRow(group, thread_id, doc, depth) {
  const el = cachedRow(docKey(group, thread_id, doc.doc_id), { group, thread_id, doc_id: doc.doc_id }, () => {
    const row = accEl('div', 'list-row table__row');
    row.dataset.id = doc.doc_id;
    if (thread_id) row.dataset.thread = thread_id;
    const cell = nameCell(depth);
    const gap = accEl('span', 'docs__gap');
    // the title text node comes first, so refilling it keeps the dot
    const title = accEl('span', 'row-title');
    title.appendChild(document.createTextNode(''));
    const dirtyDot = accEl('span', 'dirty-dot');
    dirtyDot.title = 'Unsaved changes';
    dirtyDot.hidden = !isDirty(doc.doc_id);
    title.appendChild(dirtyDot);
    cell.append(gap, iconEl('', 'table__icon'), title);
    const date = accEl('span', 'row-title docs__date');
    row.append(cell, date);
    return row;
  });
  if (rowDocs.get(el) !== doc) {
    const [, icon, title] = el.firstChild.children;
    icon.firstChild.setAttribute('href', getDocIcon(doc.kind, doc.genre));
    title.firstChild.data = doc.title;
    el.lastChild.textContent = doc.date ? DATE_FORMAT.format(doc.date) : '\u2014';
    rowDocs.set(el, doc);
  }
  return el;
}

// ----------------------------------------------------------------------------
// An entity or thread whose row the next render unfolds and scrolls to.
let revealId = null;

export function revealInDocuments(id) {
  revealId = id;
  scheduleRenderDocuments();
}

// Unfold the first group holding `id` (an entity, or a thread one of whose
// members it holds; for the session) and, for an entity in a thread, its
// first thread: the key of the row to scroll to.
function unfoldFor(groups, id) {
  for (const group of groups) {
    const doc = group.docs.find(d => d.doc_id === id);
    if (!doc && !group.docs.some(d => knownThreads(d).includes(id))) continue;
    if (groupFolded(group.key)) revealedGroups.add(group.key);
    if (!doc) return threadKey(group.key, id);
    const thread_id = knownThreads(doc)[0];
    if (thread_id) unfoldedThreads.add(thread_id);
    return docKey(group.key, thread_id, id);
  }
  return null;
}

const markRows = markSelection(rowsEl);

// A thread's rows are marked while the panel shows the thread.
function markThreadRows() {
  const { selected_thread } = store.getState();
  for (const el of rowsEl.querySelectorAll('.table__row[data-thread]:not([data-id])')) {
    setAria(el, 'aria-selected', el.dataset.thread === selected_thread);
  }
}
store.subscribe(state => state.selected_thread, markThreadRows);

function renderDocuments() {
  const state = store.getState();
  const shown = docTypes.shown();
  // the filter's entities, counted by type; those of a type shown are listed
  const counts = new Map();
  const listed = [];
  for (const doc of entities.values()) {
    if (!matchesFilter(doc, state)) continue;
    const type = typeOf(doc);
    counts.set(type, (counts.get(type) ?? 0) + 1);
    if (shown.has(type)) listed.push(doc);
  }
  const memberTotals = new Map(); // thread id -> its members in the vault
  for (const doc of entities.values()) {
    for (const thread_id of knownThreads(doc)) memberTotals.set(thread_id, (memberTotals.get(thread_id) ?? 0) + 1);
  }

  pruneRows();
  const groups = groupsOf(listed, state.selected_project);
  const revealKey = revealId ? unfoldFor(groups, revealId) : null;
  revealId = null;

  const projectNodes = [];
  const nodes = [];
  for (const group of groups) {
    const into = group.key ? projectNodes : nodes;
    const depth = group.key ? 1 : 0;
    if (group.key) {
      into.push(groupRow(group));
      if (groupFolded(group.key)) continue;
    }
    for (const item of itemsOf(group.docs)) {
      if (!item.members) {
        into.push(docRow(group.key, null, item, depth));
        continue;
      }
      into.push(threadRow(group.key, item, depth, memberTotals.get(item.thread_id)));
      if (!unfoldedThreads.has(item.thread_id)) continue;
      for (const doc of item.members) into.push(docRow(group.key, item.thread_id, doc, depth + 1));
    }
  }
  placeChildren(projectsEl, projectNodes);
  placeChildren(rowsEl, projectNodes.length ? [projectsEl, ...nodes] : nodes);
  markRows();
  markThreadRows();
  if (revealKey) rows.get(revealKey).scrollIntoView({ block: 'nearest' });

  docTypes.render(shown, counts);
  const hidden = [...counts].reduce((n, [type, count]) => n + (shown.has(type) ? 0 : count), 0);
  const listedText = plural(listed.length, 'document');
  countEl.textContent = hidden ? `${listedText} \u00b7 ${hidden} hidden` : listedText;
  hiddenEl.hidden = !hidden;
  hiddenTextEl.textContent = hidden ? `${hiddenText(shown, counts)} hidden` : '';
}

// The lenses showing entities render at most once per frame. A scan sends
// an `entity` event per file.
export const scheduleRenderDocuments = oncePerFrame(renderDocuments);

// ----------------------------------------------------------------------------
// A click: a fold folds, Open makes the group's project the filter, a group
// row folds, a thread's row shows the thread in the panel, an entity's row
// the entity, in the thread it is shown under.
rowsEl.addEventListener('click', ev => {
  const row = ev.target.closest('.list-row');
  if (!row) return;
  if (row.dataset.group !== undefined) {
    if (ev.target.closest('.group-row__open')) store.setState({ selected_project: row.dataset.group });
    else toggleGroup(row.dataset.group);
  } else if (ev.target.closest('.docs__fold')) {
    toggleThread(row.dataset.thread);
  } else if (row.dataset.id) {
    selectDocument(row.dataset.id, row.dataset.thread ?? null);
  } else {
    selectById(row.dataset.thread);
  }
});

// ----------------------------------------------------------------------------
function renderSortHead() {
  const primary = store.getState().sort_state[0];
  for (const btn of lensEl.querySelectorAll('.table__sort')) {
    const isPrimary = btn.dataset.sort === primary.key;
    btn.classList.toggle('table__sort--active', isPrimary);
    if (isPrimary) btn.dataset.dir = primary.dir;
    else delete btn.dataset.dir;
  }
}

function onSortClick(key) {
  const sortState = store.getState().sort_state;
  const entry = sortState.find(s => s.key === key);
  const rest = sortState.filter(s => s.key !== key);
  const isPrimary = sortState[0].key === key;
  const next = isPrimary
    ? [{ key, dir: entry.dir === 'asc' ? 'desc' : 'asc' }, ...rest]
    : [entry, ...rest];
  store.setState({ sort_state: next });
  renderSortHead();
  scheduleRenderDocuments();
}

export function initDocumentsLens() {
  for (const btn of lensEl.querySelectorAll('.table__sort')) {
    btn.addEventListener('click', () => onSortClick(btn.dataset.sort));
  }
  renderSortHead();
}
