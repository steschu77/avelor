// The Bank lens's Recurring view (ui.md#bank): the fixed costs. The
// recurring payments (vault.md#recurring) of the accounts in the scope --
// under a project, those in it -- by category, income first, then the
// costs, the largest first; each its schedule, its amount each time, a
// month's and a year's (`perYear`, model/bank.js), and what is wrong with
// it (missed and off occurrences, `occurrences`). Over them, a month's
// income, fixed costs and what is left, each with a year's, and how many
// occurrences were missed or off as far as the statements go. The ended
// series are apart, in no sum. A row selects its series, its card in the
// panel.

import { accEl } from '../../core/dom.js';
import { DATE_FORMAT, DAY_SHORT_MONTH, parseDate, plural, signedAmount } from '../../core/format.js';
import { amountOn, cents, occurrences, perYear, seriesEnded, seriesIn, seriesInProject, statementsEnd } from '../../model/bank.js';
import { KIND_ICONS, iconEl } from '../../ui/icons.js';
import { stat, stats } from '../../ui/page.js';
import { repeatsText } from '../panel/details.js';

// The series of `accounts` (entities) in the project `selected_project`
// (all without one; none for No project): the running ones and the ended
// ones, each `{ series, amount, month, year, occurrences }` -- the amount
// as of today and its sums, null for a schedule the page doesn't read; a
// running one's occurrences (`occurrences`).
export function recurringOf(accounts, selected_project) {
  const today = new Date();
  const ids = new Set(accounts.map(doc => doc.id));
  const current = [];
  const ended = [];
  for (const series of seriesIn(ids)) {
    if (!seriesInProject(series, selected_project)) continue;
    const amount = amountOn(series, today) ?? 0;
    const times = perYear(series);
    const entry = { series, amount, year: times == null ? null : amount * times, month: times == null ? null : (amount * times) / 12 };
    if (seriesEnded(series)) ended.push(entry);
    else current.push({ ...entry, occurrences: occurrences(series) });
  }
  return { current, ended };
}

// The view: the sums, then the series by category and the ended ones; or a
// note while there are none.
export function renderRecurring({ current, ended }, accounts) {
  if (!current.length && !ended.length) {
    return [accEl('p', 'note bank__none', 'No recurring payment here yet: make one from a booking\u2019s card, its Series row.')];
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
  const wrong = Object.entries(flagged).filter(([, n]) => n).map(([state, n]) => `${n} ${state}`);
  const parts = [stats(
    stat('Income', accEl('span', 'figure', signedAmount(plus)), `a month \u00b7 ${signedAmount(plus * 12)} a year \u00b7 ${plural(income.length, 'series', 'series')}`),
    stat('Fixed costs', accEl('span', 'figure', signedAmount(minus)), `a month \u00b7 ${signedAmount(minus * 12)} a year \u00b7 ${plural(costs.length, 'series', 'series')}`),
    stat('Left', accEl('span', 'figure', signedAmount(plus + minus)), `a month \u00b7 ${signedAmount((plus + minus) * 12)} a year`),
    stat(end ? `To ${DAY_SHORT_MONTH.format(end)}` : 'Occurrences', accEl('span', 'figure', wrong.join(' \u00b7 ') || 'All paid'),
      'as far as the statements go'),
  )];

  const table = accEl('div', 'table bank series');
  const head = accEl('div', 'list-row table__head');
  head.append(accEl('span', 'label', 'Series'), accEl('span', 'label', 'Schedule'),
    accEl('span', 'label bank__head-end', 'Each'), accEl('span', 'label bank__head-end', 'A month'),
    accEl('span', 'label bank__head-end', 'A year'));
  const body = accEl('div', 'table__body');
  for (const [category, entries] of byCategory(current)) {
    const month = sum(entries);
    body.append(groupRow(category, entries.length, signedAmount(month), signedAmount(month * 12)),
      ...entries.map(entry => seriesRow(entry, false)));
  }
  if (ended.length) body.append(groupRow('Ended', ended.length, '', ''), ...ended.map(entry => seriesRow(entry, true)));
  table.append(head, body);
  const note = accEl('div', 'lens-hidden');
  note.append(accEl('span', 'note', 'A series\u2019 amount a month and a year, as its schedule repeats; an ended one counts in no sum. '
    + `Occurrences are judged as far as the statements go${end ? `: to ${DATE_FORMAT.format(end)}` : ''}.`));
  table.append(note);
  parts.push(table);
  return parts;
}

// The running series by category: income first, then the costs, the
// largest a month first; without a category, Other last.
function byCategory(entries) {
  const groups = new Map();
  for (const entry of entries) {
    const name = entry.series.recurring.category?.trim() || '';
    if (!groups.has(name)) groups.set(name, []);
    groups.get(name).push(entry);
  }
  const month = list => list.reduce((n, entry) => n + (entry.month ?? 0), 0);
  return [...groups].map(([name, list]) => [name ? name[0].toUpperCase() + name.slice(1) : 'Other', list, name])
    .sort((a, b) => (!a[2]) - (!b[2]) || (month(b[1]) > 0) - (month(a[1]) > 0) || month(a[1]) - month(b[1]))
    .map(([label, list]) => [label, list.sort((a, b) => (a.month ?? 0) - (b.month ?? 0))]);
}

function groupRow(name, count, month, year) {
  const row = accEl('div', 'list-row group-row');
  const label = accEl('span', 'bank__group-name');
  label.append(accEl('span', 'group-row__name', name), accEl('span', 'meta', plural(count, 'series', 'series')));
  row.append(label, accEl('span'), accEl('span', 'bank__net', month), accEl('span', 'bank__net', year));
  return row;
}

// A series: its icon and title, what is wrong with it, its account, its
// schedule (and since when, or when it ended), its amount each time, a
// month's and a year's.
function seriesRow({ series, amount, month, year, occurrences: list = [] }, ended) {
  const recurring = series.recurring;
  const row = accEl('div', `list-row table__row bank__row${ended ? ' series__row--ended' : ''}`);
  row.tabIndex = 0;
  row.setAttribute('role', 'button');
  row.dataset.statement = series.id;
  row.dataset.series = '';
  row.title = `Open ${series.title || 'Untitled'}`;
  const cell = accEl('span', 'bank__cell');
  cell.append(iconEl(KIND_ICONS.recurring.icon, 'table__icon'), accEl('span', 'row-title', series.title || 'Untitled'));
  if (!ended) {
    for (const state of ['missed', 'off']) {
      const days = list.filter(occurrence => occurrence.state === state).map(occurrence => DAY_SHORT_MONTH.format(occurrence.day));
      if (!days.length) continue;
      const tag = accEl('span', 'chip chip--danger', `${days.length} ${state}`);
      tag.title = `${state === 'off' ? 'Off the amount' : 'Missed'}: ${days.join(', ')}`;
      cell.append(tag);
    }
  }
  const detail = [recurring.party && recurring.party !== series.title ? recurring.party : null, recurring.category && ended ? recurring.category : null]
    .filter(Boolean).join(' \u00b7 ');
  if (detail) cell.append(accEl('span', 'meta bank__detail', detail));
  const from = parseDate(recurring.from);
  const until = parseDate(recurring.until);
  const when = ended && until ? `ended ${DATE_FORMAT.format(until)}` : from > new Date() ? `from ${DATE_FORMAT.format(from)}` : '';
  const schedule = accEl('span', 'series__schedule', repeatsText(recurring.every ?? []));
  if (when) schedule.append(' ', accEl('span', 'meta', `\u00b7 ${when}`));
  const sumEl = value => accEl('span', `bank__amount${amount > 0 ? ' bank__amount--in' : ''}`, value);
  row.append(cell, schedule, sumEl(signedAmount(amount)),
    sumEl(ended || month == null ? '\u2014' : signedAmount(month)),
    sumEl(ended || year == null ? '\u2014' : signedAmount(year)));
  return row;
}
