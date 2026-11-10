// Bank accounts and their statements (vault.md, Bank Account and
// Transactions): an account's statements (`statementsOf`) and its ledger,
// the booked bookings with the balance after each (`ledger`); a booking's
// party (`partyOf`), the other half of a transfer (`pairOf`), its name
// (`bookingName`) and its projects through its link (`linkProjects`); the
// accounts and statements in the vault panel's filter (`bankScope`); the
// recurring payments (vault.md#recurring), their bookings and occurrences,
// and what they are expected to pay or bring (`expectedOf`) by the end of
// next month (`horizonOf`). The Bank lens (features/lenses/bank.js), a
// booking's card (features/panel/booking.js), a series' card
// (features/panel/series.js) and a statements watch's pane
// (features/workspace/folders.js) show them.

import { dayStart, daysBetween, localDay, parseDate } from '../core/format.js';
import { addDays, periodOf, recurrenceStarts, timesAYear } from './calendar.js';
import {
  NO_PROJECT, entities, matchesFilter, parseContact, projects, projectsOf, threadProjects, threads, watchSourceKey,
  watches,
} from './entities.js';

// The statements of the bank account `id`: the transactions entities
// naming it.
export function statementsOf(id) {
  return [...entities.values()].filter(doc => doc.transactions?.account === id);
}

// A bank account's kinds (`bank-account.type`), in words.
export const ACCOUNT_KINDS = [['checking', 'Checking'], ['savings', 'Savings'], ['credit-card', 'Credit card']];

// A booking's kinds (`import.md#statements`), in words.
export const BOOKING_KINDS = new Map([
  ['card', 'Card payment'], ['direct-debit', 'Direct debit'], ['transfer', 'Transfer'],
  ['cash', 'Cash'], ['fee', 'Fee'],
]);

// The booking `id` of the statement `statement` (its entity), if it has it.
export function bookingOf(statement, id) {
  return statement?.transactions?.transactions?.find(booking => booking.id === id) ?? null;
}

// The other side of `booking` as the vault keeps it (a contact reference):
// who was paid for money out, who paid for money in, as the import writes
// them.
export function partyRef(booking) {
  return booking.amount < 0 ? booking.receiver : booking.sender;
}

// Who paid or was paid: the other side, else the booking's title.
function partyOf(booking) {
  return partyRef(booking) || booking.title || 'Booking';
}

// The other half of `booking`, a transfer between two of the vault's
// accounts (its `transfer`, import.md#statements): `{ statement, booking,
// account }`, or null while no booking names it back.
export function pairOf(booking) {
  const id = booking?.transfer;
  if (!id) return null;
  for (const statement of entities.values()) {
    const other = statement.transactions?.transactions?.find(item => item.id === id);
    if (!other) continue;
    if (other.transfer !== booking.id) return null;
    return { statement, booking: other, account: entities.get(statement.transactions.account) ?? null };
  }
  return null;
}

// What a booking is called: a transfer after its other half's account, `pair`
// ("To House fund", "From Checking"), else who paid or was paid.
export function bookingName(booking, pair) {
  if (!pair) return partyOf(booking);
  return `${booking.amount > 0 ? 'From' : 'To'} ${pair.account?.title || 'Untitled'}`;
}

// The projects of `booking` through its link: the project it links to, or
// the projects of the entity or thread it links to; without a link, its
// series' projects.
export function linkProjects(booking) {
  const to = booking?.['linked-to'];
  if (!to) {
    const series = seriesOf(booking);
    return series ? projectsOf(series) : [];
  }
  if (projects.has(to)) return [to];
  if (threads.has(to)) return threadProjects(to);
  const doc = entities.get(to);
  return doc ? projectsOf(doc) : [];
}

// Whether `booking` is in the project `selected_project`, through its link
// (`linkProjects`): for No project, it has none that way; without a
// project, every booking.
export function bookingInProject(booking, selected_project) {
  if (!selected_project) return true;
  const ids = linkProjects(booking);
  return selected_project === NO_PROJECT ? !ids.length : ids.includes(selected_project);
}

// Whether the series `series` is in the project `selected_project`, by its
// own projects: for No project, it has none; without a project, every one.
export function seriesInProject(series, selected_project) {
  if (!selected_project) return true;
  const ids = projectsOf(series);
  return selected_project === NO_PROJECT ? !ids.length : ids.includes(selected_project);
}

// The bank accounts and statements in the vault panel's filter `state`:
// `{ accounts, statements }`, the statements a Set. Under a project, or No
// project, the statements are filtered without it: their bookings are in it
// through their links (`bookingInProject`). With nothing narrowing the
// scope, every bank account; else the accounts of the statements in it,
// and the account of a statements watch picked as the source.
export function bankScope(state) {
  const filter = state.selected_project ? { ...state, selected_project: null } : state;
  const statements = new Set();
  const all = [];
  for (const doc of entities.values()) {
    if (doc.kind === 'bank-account') all.push(doc);
    else if (doc.transactions && matchesFilter(doc, filter)) statements.add(doc);
  }
  const narrowed = state.selected_project || state.selected_source || state.selected_labels.length
    || state.word_filter;
  if (!narrowed) return { accounts: all, statements };
  const ids = new Set([...statements].map(doc => doc.transactions.account));
  for (const watch of watches.values()) {
    if (watch.account && watchSourceKey(watch.id) === state.selected_source) ids.add(watch.account);
  }
  return { accounts: all.filter(doc => ids.has(doc.id)), statements };
}

// Money in cents while it is summed, so the sums don't drift.
export const cents = amount => Math.round((amount ?? 0) * 100);
const units = n => (n == null ? null : n / 100);

// The date of the newest booking of the bank account `id`'s statements,
// pending ones too: how far its statements go; null without any.
export function statementsEnd(id) {
  let end = null;
  if (!id) return end;
  for (const statement of statementsOf(id)) {
    for (const booking of statement.transactions.transactions ?? []) {
      const date = parseDate(booking.date);
      if (date && (!end || date > end)) end = date;
    }
  }
  return end;
}

// The ledger of the bank account `account` (its entity), from all its
// statements:
// - `entries`: its booked bookings, oldest first (by day, then as their
//   statement lists them), each `{ booking, statement, time, balance }`,
//   `time` its day's or instant's ms, `balance` the account's after it
//   (null while nothing gives one);
// - `opening`: the balance before the first, `balance` after the last;
// - `pending`: the pending bookings, newest first, and `pendingSum`.
// The balances are walked back from the account's `balance`; a statement
// with a `balance` of its own sets it again at its last booking, the newest
// statement's first. Pending bookings have none.
export function ledger(account) {
  const booked = [];
  const pending = [];
  for (const statement of statementsOf(account.id)) {
    (statement.transactions.transactions ?? []).forEach((booking, index) => {
      const entry = { booking, statement, index, time: parseDate(booking.date)?.getTime() ?? 0, balance: null };
      (booking.status === 'pending' ? pending : booked).push(entry);
    });
  }
  const order = (a, b) => a.time - b.time
    || (a.statement.date?.getTime() ?? 0) - (b.statement.date?.getTime() ?? 0)
    || a.index - b.index;
  booked.sort(order);
  pending.sort((a, b) => order(b, a));

  const own = account['bank-account']?.balance;
  let balance = own == null ? null : cents(own);
  const anchored = new Set();
  for (let i = booked.length - 1; i >= 0; i--) {
    const { statement } = booked[i];
    if (statement.transactions.balance != null && !anchored.has(statement)) {
      anchored.add(statement);
      balance = cents(statement.transactions.balance);
    }
    booked[i].balance = units(balance);
    if (balance != null) balance -= cents(booked[i].booking.amount);
  }

  return {
    entries: booked,
    opening: booked.length ? units(balance) : null,
    balance: booked.length ? booked.at(-1).balance : own ?? null,
    pending,
    pendingSum: units(pending.reduce((sum, entry) => sum + cents(entry.booking.amount), 0)),
  };
}

// ----------------------------------------------------------------------------
// Recurring payments (vault.md#recurring): a series of a bank account, the
// bookings naming it (`recurring`), and its occurrences as far as the
// account's statements go, by the rule the import puts bookings into their
// series by (app/src/recurring.rs).

// How many days from an occurrence a booking still pays it; half the gap
// to the next occurrence when that is less.
const PAY_DAYS = 7;
// How far past the statements the occurrences are read, so the last one's
// gap to the next is known.
const AHEAD_DAYS = 62;

// The categories a series is offered; the vault's own are offered too.
export const CATEGORIES = ['housing', 'insurance', 'utilities', 'phone', 'subscriptions', 'transport', 'loans', 'income'];

// The day a booking was booked (a pending one's card day), at its start.
export function bookingDay(booking) {
  const date = parseDate(booking.date);
  return date ? dayStart(date) : null;
}

// The name of a booking's other side (`partyRef`), '' without one.
export function partyName(booking) {
  const ref = partyRef(booking);
  return ref ? parseContact(ref).name : '';
}

// The series `booking` belongs to (its `recurring`), if it is in the vault.
export function seriesOf(booking) {
  const series = entities.get(booking?.recurring);
  return series?.recurring ? series : null;
}

// The series of the bank accounts `ids` (a Set), or of all of them.
export function seriesIn(ids = null) {
  return [...entities.values()].filter(doc => doc.recurring && (!ids || ids.has(doc.recurring.account)));
}

// The bookings naming the series `id`, oldest first: `{ statement, booking,
// day }` each.
function bookingsOf(id) {
  const found = [];
  const account = entities.get(id)?.recurring?.account;
  if (!account) return found;
  for (const statement of statementsOf(account)) {
    for (const booking of statement.transactions.transactions ?? []) {
      const day = bookingDay(booking);
      if (booking.recurring === id && day) found.push({ statement, booking, day });
    }
  }
  return found.sort((a, b) => a.day - b.day);
}

// A series' amounts (its role `recurring`'s), oldest first.
export function amountsOf(recurring) {
  return [...(recurring?.amounts ?? [])].sort((a, b) => (parseDate(a.from) ?? 0) - (parseDate(b.from) ?? 0));
}

// The role `recurring` with `amount` from the day `from` (yyyy-mm-dd) on, in
// place of one from that day.
export function withAmount(recurring, from, amount) {
  const kept = amountsOf(recurring).filter(entry => localDay(parseDate(entry.from)) !== from);
  return { ...recurring, amounts: amountsOf({ amounts: [...kept, { from, amount }] }) };
}

// The series' amount on `day`: its last amount from that day or before,
// else its first; null without any.
export function amountOn(series, day) {
  const amounts = amountsOf(series.recurring);
  const current = amounts.filter(entry => parseDate(entry.from) <= day).at(-1) ?? amounts[0];
  return current?.amount ?? null;
}

// The names the series' party goes by, lowercase: its party's and those of
// its bookings (a name added by hand is known from then on).
export function partyNames(series) {
  const names = new Set();
  const add = ref => {
    if (ref) names.add(parseContact(ref).name.trim().toLowerCase());
  };
  add(series.recurring?.party);
  for (const { booking } of bookingsOf(series.id)) add(partyRef(booking));
  return names;
}

// Whether the names `a` and `b` (lowercase) are similar: one starts with the
// other, as an export may name a party anew ("StreamFlix", "StreamFlix Inc.").
export function similarNames(a, b) {
  return !!a && !!b && a !== b && (a.startsWith(b) || b.startsWith(a));
}

// The days of the series' schedule from its first occurrence on, before
// `before`, and not after its `until`.
function seriesDays(series, before) {
  const { every = [], from, until } = series.recurring ?? {};
  const first = parseDate(from);
  if (!first) return [];
  const last = until ? parseDate(until) : null;
  return recurrenceStarts(first, every, before).filter(day => !last || day <= last);
}

// The occurrences of `series` as far as its account's statements go, newest
// first: `{ day, state, booking, statement }` each, `state` one of
// - `paid`: a booking of the series is within its reach (`PAY_DAYS`, or
//   half the gap to the next occurrence), the nearest unpaid one taking it,
//   the bookings oldest first;
// - `off`: paid by a booking more than `vary` off the amount on its day;
// - `missed`: none, and the statements go `PAY_DAYS` past it;
// - `extra`: a booking of the series that pays none, on its own day.
// An occurrence the statements don't go far enough past is left out.
export function occurrences(series) {
  const end = statementsEnd(series.recurring?.account);
  if (!end) return [];
  const days = seriesDays(series, addDays(end, AHEAD_DAYS));
  const reach = i => {
    const before = i > 0 ? daysBetween(days[i - 1], days[i]) : Infinity;
    const after = i + 1 < days.length ? daysBetween(days[i], days[i + 1]) : Infinity;
    return Math.min(PAY_DAYS, Math.floor(Math.min(before, after) / 2));
  };
  const paid = new Map(); // index in `days` -> the booking paying it
  const out = [];
  for (const entry of bookingsOf(series.id)) {
    const apart = i => Math.abs(daysBetween(days[i], entry.day));
    let best = -1;
    days.forEach((_, i) => {
      if (paid.has(i) || apart(i) > reach(i)) return;
      if (best < 0 || apart(i) < apart(best)) best = i;
    });
    if (best < 0) out.push({ day: entry.day, state: 'extra', booking: entry.booking, statement: entry.statement });
    else paid.set(best, entry);
  }
  const vary = cents(series.recurring?.vary ?? 0);
  days.forEach((day, i) => {
    const entry = paid.get(i);
    if (entry) {
      const expected = amountOn(series, day);
      const off = expected != null && Math.abs(cents(entry.booking.amount) - cents(expected)) > vary;
      out.push({ day, state: off ? 'off' : 'paid', booking: entry.booking, statement: entry.statement });
    } else if (daysBetween(day, end) >= PAY_DAYS) {
      out.push({ day, state: 'missed', booking: null, statement: null });
    }
  });
  return out.sort((a, b) => b.day - a.day);
}

// The occurrences of `series` no booking pays yet (pending ones included),
// to `until`, oldest first: `{ series, day, amount }` each, the amount as
// on its day (`amountOn`). One the statements go `PAY_DAYS` past is missed
// instead (`occurrences`); without statements, they start today.
function expectedOf(series, until) {
  const end = statementsEnd(series.recurring?.account);
  const paid = new Set(occurrences(series).filter(o => o.state !== 'extra').map(o => o.day.getTime()));
  const from = end ? addDays(dayStart(end), 1 - PAY_DAYS) : dayStart(new Date());
  return seriesDays(series, addDays(dayStart(until), 1))
    .filter(day => day >= from && !paid.has(day.getTime()))
    .map(day => ({ series, day, amount: amountOn(series, day) ?? 0 }));
}

// What the series `list` are expected to pay or bring to `until`
// (`expectedOf`), by day.
export function expectedIn(list, until) {
  return list.flatMap(series => expectedOf(series, until)).sort((a, b) => a.day - b.day);
}

// The series' next occurrence after its account's statements (after today
// without any) that no booking pays yet; null when it has ended.
export function nextOccurrence(series) {
  const after = statementsEnd(series.recurring?.account) ?? dayStart(new Date());
  const until = new Date(after.getFullYear() + 2, after.getMonth(), after.getDate());
  return expectedOf(series, until).find(entry => entry.day > after)?.day ?? null;
}

// The day the payments expected are counted to: the end of the month after
// the one holding the later of `today` and the day after the newest
// statement of the bank accounts `accounts` (entities).
export function horizonOf(accounts, today = dayStart(new Date())) {
  let from = today;
  for (const account of accounts) {
    const end = statementsEnd(account.id);
    if (end && addDays(dayStart(end), 1) > from) from = addDays(dayStart(end), 1);
  }
  return new Date(from.getFullYear(), from.getMonth() + 2, 0);
}

// An account's balance once what is pending is booked and the payments
// `expected` (`expectedOf`, its series') are made: its ledger `book`'s
// balance with both; null without a balance.
export function balanceOn(book, expected) {
  if (book.balance == null) return null;
  const ahead = expected.reduce((sum, entry) => sum + cents(entry.amount), 0);
  return units(cents(book.balance) + cents(book.pendingSum) + ahead);
}

// Whether the series has ended: its `until` before `today`.
export function seriesEnded(series, today = dayStart(new Date())) {
  const until = parseDate(series.recurring?.until);
  return !!until && until < today;
}

// How many times a year the series' schedule repeats; null for one the page
// doesn't read.
export function perYear(series) {
  return timesAYear(series.recurring?.every ?? []);
}

// How often the series repeats, in a word: "monthly", "twice a month", "4
// times a year".
export function howOften(series) {
  const word = { week: 'weekly', month: 'monthly', quarter: 'quarterly', half: 'half-yearly', year: 'yearly' };
  const period = periodOf(series.recurring?.every)?.period;
  if (period) return word[period];
  const times = perYear(series);
  if (times === 24) return 'twice a month';
  return times ? `${Math.round(times * 10) / 10} times a year` : 'on its schedule';
}
