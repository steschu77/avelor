// The Documents lens (ui.md#documents): the entities as rows. This file
// owns the rows and the sort-state UI, model/entities.js the entities.

import { $, accEl, oncePerFrame, placeChildren } from '../../core/dom.js';
import { DATE_FORMAT } from '../../core/format.js';
import { store } from '../../model/state.js';
import { entities } from '../../model/entities.js';
import { getDocIcon, iconEl } from '../../ui/icons.js';

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

// ----------------------------------------------------------------------------
// The rows by entity id, kept across renders, so a scan only adds rows
// instead of rebuilding the lens every frame.
const rows = new Map();
const rowDocs = new WeakMap(); // a document's row -> the entity it was filled from

// Rows of what left the vault are dropped.
function pruneRows() {
  for (const doc_id of rows.keys()) {
    if (!entities.has(doc_id)) rows.delete(doc_id);
  }
}

// An entity's row: created on first use, refilled when `entities` holds a
// new object for it (`onDocMeta` always stores a fresh one).
function docRow(doc) {
  let el = rows.get(doc.doc_id);
  if (!el) {
    el = accEl('div', 'list-row table__row');
    el.dataset.id = doc.doc_id;
    const cell = accEl('span', 'docs__cell');
    cell.append(iconEl('', 'table__icon'), accEl('span', 'row-title'));
    const date = accEl('span', 'row-title docs__date');
    el.append(cell, date);
    rows.set(doc.doc_id, el);
  }
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
  pruneRows();
  placeChildren(rowsEl, sortItems([...entities.values()]).map(docRow));
}

// The lenses showing entities render at most once per frame. A scan sends
// an `entity` event per file.
export const scheduleRenderDocuments = oncePerFrame(renderDocuments);

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
