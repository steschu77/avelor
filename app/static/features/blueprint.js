// A project's blueprint in the lens header (panel.md#blueprint,
// vault.md#project), under its description while its details are unfolded
// (`showBlueprint`, features/header.js): its facts on a line, each edited in
// place (features/facts.js); then a tab line, Missing and Next with their
// counts and how far it is (`checksOf`, model/blueprint.js). One list is
// open at a time, its tab again folds it, so the lens keeps its room. A
// row's task opens on a click; an expected document's gap has Find, which
// picks one of the project's members to meet it.

import { $, accEl, oncePerFrame } from '../core/dom.js';
import { pressable } from '../core/dom.js';
import { DATE_FORMAT, amountText } from '../core/format.js';
import { t } from '../core/text.js';
import { store } from '../model/state.js';
import { projects } from '../model/entities.js';
import { checksOf, factsOf, hasBlueprint, membersOf } from '../model/blueprint.js';
import { setProjectLocal } from '../model/structure.js';
import { KIND_ICONS, docIcon, iconEl } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import { factEl } from './facts.js';
import { TASK_STATUSES } from './panel/details.js';
import { selectDocument } from './selection.js';

const bpEl = $('project-bp');

// The project shown, null while the details are folded or none is picked;
// the list open (`missing`, `next`), null for none.
let shownId = null;
let openList = 'missing';

// Show the blueprint of the project `id`, null to hide it.
export function showBlueprint(id) {
  shownId = id;
  render();
}

// A render waits while a fact is typed in, and runs once the facts are left.
function typing() {
  return bpEl.contains(document.activeElement) && document.activeElement.tagName === 'INPUT';
}

const render = oncePerFrame(() => {
  if (typing()) return;
  const project = shownId ? projects.get(shownId) : null;
  bpEl.hidden = !project || !hasBlueprint(project.project);
  if (bpEl.hidden) {
    bpEl.replaceChildren();
    return;
  }
  bpEl.replaceChildren(...renderBlueprint(project));
});
bpEl.addEventListener('focusout', () => setTimeout(() => {
  if (!typing()) render();
}));
store.subscribe([state => state.projects_rev, state => state.entities_rev], () => render());

// ----------------------------------------------------------------------------
function renderBlueprint({ id, project, writable }) {
  const result = checksOf(id, project);
  const budget = result.progress.budget;
  const ctx = { id, project, writable };
  const facts = accEl('div', 'bp__facts');
  facts.append(...factsOf(project).map(fact =>
    factEl(fact, ctx, fact.name === 'budget' && budget ? t('bp.spent', { amount: amountText(budget.spent) }) : null)));

  const lists = {
    missing: [t('bp.missing'), result.missing.map(gap => gapRow(gap, ctx)), t('bp.nothing_missing')],
    next: [t('props.next'), result.next.map(nextRow), t('bp.nothing_next')],
  };
  const tabs = accEl('div', 'bp__tabs');
  for (const [key, [label, rows]] of Object.entries(lists)) {
    const tab = accEl('button', 'tool label', label);
    tab.type = 'button';
    tab.setAttribute('aria-pressed', String(openList === key));
    tab.append(accEl('span', 'meta', String(rows.length)));
    tab.addEventListener('click', () => {
      openList = openList === key ? null : key;
      render();
    });
    tabs.append(tab);
  }
  tabs.append(accEl('span', 'meta bp__progress', progressText(result.progress)));

  const parts = [facts, tabs];
  if (openList) {
    const [, rows, none] = lists[openList];
    const list = accEl('div', 'bp__list');
    list.append(...(rows.length ? rows : [accEl('p', 'note bp__none', none)]));
    parts.push(list);
  }
  return parts;
}

// How far the project is, a part per check.
function progressText({ documents, payments, budget, steps }) {
  return [
    documents && `documents ${documents[0]} of ${documents[1]}`,
    payments && `payments ${payments[0]} of ${payments[1]}`,
    steps && `steps ${steps[0]} of ${steps[1]}`,
    budget && `${amountText(budget.spent)} of ${amountText(budget.budget)}`,
  ].filter(Boolean).join(' \u00b7 ');
}

// ----------------------------------------------------------------------------
// A line of the blueprint: its icon, its text with a note under it (the
// rule that found a gap first), and what acts on it at the right.
function itemEl(icon, title, sub, { gap = false, rule = null, action = null, open = null } = {}) {
  const item = accEl('div', 'bp__item' + (gap ? ' bp__item--gap' : ''));
  const text = accEl('span', 'bp__text', title);
  const note = accEl('span', 'meta bp__sub');
  if (rule) note.append(accEl('span', 'bp__rule', rule), sub ? ` \u00b7 ${sub}` : '');
  else note.append(sub);
  text.append(note);
  item.append(iconEl(icon), text, action ?? accEl('span'));
  if (open) {
    item.classList.add('bp__item--open');
    pressable(text).addEventListener('click', open);
  }
  return item;
}

function toolButton(label, title, run) {
  const btn = accEl('button', 'tool label', label);
  btn.type = 'button';
  btn.title = title;
  btn.addEventListener('click', () => run(btn));
  return btn;
}

const WARNING = '#i-warning';
const dayText = day => (day ? DATE_FORMAT.format(day) : '');

// A gap the checks found (`checksOf`'s `missing`).
function gapRow(gap, ctx) {
  if (gap.rule === 'documents') {
    const { expected, index, stale } = gap;
    const name = expected.name || t('bp.document');
    const how = [expected.type, expected.label, expected.within && t('bp.within', { within: expected.within })].filter(Boolean).join(', ');
    const action = ctx.writable
      ? toolButton(t('bp.find'), t('bp.find_title', { name }), btn => findDocument(btn, ctx, index))
      : null;
    return itemEl(WARNING, stale ? t('bp.too_old', { name }) : t('bp.none_yet', { name }), how,
      { gap: true, rule: t('types.documents'), action });
  }
  if (gap.rule === 'payments') {
    const { payment, due } = gap;
    return itemEl(WARNING, t('bp.not_paid', { what: paymentText(payment) }), t('timeline.due', { day: dayText(due) }),
      { gap: true, rule: t('dash.card.payments') });
  }
  if (gap.rule === 'budget') {
    return itemEl(WARNING, t('bp.over_budget', { amount: amountText(gap.spent - gap.budget) }),
      t('count.of_total', { at: amountText(gap.spent), n: amountText(gap.budget) }), { gap: true, rule: t('bp.rule.budget') });
  }
  const { doc } = gap;
  return itemEl(WARNING, doc.title || t('untitled'), t('timeline.due', { day: dayText(doc.due) }),
    { gap: true, rule: t('bp.rule.deadlines'), open: () => selectDocument(doc.doc_id) });
}

const STATUS = new Map(TASK_STATUSES);

// A next step (`checksOf`'s `next`): an open task, or a payment awaited.
function nextRow(next) {
  if (next.doc) {
    const { doc } = next;
    const sub = [STATUS.get(doc.task.status) ?? doc.task.status, !isNaN(doc.due) && doc.due ? t('timeline.due', { day: dayText(doc.due) }) : null];
    return itemEl(KIND_ICONS.task.icon, doc.title || t('untitled'), sub.filter(Boolean).join(' \u00b7 '),
      { open: () => selectDocument(doc.doc_id) });
  }
  const { payment, due } = next;
  return itemEl(KIND_ICONS.statement.icon, paymentText(payment), due ? t('timeline.due', { day: dayText(due) }) : t('bp.awaited'));
}

// A payment expected, with its amount: "Deposit, 200.00".
function paymentText(payment) {
  const name = payment.name || t('bp.a_payment');
  return payment.amount != null ? `${name}, ${amountText(payment.amount)}` : name;
}

// Find: the project's entries to pick the one that meets the expected
// document `index`; the pick is written into its `by`.
function findDocument(anchor, { id, project }, index) {
  const expected = project.documents[index];
  const by = new Set(expected.by ?? []);
  const members = membersOf(id).filter(doc => !by.has(doc.doc_id) && !doc.task)
    .sort((a, b) => (b.date?.getTime() ?? 0) - (a.date?.getTime() ?? 0));
  const items = members.map(doc => ({
    label: doc.title || t('untitled'), icon: docIcon(doc),
    desc: doc.date ? DATE_FORMAT.format(doc.date) : '',
    run: () => setProjectLocal(id, {
      ...project,
      documents: project.documents.map((d, i) => (i === index ? { ...d, by: [...(d.by ?? []), doc.doc_id] } : d)),
    }),
  }));
  showMenu(anchor, items.length ? items : [{ label: t('bp.no_entries'), dim: true }],
    { head: t('bp.meets', { name: expected.name || t('bp.document') }), filter: { placeholder: t('bp.filter_entries') } });
}
