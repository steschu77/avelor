// The Documents lens (ui.md#documents): the entities in the vault panel's
// filter as rows -- under All Projects in a group per project, then those
// in no project -- each group folding (the projects folded kept in the
// `doc_folded` preference). This file owns the rows, the folds and the
// sort-state UI, model/entities.js the entities.

import { $, accEl, oncePerFrame, placeChildren } from '../../core/dom.js';
import { DATE_FORMAT } from '../../core/format.js';
import { store } from '../../model/state.js';
import { savePrefs } from '../../model/state.js';
import { entities } from '../../model/entities.js';
import { matchesFilter } from '../../model/entities.js';
import { projects, projectsInOrder, projectsOf } from '../../model/entities.js';
import { getDocIcon, iconEl } from '../../ui/icons.js';
import { KIND_ICONS } from '../../ui/icons.js';

const lensEl = $('lens-documents');
const rowsEl = $('doc-rows');

// Sorting through a shared instance: `localeCompare` builds a collator per
// call.
const TEXT_ORDER = new Intl.Collator();

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

// Entities (`title`, `kind`, `date`).
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

// ----------------------------------------------------------------------------
// The folds. A project's group is folded while its id is in the
// `doc_folded` preference (the ids joined by commas; before the first
// fold, the archived projects' are), so the folds outlast a restart
// without changing the vault (vault.md#principles).
function foldedProjects() {
  const saved = store.getState().prefs.doc_folded;
  if (saved === undefined) return new Set([...projects.values()].filter(p => p.archived).map(p => p.id));
  return new Set(saved.split(',').filter(Boolean));
}

function groupFolded(key) {
  return foldedProjects().has(key);
}

function toggleGroup(key) {
  const folded = foldedProjects();
  if (groupFolded(key)) folded.delete(key);
  else folded.add(key);
  savePrefs({ doc_folded: [...folded].join(',') });
  scheduleRenderDocuments();
}

// ----------------------------------------------------------------------------
// The rows by key -- `g`roup or `d`ocument, with the group and the entity
// they are for -- kept across renders (also while filtered out or folded),
// so a scan only adds rows instead of rebuilding the lens every frame.
const rows = new Map();
const rowFor = new WeakMap(); // row element -> { group, doc_id }
const rowDocs = new WeakMap(); // a document's row -> the entity it was filled from

const groupKey = key => `g\n${key}`;
const docKey = (group, doc_id) => `d\n${group}\n${doc_id}`;

// Rows of what left the vault are dropped.
function pruneRows() {
  for (const [key, el] of rows) {
    const { group, doc_id } = rowFor.get(el);
    const gone = (group && !projects.has(group)) || (doc_id && !entities.has(doc_id));
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

// A fold's chevron: a group row folds as a whole.
function foldEl() {
  const fold = accEl('span', 'docs__fold');
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
    cell.append(foldEl(), icon, name, count, open);
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

// An entity's row: created on first use, refilled when `entities` holds a
// new object for it (`onDocMeta` always stores a fresh one).
function docRow(group, doc, depth) {
  const el = cachedRow(docKey(group, doc.doc_id), { group, doc_id: doc.doc_id }, () => {
    const row = accEl('div', 'list-row table__row');
    row.dataset.id = doc.doc_id;
    const cell = nameCell(depth);
    cell.append(iconEl('', 'table__icon'), accEl('span', 'row-title'));
    const date = accEl('span', 'row-title docs__date');
    row.append(cell, date);
    return row;
  });
  if (rowDocs.get(el) !== doc) {
    const [icon, title] = el.firstChild.children;
    icon.firstChild.setAttribute('href', getDocIcon(doc.kind, doc.genre));
    title.textContent = doc.title;
    el.lastChild.textContent = doc.date ? DATE_FORMAT.format(doc.date) : '\u2014';
    rowDocs.set(el, doc);
  }
  return el;
}

function renderDocuments() {
  const state = store.getState();
  const listed = [];
  for (const doc of entities.values()) {
    if (matchesFilter(doc, state)) listed.push(doc);
  }

  pruneRows();
  const groups = groupsOf(listed, state.selected_project);

  const projectNodes = [];
  const nodes = [];
  for (const group of groups) {
    const into = group.key ? projectNodes : nodes;
    const depth = group.key ? 1 : 0;
    if (group.key) {
      into.push(groupRow(group));
      if (groupFolded(group.key)) continue;
    }
    for (const doc of sortItems(group.docs)) into.push(docRow(group.key, doc, depth));
  }
  placeChildren(projectsEl, projectNodes);
  placeChildren(rowsEl, projectNodes.length ? [projectsEl, ...nodes] : nodes);
}

// The lenses showing entities render at most once per frame. A scan sends
// an `entity` event per file.
export const scheduleRenderDocuments = oncePerFrame(renderDocuments);

// ----------------------------------------------------------------------------
// A click: Open makes the group's project the filter, a group row folds.
rowsEl.addEventListener('click', ev => {
  const row = ev.target.closest('.list-row');
  if (!row) return;
  if (row.dataset.group !== undefined) {
    if (ev.target.closest('.group-row__open')) store.setState({ selected_project: row.dataset.group });
    else toggleGroup(row.dataset.group);
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
