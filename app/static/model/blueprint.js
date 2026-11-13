// The shipped blueprints (vault.md#project, blueprints.rs), from the page's
// data block (#blueprints, page.rs): what New project offers, and the block
// a project of one starts with. A project's facts as its header shows them
// (`factsOf`), and what its checks find (`checksOf`): the gaps, the next
// steps and how far it is (features/blueprint.js).

import { $ } from '../core/dom.js';
import { addDays, dayStart, parseDate } from '../core/format.js';
import { entities, projects, projectsOf } from './entities.js';
import { transactionInProject, cents, pairOf } from './bank.js';
import { tripGaps } from './trip.js';

export const BLUEPRINTS = JSON.parse($('blueprints')?.textContent || '[]');

// The checks a trip's route answers (`tripGaps`).
const TRIP_CHECKS = ['nights', 'journeys', 'tickets'];

// The shipped blueprint `name`, if there is one.
function blueprintOf(name) {
  return BLUEPRINTS.find(blueprint => blueprint.name === name) ?? null;
}

// The block a new project of `blueprint` starts with: its name and what it
// takes from it (its checks, its labels), so a release changes no project.
export function newProjectBlock(blueprint) {
  const block = { blueprint: blueprint.name };
  if (blueprint.checks.length) block.checks = blueprint.checks;
  if (blueprint.labels.length) block.labels = blueprint.labels;
  return block;
}

// The block `project` with the shipped `blueprint` put on, as a project
// without one gets it (features/header.js): its name, and the checks and
// labels it brings that the block lacks; what the block has stays.
export function withBlueprint(project, blueprint) {
  const block = newProjectBlock(blueprint);
  const union = (had = [], more = []) => [...new Set([...had, ...more])];
  const next = { ...project, blueprint: block.blueprint };
  if (block.checks) next.checks = union(project.checks, block.checks);
  if (block.labels) next.labels = union(project.labels, block.labels);
  return next;
}

// Whether the project's block has anything for its header to show.
export function hasBlueprint(project) {
  return Boolean(project?.blueprint || Object.keys(project?.facts ?? {}).length
    || project?.documents?.length || project?.payments?.length || project?.checks?.length);
}

// The facts the header shows, `{ name, label, type, value }` each: the
// blueprint's, in its order, set or not; then the project's others, as text.
export function factsOf(project) {
  const facts = project?.facts ?? {};
  const named = blueprintOf(project?.blueprint)?.facts ?? [];
  const known = new Set(named.map(fact => fact.name));
  return [
    ...named.map(fact => ({ ...fact, value: facts[fact.name] ?? null })),
    ...Object.keys(facts).filter(name => !known.has(name))
      .map(name => ({ name, label: name, type: 'text', value: facts[name] })),
  ];
}

// `project` with the fact `name` set to `value`; null or empty removes it.
export function withFact(project, name, value) {
  const facts = { ...(project.facts ?? {}) };
  const empty = value == null || value === '' || (Array.isArray(value) && !value.length);
  if (empty) delete facts[name];
  else facts[name] = value;
  const next = { ...project, facts };
  if (!Object.keys(facts).length) delete next.facts;
  return next;
}

// The calendar the project `id` names (a trip's `calendar` fact), if it is
// still in the vault.
export function projectCalendar(id) {
  const calendar = projects.get(id)?.project?.facts?.calendar;
  return entities.get(calendar)?.kind === 'calendar' ? calendar : null;
}

// The entities in the project `id`.
export function membersOf(id) {
  return [...entities.values()].filter(doc => projectsOf(doc).includes(id));
}

// What the project `id` has spent: its transactions, those linked into it
// (`transactionInProject`), money out less money in, pending ones too;
// transfers between the vault's accounts count for nothing.
function spentIn(id) {
  let sum = 0;
  for (const statement of entities.values()) {
    for (const transaction of statement.statement?.transactions ?? []) {
      if (transactionInProject(transaction, id) && !pairOf(transaction)) sum -= cents(transaction.amount);
    }
  }
  return sum / 100;
}

// Whether a transaction of the vault is linked to the entity `id`.
function paidFor(id) {
  if (!id) return false;
  for (const statement of entities.values()) {
    if ((statement.statement?.transactions ?? []).some(transaction => transaction['linked-to'] === id)) return true;
  }
  return false;
}

// A span (`90d`, `6w`, `3m`, `1y`) back from `day`; null if it isn't one.
function before(day, span) {
  const m = /^(\d+)\s*([dwmy])$/.exec(String(span ?? '').trim());
  if (!m) return null;
  const n = Number(m[1]);
  if (m[2] === 'd') return addDays(day, -n);
  if (m[2] === 'w') return addDays(day, -7 * n);
  const months = m[2] === 'm' ? n : 12 * n;
  return new Date(day.getFullYear(), day.getMonth() - months, day.getDate());
}

const isOpenTask = doc => doc.task && doc.task.status !== 'done';
const byDay = day => item => day(item)?.getTime() ?? Infinity;

// What the checks of the project `id` (its block `project`) find on `today`:
// - `missing`: the gaps by their `rule` -- a trip's `nights`, `journeys` and
//   `tickets` first, in the order of its route (`tripGaps`, model/trip.js);
//   `documents`, an expected one not met or too old (`expected`, its
//   `index` for Find, `stale`); `payments`, one past its day not paid
//   (`payment`, `due`); `budget`, spent beyond it (`spent`, `budget`);
//   `deadlines`, an open task past its due day (`doc`);
// - `next`: the open tasks by their due days, those without one last, then
//   the payments awaited by their days, `{ doc }` or `{ payment, due }`;
// - `progress`: how far it is, by check: a trip's `nights` covered and
//   `journeys` booked, `documents` and `payments` as `[met, all]`, `budget` as
//   `{ spent, budget }`, `steps` as `[done, all]`;
// - `later`: while a trip's days are open, what its sketch will need.
export function checksOf(id, project, today = dayStart(new Date())) {
  const checks = new Set(project?.checks ?? []);
  const missing = [];
  const progress = {};
  const trip = TRIP_CHECKS.some(check => checks.has(check)) ? tripGaps(id, project) : null;
  if (trip) {
    missing.push(...trip.gaps.filter(gap => checks.has(gap.rule)));
    for (const check of ['nights', 'journeys']) {
      if (checks.has(check) && trip.progress[check]?.[1]) progress[check] = trip.progress[check];
    }
  }
  const tasks = membersOf(id).filter(doc => doc.task);
  const open = tasks.filter(isOpenTask);
  const dueOf = doc => (isNaN(doc.due) ? null : doc.due);
  open.sort((a, b) => byDay(dueOf)(a) - byDay(dueOf)(b));

  const documents = project?.documents ?? [];
  if (checks.has('documents') && documents.length) {
    let met = 0;
    documents.forEach((expected, index) => {
      const by = (expected.by ?? []).map(doc_id => entities.get(doc_id)).filter(Boolean);
      const since = expected.within ? before(today, expected.within) : null;
      if (by.some(doc => !since || (doc.date && doc.date >= since))) met++;
      else missing.push({ rule: 'documents', expected, index, stale: by.length > 0 });
    });
    progress.documents = [met, documents.length];
  }

  const awaited = [];
  const payments = project?.payments ?? [];
  if (payments.length) {
    let met = 0;
    for (const payment of payments) {
      if (paidFor(payment.for)) {
        met++;
        continue;
      }
      const due = parseDate(payment.due);
      awaited.push({ payment, due });
      if (checks.has('payments') && due && due < today) missing.push({ rule: 'payments', payment, due });
    }
    if (checks.has('payments')) progress.payments = [met, payments.length];
    awaited.sort((a, b) => byDay(item => item.due)(a) - byDay(item => item.due)(b));
  }

  const budget = Number(project?.facts?.budget);
  if (checks.has('budget') && Number.isFinite(budget) && budget > 0) {
    const spent = spentIn(id);
    if (spent > budget) missing.push({ rule: 'budget', spent, budget });
    progress.budget = { spent, budget };
  }

  if (checks.has('deadlines')) {
    for (const doc of open) {
      if (dueOf(doc) && doc.due < today) missing.push({ rule: 'deadlines', doc });
    }
    if (tasks.length) progress.steps = [tasks.length - open.length, tasks.length];
  }

  return { missing, next: [...open.map(doc => ({ doc })), ...awaited], progress, later: trip?.later ?? null };
}
