// The Bank lens's Recurring view (ui.md#bank): the fixed costs. The
// recurring payments (vault.md#recurring) of the accounts in the scope --
// under a project, those in it -- by category, income first, then the
// costs, the largest first; each its schedule, its amount each time, a
// month's and a year's (`perYear`, model/bank.js), and what is wrong with
// it (missed and off occurrences, `occurrences`). Over them, a month's
// income, fixed costs and what is left, each with a year's, and how many
// occurrences were missed or off as far as the statements go. The ended
// recurring payments are apart, in no sum. A row selects its recurring payment, its card in the
// panel.

import { accEl } from '../../core/dom.js';
import { pressable } from '../../core/dom.js';
import { DATE_FORMAT, DAY_SHORT_MONTH, parseDate, signedAmount } from '../../core/format.js';
import { t } from '../../core/text.js';
import { NO_PROJECT, projectsOf } from '../../model/entities.js';
import { amountOn, cents, occurrences, perYear, recurringPaymentEnded, recurringPaymentsIn, statementsEnd } from '../../model/bank.js';
import { KIND_ICONS, iconEl } from '../../ui/icons.js';
import { stat, stats } from '../../ui/page.js';
import { scheduleText } from '../panel/details.js';

// The recurring payments of `accounts` (entities) in the project `selected_project`
// (all without one; none for No project): the running ones and the ended
// ones, each `{ recurringPayment, amount, month, year, occurrences }` -- the amount
// as of today and its sums, null for a schedule the page doesn't read; a
// running one's occurrences (`occurrences`).
export function recurringOf(accounts, selected_project) {
  const today = new Date();
  const ids = new Set(accounts.map(doc => doc.id));
  const inProject = recurringPayment => !selected_project
    || (selected_project === NO_PROJECT ? !projectsOf(recurringPayment).length : projectsOf(recurringPayment).includes(selected_project));
  const current = [];
  const ended = [];
  for (const recurringPayment of recurringPaymentsIn(ids)) {
    if (!inProject(recurringPayment)) continue;
    const amount = amountOn(recurringPayment, today) ?? 0;
    const times = perYear(recurringPayment);
    const entry = { recurringPayment, amount, year: times == null ? null : amount * times, month: times == null ? null : (amount * times) / 12 };
    if (recurringPaymentEnded(recurringPayment)) ended.push(entry);
    else current.push({ ...entry, occurrences: occurrences(recurringPayment) });
  }
  return { current, ended };
}

// The view: the sums, then the recurring payments by category and the ended ones; or a
// note while there are none.
export function renderRecurring({ current, ended }, accounts) {
  if (!current.length && !ended.length) {
    return [accEl('p', 'note bank__none', t('recurring.none'))];
  }
  const sum = list => list.reduce((n, entry) => n + cents(entry.month), 0) / 100;
  const income = current.filter(entry => entry.amount > 0);
  const costs = current.filter(entry => entry.amount <= 0);
  const [plus, minus] = [sum(income), sum(costs)];
  const ends = accounts.map(doc => statementsEnd(doc.id)).filter(Boolean);
  const end = ends.length ? new Date(Math.max(...ends)) : null;
  const flagged = { missed: 0, off: 0 };
  for (const entry of current) {
    for (const occurrence of entry.occurrences) {
      if (occurrence.state in flagged) flagged[occurrence.state]++;
    }
  }
  const wrong = Object.entries(flagged).filter(([, n]) => n)
    .map(([state, n]) => state === 'off' ? t('occurrence.count_off', { n }) : t('occurrence.count_missed', { n }));
  const parts = [stats(
    stat(t('recurring.income'), accEl('span', 'figure', signedAmount(plus)), t('recurring.month_year_count',
      { year: signedAmount(plus * 12), recurring: t('count.recurring_payments', { n: income.length }) })),
    stat(t('recurring.fixed_costs'), accEl('span', 'figure', signedAmount(minus)), t('recurring.month_year_count',
      { year: signedAmount(minus * 12), recurring: t('count.recurring_payments', { n: costs.length }) })),
    stat(t('recurring.left'), accEl('span', 'figure', signedAmount(plus + minus)),
      t('recurring.month_year', { year: signedAmount((plus + minus) * 12) })),
    stat(end ? t('recurring.to', { day: DAY_SHORT_MONTH.format(end) }) : t('occurrence.head'),
      accEl('span', 'figure', wrong.join(' \u00b7 ') || t('recurring.all_paid')), t('recurring.as_far')),
  )];

  const table = accEl('div', 'table bank recurring-payments');
  const head = accEl('div', 'list-row table__head');
  head.append(accEl('span', 'label', t('props.recurring_payment')), accEl('span', 'label', t('props.schedule')),
    accEl('span', 'label bank__head-end', t('recurring.each')), accEl('span', 'label bank__head-end', t('recurring.a_month_head')),
    accEl('span', 'label bank__head-end', t('recurring.a_year_head')));
  const body = accEl('div', 'table__body');
  for (const [category, entries] of byCategory(current)) {
    const month = sum(entries);
    body.append(groupRow(category, entries.length, signedAmount(month), signedAmount(month * 12)),
      ...entries.map(entry => recurringPaymentRow(entry, false)));
  }
  if (ended.length) body.append(groupRow(t('recurring.ended'), ended.length, '', ''), ...ended.map(entry => recurringPaymentRow(entry, true)));
  table.append(head, body);
  const note = accEl('div', 'lens-hidden');
  const judged = end ? t('recurring.judged_to', { day: DATE_FORMAT.format(end) }) : t('recurring.judged');
  note.append(accEl('span', 'note', `${t('recurring.note')} ${judged}`));
  table.append(note);
  parts.push(table);
  return parts;
}

// The running recurring payments by category: income first, then the costs, the
// largest a month first; without a category, Other last.
function byCategory(entries) {
  const groups = new Map();
  for (const entry of entries) {
    const name = entry.recurringPayment['recurring-payment'].category?.trim() || '';
    if (!groups.has(name)) groups.set(name, []);
    groups.get(name).push(entry);
  }
  const month = list => list.reduce((n, entry) => n + (entry.month ?? 0), 0);
  return [...groups].map(([name, list]) => [name ? name[0].toUpperCase() + name.slice(1) : t('recurring.other'), list, name])
    .sort((a, b) => (!a[2]) - (!b[2]) || (month(b[1]) > 0) - (month(a[1]) > 0) || month(a[1]) - month(b[1]))
    .map(([label, list]) => [label, list.sort((a, b) => (a.month ?? 0) - (b.month ?? 0))]);
}

function groupRow(name, count, month, year) {
  const row = accEl('div', 'list-row group-row');
  const label = accEl('span', 'bank__group-name');
  label.append(accEl('span', 'group-row__name', name), accEl('span', 'meta', t('count.recurring_payments', { n: count })));
  row.append(label, accEl('span'), accEl('span', 'bank__net', month), accEl('span', 'bank__net', year));
  return row;
}

// A recurring payment: its icon and title, what is wrong with it, its account, its
// schedule (and since when, or when it ended), its amount each time, a
// month's and a year's.
function recurringPaymentRow({ recurringPayment, amount, month, year, occurrences: list = [] }, ended) {
  const recurring = recurringPayment['recurring-payment'];
  const row = pressable(accEl('div', `list-row table__row bank__row${ended ? ' recurring-payments__row--ended' : ''}`));
  row.dataset.statement = recurringPayment.id;
  row.dataset.recurringPayment = '';
  row.title = t('open.named', { name: recurringPayment.title || t('untitled') });
  const cell = accEl('span', 'bank__cell');
  cell.append(iconEl(KIND_ICONS['recurring-payment'].icon, 'table__icon'), accEl('span', 'row-title', recurringPayment.title || t('untitled')));
  if (!ended) {
    for (const state of ['missed', 'off']) {
      const days = list.filter(occurrence => occurrence.state === state).map(occurrence => DAY_SHORT_MONTH.format(occurrence.day));
      if (!days.length) continue;
      const off = state === 'off';
      const n = days.length;
      const tag = accEl('span', 'chip chip--danger', off ? t('occurrence.count_off', { n }) : t('occurrence.count_missed', { n }));
      tag.title = off ? t('recurring.off_days', { days: days.join(', ') }) : t('recurring.missed_days', { days: days.join(', ') });
      cell.append(tag);
    }
  }
  const detail = [recurring.party && recurring.party !== recurringPayment.title ? recurring.party : null, recurring.category && ended ? recurring.category : null]
    .filter(Boolean).join(' \u00b7 ');
  if (detail) cell.append(accEl('span', 'meta bank__detail', detail));
  const from = parseDate(recurring.from);
  const until = parseDate(recurring.until);
  const when = ended && until ? t('recurring.ended_on', { day: DATE_FORMAT.format(until) })
    : from > new Date() ? t('recurring.from', { day: DATE_FORMAT.format(from) }) : '';
  const schedule = accEl('span', 'recurring-payments__schedule', scheduleText(recurring.schedule ?? []));
  if (when) schedule.append(' ', accEl('span', 'meta', `\u00b7 ${when}`));
  const sumEl = value => accEl('span', `bank__amount${amount > 0 ? ' bank__amount--in' : ''}`, value);
  row.append(cell, schedule, sumEl(signedAmount(amount)),
    sumEl(ended || month == null ? '\u2014' : signedAmount(month)),
    sumEl(ended || year == null ? '\u2014' : signedAmount(year)));
  return row;
}
