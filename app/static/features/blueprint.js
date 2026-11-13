// A project's blueprint in the lens header (panel.md#blueprint,
// vault.md#project), under its description while its details are unfolded
// (`showBlueprint`, features/header.js): its facts on a line, each edited in
// place (features/facts.js); a trip's route (features/route.js); then a tab
// line, Missing and Next with their counts and how far it is (`checksOf`,
// model/blueprint.js); while a trip's days are open, its When first
// (features/when.js), open once the trip is shown. One list is open at a
// time, its tab again folds it, so the lens keeps its room. A row's task
// opens on a click; an expected document's gap has Find, which picks one of
// the project's members to meet it; a trip's nights and journeys missing have
// Add (features/gaps.js), its other gaps open the event they are about.

import { $, accEl, oncePerFrame } from '../core/dom.js';
import { pressable } from '../core/dom.js';
import { DATE_FORMAT, amountText } from '../core/format.js';
import { t } from '../core/text.js';
import { store } from '../model/state.js';
import { projects } from '../model/entities.js';
import { checksOf, factsOf, hasBlueprint, membersOf } from '../model/blueprint.js';
import { setProjectLocal } from '../model/structure.js';
import { zoneName } from '../model/trip.js';
import { whenOf } from '../model/when.js';
import { KIND_ICONS, docIcon, iconEl } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import { toolButton } from '../ui/toolbar.js';
import { factEl, newFactEl } from './facts.js';
import { fillGap, gapText } from './gaps.js';
import { TASK_STATUSES } from './panel/details.js';
import { routeEls } from './route.js';
import { selectDocument } from './selection.js';
import { whenEls } from './when.js';

const bpEl = $('project-bp');

// The project shown, null while the details are folded or none is picked;
// the list open (`when`, `missing`, `next`), null for none, and the project
// it was opened for.
let shownId = null;
let openList = 'missing';
let openedFor = null;

// Show the blueprint of the project `id`, null to hide it.
export function showBlueprint(id) {
  shownId = id;
  render();
}

// A render waits while a fact is typed in, and runs once the facts are left;
// a radio picked (When's windows) is no typing.
function typing() {
  const el = document.activeElement;
  return bpEl.contains(el) && el.tagName === 'INPUT' && el.type !== 'radio' && el.type !== 'checkbox';
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
// Focus moving to a button of the blueprint (a pressed one takes it) draws
// nothing anew: the button would be replaced before its click.
bpEl.addEventListener('focusout', e => {
  if (bpEl.contains(e.relatedTarget)) return;
  setTimeout(() => {
    if (!typing()) render();
  });
});
store.subscribe([state => state.projects_rev, state => state.entities_rev, state => state.selected_entity], () => render());

// ----------------------------------------------------------------------------
function renderBlueprint({ id, project, writable }) {
  const result = checksOf(id, project);
  const budget = result.progress.budget;
  const ctx = { id, project, writable };
  const facts = accEl('div', 'bp__facts');
  facts.append(...factsOf(project).map(fact =>
    factEl(fact, ctx, fact.name === 'budget' && budget ? t('bp.spent', { amount: amountText(budget.spent) }) : null)));
  if (writable) facts.append(newFactEl(ctx));

  const when = whenOf(project);
  if (when && openedFor !== id) openList = 'when';
  if (!when && openList === 'when') openList = 'missing';
  openedFor = id;
  const missing = result.missing.map(gap => gapRow(gap, ctx));
  const next = result.next.map(nextRow);
  const rowsOr = (rows, none) => () => (rows.length ? rows : [accEl('p', 'note bp__none', none)]);
  // each list: its tab's label and count, and its rows
  const lists = {
    ...(when && { when: [t('props.when'), when.fits.length, () => whenEls(ctx, when, render)] }),
    missing: [t('bp.missing'), missing.length, rowsOr(missing, result.later ? laterText(result.later) : t('bp.nothing_missing'))],
    next: [t('props.next'), next.length, rowsOr(next, t('bp.nothing_next'))],
  };
  const tabs = accEl('div', 'bp__tabs');
  for (const [key, [label, count]] of Object.entries(lists)) {
    const tab = accEl('button', 'tool label', label);
    tab.type = 'button';
    tab.setAttribute('aria-pressed', String(openList === key));
    tab.append(accEl('span', 'meta', String(count)));
    tab.addEventListener('click', () => {
      openList = openList === key ? null : key;
      render();
    });
    tabs.append(tab);
  }
  tabs.append(accEl('span', 'meta bp__progress', progressText(result.progress)));

  const parts = [facts, ...routeEls(ctx, render), tabs];
  if (openList) {
    const list = accEl('div', 'bp__list');
    list.append(...lists[openList][2]());
    parts.push(list);
  }
  return parts;
}

// How far the project is, a part per check.
function progressText({ nights, journeys, documents, payments, budget, steps }) {
  return [
    nights && `nights ${nights[0]} of ${nights[1]}`,
    journeys && `journeys ${journeys[0]} of ${journeys[1]} booked`,
    documents && `documents ${documents[0]} of ${documents[1]}`,
    payments && `payments ${payments[0]} of ${payments[1]}`,
    steps && `steps ${steps[0]} of ${steps[1]}`,
    budget && `${amountText(budget.spent)} of ${amountText(budget.budget)}`,
  ].filter(Boolean).join(' \u00b7 ');
}

// What a trip's sketch will need, while its days are open.
function laterText({ nights, journeys }) {
  const parts = [nights && t('bp.later.nights', { n: nights }), journeys && t('bp.later.journeys', { n: journeys })].filter(Boolean);
  return parts.length ? t('bp.later', { parts: parts.join(', ') }) : t('bp.later_none');
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

const WARNING = '#i-warning';
const dayText = day => (day ? DATE_FORMAT.format(day) : '');

// A gap the checks found (`checksOf`'s `missing`).
function gapRow(gap, ctx) {
  if (TRIP_RULES[gap.rule]) return tripGapRow(gap, ctx);
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

const TRIP_RULES = { nights: t('props.nights'), journeys: t('bp.rule.journeys'), tickets: t('bp.rule.tickets') };

// A gap of a trip (`tripGaps`, model/trip.js): nights or a journey missing,
// with Add, the menu that fills it (`fillGap`); else what it is about a
// click away: the event, or the open task linked to a planned journey, which the
// row names instead.
function tripGapRow(gap, { id }) {
  const rule = gap.kind === 'outside' ? t('bp.rule.days') : TRIP_RULES[gap.rule];
  const opens = doc => ({ gap: true, rule, open: () => selectDocument(doc.doc_id) });
  const adds = title => ({ gap: true, rule, action: toolButton(t('timeline.add'), title, btn => fillGap(btn, id, gap)) });
  const name = doc => doc.title || t('untitled');
  switch (gap.kind) {
    case 'night':
    case 'missing': {
      const { title, when } = gapText(gap);
      return itemEl(WARNING, title, when, adds(gap.kind === 'night' ? t('timeline.add_stay') : t('timeline.add_journey')));
    }
    case 'planned': {
      const { doc, task } = gap;
      if (task) {
        const due = !isNaN(task.due) && task.due ? t('timeline.due', { day: dayText(task.due) }) : null;
        return itemEl(KIND_ICONS.task.icon, name(task), [t('bp.for_planned', { name: name(doc) }), due].filter(Boolean).join(' \u00b7 '),
          opens(task));
      }
      return itemEl(WARNING, t('bp.not_booked', { name: name(doc) }), t('bp.planned_on', { day: dayText(doc.date) }), opens(doc));
    }
    case 'apart': {
      const { arrive, leave } = gap.issue;
      return itemEl(WARNING, t('bp.dont_meet', { a: name(arrive.doc), b: name(leave.doc) }),
        t('bp.arrives_leaves', { to: arrive.place, from: leave.place }), opens(leave.doc));
    }
    case 'zones': {
      const { place, arrive, leave } = gap.issue;
      const args = { a: name(arrive.doc), arrived: zoneName(arrive.zone), b: name(leave.doc), zone: zoneName(leave.zone) };
      return itemEl(WARNING, place ? t('bp.zones_differ_in', { place }) : t('bp.zones_differ'),
        leave.doc.event.type === 'activity' ? t('bp.zones_is', args) : t('bp.zones_leaves', args),
        opens(leave.doc));
    }
    case 'ticket':
      return itemEl(WARNING, t('bp.no_ticket', { name: name(gap.doc) }), t('bp.booking_ref', { reference: gap.doc.event.booking }), opens(gap.doc));
    default:
      return itemEl(WARNING, name(gap.doc), t('bp.outside_days', { day: dayText(gap.doc.date) }), opens(gap.doc));
  }
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
