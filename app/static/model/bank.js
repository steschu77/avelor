// Bank accounts and their statements (vault.md, Bank Account and
// Statement): an account's statements (`statementsOf`) and its ledger,
// the posted transactions with the balance after each (`ledger`); a transaction's
// party (`partyOf`), the other half of a transfer (`pairOf`), its name
// (`transactionName`) and its projects through its link (`linkProjects`); the
// accounts and statements in the vault panel's filter (`bankScope`); the
// recurring payments (vault.md#recurring), their transactions and occurrences,
// and what they are expected to pay or bring (`expectedOf`) by the end of
// next month (`horizonOf`). The Bank lens (features/lenses/bank.js), a
// transaction's card (features/panel/transaction.js), a recurring payment's card
// (features/panel/recurring-payment.js) and a statements watch's pane
// (features/workspace/folders.js) show them.

import { addDays, dayStart, daysBetween, localDay, parseDate } from '../core/format.js';
import { t } from '../core/text.js';
import { periodOf, scheduleStarts, timesAYear } from './schedule.js';
import {
  NO_PROJECT, entities, matchesFilter, parseContact, projects, projectsOf, threadProjects, threads, watchSourceKey,
  watches,
} from './entities.js';

// The statements of the bank account `id`: the statement entities
// naming it.
export function statementsOf(id) {
  return [...entities.values()].filter(doc => doc.statement?.account === id);
}

// A bank account's types (`bank-account.type`), in words.
export const ACCOUNT_TYPES = [
  ['checking', t('account_type.checking')], ['savings', t('account_type.savings')], ['credit-card', t('account_type.credit_card')],
];

// A transaction's types (`import.md#statements`), in words.
export const TRANSACTION_TYPES = new Map([
  ['card', t('transaction_type.card')], ['direct-debit', t('transaction_type.direct_debit')], ['transfer', t('transaction_type.transfer')],
  ['cash', t('transaction_type.cash')], ['fee', t('transaction_type.fee')],
]);

// The transaction `id` of the statement `statement` (its entity), if it has it.
export function transactionOf(statement, id) {
  return statement?.statement?.transactions?.find(transaction => transaction.id === id) ?? null;
}

// The other side of `transaction` as the vault keeps it (a contact reference):
// who was paid for money out, who paid for money in, as the import writes
// them.
export function partyRef(transaction) {
  return transaction.amount < 0 ? transaction.receiver : transaction.sender;
}

// Who paid or was paid: the other side, else the transaction's title.
function partyOf(transaction) {
  return partyRef(transaction) || transaction.title || t('bank.transaction');
}

// The other half of `transaction`, a transfer between two of the vault's
// accounts (its `transfer`, import.md#statements): `{ statement, transaction,
// account }`, or null while no transaction names it back.
export function pairOf(transaction) {
  const id = transaction?.transfer;
  if (!id) return null;
  for (const statement of entities.values()) {
    const other = statement.statement?.transactions?.find(item => item.id === id);
    if (!other) continue;
    if (other.transfer !== transaction.id) return null;
    return { statement, transaction: other, account: entities.get(statement.statement.account) ?? null };
  }
  return null;
}

// The transactions `transaction` of the statement `statement` may pair with by hand
// as a transfer's other half (`pair_transfer`): posted and no half yet, in
// another of the vault's accounts, of the opposite amount, within PAIR_DAYS
// of its day; `{ statement, transaction, account }` each, the nearest first.
const PAIR_DAYS = 14;

export function pairCandidates(statement, transaction) {
  const day = transactionDay(transaction);
  const amount = cents(transaction.amount);
  const found = [];
  if (!day || !amount) return found;
  for (const other of entities.values()) {
    const account = other.statement?.account;
    if (!account || account === statement.statement.account) continue;
    for (const item of other.statement.transactions ?? []) {
      const at = transactionDay(item);
      if (item.transfer || item.status === 'pending' || cents(item.amount) !== -amount || !at) continue;
      const days = Math.abs(daysBetween(day, at));
      if (days <= PAIR_DAYS) found.push({ statement: other, transaction: item, account: entities.get(account) ?? null, days });
    }
  }
  return found.sort((a, b) => a.days - b.days);
}

// What a transaction is called: a transfer after its other half's account, `pair`
// ("To House fund", "From Checking"), else who paid or was paid.
export function transactionName(transaction, pair) {
  if (!pair) return partyOf(transaction);
  const name = pair.account?.title || t('untitled');
  return transaction.amount > 0 ? t('transaction.from_account', { name }) : t('transaction.to_account', { name });
}

// The projects of `transaction` through its link: the project it links to, or
// the projects of the entity or thread it links to; without a link, its
// recurring payments' projects.
export function linkProjects(transaction) {
  const to = transaction?.['linked-to'];
  if (!to) {
    const recurringPayment = recurringPaymentOf(transaction);
    return recurringPayment ? projectsOf(recurringPayment) : [];
  }
  if (projects.has(to)) return [to];
  if (threads.has(to)) return threadProjects(to);
  const doc = entities.get(to);
  return doc ? projectsOf(doc) : [];
}

// Whether `transaction` is in the project `selected_project`, through its link
// (`linkProjects`): for No project, it has none that way; without a
// project, every transaction.
export function transactionInProject(transaction, selected_project) {
  if (!selected_project) return true;
  const ids = linkProjects(transaction);
  return selected_project === NO_PROJECT ? !ids.length : ids.includes(selected_project);
}

// Whether the recurring payment `recurringPayment` is in the project `selected_project`, by its
// own projects: for No project, it has none; without a project, every one.
export function recurringPaymentInProject(recurringPayment, selected_project) {
  if (!selected_project) return true;
  const ids = projectsOf(recurringPayment);
  return selected_project === NO_PROJECT ? !ids.length : ids.includes(selected_project);
}

// The bank accounts and statements in the vault panel's filter `state`:
// `{ accounts, statements }`, the statements a Set. Under a project, or No
// project, the statements are filtered without it: their transactions are in it
// through their links (`transactionInProject`). With nothing narrowing the
// scope, every bank account; else the accounts of the statements in it,
// and the account of a statements watch picked as the source.
export function bankScope(state) {
  const filter = state.selected_project ? { ...state, selected_project: null } : state;
  const statements = new Set();
  const all = [];
  for (const doc of entities.values()) {
    if (doc.kind === 'bank-account') all.push(doc);
    else if (doc.statement && matchesFilter(doc, filter)) statements.add(doc);
  }
  const narrowed = state.selected_project || state.selected_source || state.selected_labels.length
    || state.word_filter;
  if (!narrowed) return { accounts: all, statements };
  const ids = new Set([...statements].map(doc => doc.statement.account));
  for (const watch of watches.values()) {
    if (watch.account && watchSourceKey(watch.id) === state.selected_source) ids.add(watch.account);
  }
  return { accounts: all.filter(doc => ids.has(doc.id)), statements };
}

// Money in cents while it is summed, so the sums don't drift.
export const cents = amount => Math.round((amount ?? 0) * 100);
const units = n => (n == null ? null : n / 100);

// The date of the newest transaction of the bank account `id`'s statements,
// pending ones too: how far its statements go; null without any.
export function statementsEnd(id) {
  let end = null;
  if (!id) return end;
  for (const statement of statementsOf(id)) {
    for (const transaction of statement.statement.transactions ?? []) {
      const date = parseDate(transaction.date);
      if (date && (!end || date > end)) end = date;
    }
  }
  return end;
}

// The ledger of the bank account `account` (its entity), from all its
// statements:
// - `entries`: its posted transactions, oldest first (by day, then as their
//   statement lists them), each `{ transaction, statement, time, balance }`,
//   `time` its day's or instant's ms, `balance` the account's after it
//   (null while nothing gives one);
// - `opening`: the balance before the first, `balance` after the last;
// - `pending`: the pending transactions, newest first, and `pendingSum`.
// The balances are walked back from the account's `balance`; a statement
// with a `balance` of its own sets it again at its last transaction, the newest
// statement's first. Pending transactions have none.
export function ledger(account) {
  const posted = [];
  const pending = [];
  for (const statement of statementsOf(account.id)) {
    (statement.statement.transactions ?? []).forEach((transaction, index) => {
      const entry = { transaction, statement, index, time: parseDate(transaction.date)?.getTime() ?? 0, balance: null };
      (transaction.status === 'pending' ? pending : posted).push(entry);
    });
  }
  const order = (a, b) => a.time - b.time
    || (a.statement.date?.getTime() ?? 0) - (b.statement.date?.getTime() ?? 0)
    || a.index - b.index;
  posted.sort(order);
  pending.sort((a, b) => order(b, a));

  const own = account['bank-account']?.balance;
  let balance = own == null ? null : cents(own);
  const anchored = new Set();
  for (let i = posted.length - 1; i >= 0; i--) {
    const { statement } = posted[i];
    if (statement.statement.balance != null && !anchored.has(statement)) {
      anchored.add(statement);
      balance = cents(statement.statement.balance);
    }
    posted[i].balance = units(balance);
    if (balance != null) balance -= cents(posted[i].transaction.amount);
  }

  return {
    entries: posted,
    opening: posted.length ? units(balance) : null,
    balance: posted.length ? posted.at(-1).balance : own ?? null,
    pending,
    pendingSum: units(pending.reduce((sum, entry) => sum + cents(entry.transaction.amount), 0)),
  };
}

// ----------------------------------------------------------------------------
// Recurring payments (vault.md#recurring): a recurring payment of a bank account, the
// transactions naming it (`recurring-payment`), and its occurrences as far as the
// account's statements go, by the rule the import puts transactions into their
// recurring payments by (app/src/recurring_payments.rs).

// How many days from an occurrence a transaction still pays it; half the gap
// to the next occurrence when that is less.
const PAY_DAYS = 7;
// How far past the statements the occurrences are read, so the last one's
// gap to the next is known.
const AHEAD_DAYS = 62;

// The categories a recurring payment is offered; the vault's own are offered too.
export const CATEGORIES = [
  t('category.housing'), t('category.insurance'), t('category.utilities'), t('category.phone'), t('category.subscriptions'),
  t('category.transport'), t('category.loans'), t('category.income'),
];

// The day a transaction was posted (a pending one's card day), at its start.
export function transactionDay(transaction) {
  const date = parseDate(transaction.date);
  return date ? dayStart(date) : null;
}

// The name of a transaction's other side (`partyRef`), '' without one.
export function partyName(transaction) {
  const ref = partyRef(transaction);
  return ref ? parseContact(ref).name : '';
}

// The recurring payment `transaction` belongs to (its `recurring-payment`), if it is in the vault.
export function recurringPaymentOf(transaction) {
  const recurringPayment = entities.get(transaction?.['recurring-payment']);
  return recurringPayment?.['recurring-payment'] ? recurringPayment : null;
}

// The recurring payments of the bank accounts `ids` (a Set), or of all of them.
export function recurringPaymentsIn(ids = null) {
  return [...entities.values()].filter(doc => doc['recurring-payment'] && (!ids || ids.has(doc['recurring-payment'].account)));
}

// The transactions naming the recurring payment `id`, oldest first: `{ statement, transaction,
// day }` each.
function transactionsOf(id) {
  const found = [];
  const account = entities.get(id)?.['recurring-payment']?.account;
  if (!account) return found;
  for (const statement of statementsOf(account)) {
    for (const transaction of statement.statement.transactions ?? []) {
      const day = transactionDay(transaction);
      if (transaction['recurring-payment'] === id && day) found.push({ statement, transaction, day });
    }
  }
  return found.sort((a, b) => a.day - b.day);
}

// A recurring payment's amounts (its role `recurring-payment`'s), oldest first.
export function amountsOf(recurring) {
  return [...(recurring?.amounts ?? [])].sort((a, b) => (parseDate(a.from) ?? 0) - (parseDate(b.from) ?? 0));
}

// The role `recurring-payment` with `amount` from the day `from` (yyyy-mm-dd) on, in
// place of one from that day.
export function withAmount(recurring, from, amount) {
  const kept = amountsOf(recurring).filter(entry => localDay(parseDate(entry.from)) !== from);
  return { ...recurring, amounts: amountsOf({ amounts: [...kept, { from, amount }] }) };
}

// The recurring payment's amount on `day`: its last amount from that day or before,
// else its first; null without any.
export function amountOn(recurringPayment, day) {
  const amounts = amountsOf(recurringPayment['recurring-payment']);
  const current = amounts.filter(entry => parseDate(entry.from) <= day).at(-1) ?? amounts[0];
  return current?.amount ?? null;
}

// The names the recurring payment's party goes by, lowercase: its party's and those of
// its transactions (a name added by hand is known from then on).
export function partyNames(recurringPayment) {
  const names = new Set();
  const add = ref => {
    if (ref) names.add(parseContact(ref).name.trim().toLowerCase());
  };
  add(recurringPayment['recurring-payment']?.party);
  for (const { transaction } of transactionsOf(recurringPayment.id)) add(partyRef(transaction));
  return names;
}

// Whether the names `a` and `b` (lowercase) are similar: one starts with the
// other, as an export may name a party anew ("StreamFlix", "StreamFlix Inc.").
export function similarNames(a, b) {
  return !!a && !!b && a !== b && (a.startsWith(b) || b.startsWith(a));
}

// The days of the recurring payment's schedule from its first occurrence on, before
// `before`, and not after its `until`.
function occurrenceDays(recurringPayment, before) {
  const { schedule = [], from, until } = recurringPayment['recurring-payment'] ?? {};
  const first = parseDate(from);
  if (!first) return [];
  const last = until ? parseDate(until) : null;
  return scheduleStarts(first, schedule, before).filter(day => !last || day <= last);
}

// The occurrences of `recurringPayment` as far as its account's statements go, newest
// first: `{ day, state, transaction, statement }` each, `state` one of
// - `paid`: a transaction of the recurring payment is within its reach (`PAY_DAYS`, or
//   half the gap to the next occurrence), the nearest unpaid one taking it,
//   the transactions oldest first;
// - `off`: paid by a transaction more than `vary` off the amount on its day;
// - `missed`: none, and the statements go `PAY_DAYS` past it;
// - `extra`: a transaction of the recurring payment that pays none, on its own day.
// An occurrence the statements don't go far enough past is left out.
export function occurrences(recurringPayment) {
  const end = statementsEnd(recurringPayment['recurring-payment']?.account);
  if (!end) return [];
  const days = occurrenceDays(recurringPayment, addDays(end, AHEAD_DAYS));
  const reach = i => {
    const before = i > 0 ? daysBetween(days[i - 1], days[i]) : Infinity;
    const after = i + 1 < days.length ? daysBetween(days[i], days[i + 1]) : Infinity;
    return Math.min(PAY_DAYS, Math.floor(Math.min(before, after) / 2));
  };
  const paid = new Map(); // index in `days` -> the transaction paying it
  const out = [];
  for (const entry of transactionsOf(recurringPayment.id)) {
    const apart = i => Math.abs(daysBetween(days[i], entry.day));
    let best = -1;
    days.forEach((_, i) => {
      if (paid.has(i) || apart(i) > reach(i)) return;
      if (best < 0 || apart(i) < apart(best)) best = i;
    });
    if (best < 0) out.push({ day: entry.day, state: 'extra', transaction: entry.transaction, statement: entry.statement });
    else paid.set(best, entry);
  }
  const vary = cents(recurringPayment['recurring-payment']?.vary ?? 0);
  days.forEach((day, i) => {
    const entry = paid.get(i);
    if (entry) {
      const expected = amountOn(recurringPayment, day);
      const off = expected != null && Math.abs(cents(entry.transaction.amount) - cents(expected)) > vary;
      out.push({ day, state: off ? 'off' : 'paid', transaction: entry.transaction, statement: entry.statement });
    } else if (daysBetween(day, end) >= PAY_DAYS) {
      out.push({ day, state: 'missed', transaction: null, statement: null });
    }
  });
  return out.sort((a, b) => b.day - a.day);
}

// The occurrences of `recurringPayment` no transaction pays yet (pending ones included),
// to `until`, oldest first: `{ recurringPayment, day, amount }` each, the amount as
// on its day (`amountOn`). One the statements go `PAY_DAYS` past is missed
// instead (`occurrences`); without statements, they start today.
function expectedOf(recurringPayment, until) {
  const end = statementsEnd(recurringPayment['recurring-payment']?.account);
  const paid = new Set(occurrences(recurringPayment).filter(o => o.state !== 'extra').map(o => o.day.getTime()));
  const from = end ? addDays(dayStart(end), 1 - PAY_DAYS) : dayStart(new Date());
  return occurrenceDays(recurringPayment, addDays(dayStart(until), 1))
    .filter(day => day >= from && !paid.has(day.getTime()))
    .map(day => ({ recurringPayment, day, amount: amountOn(recurringPayment, day) ?? 0 }));
}

// What the recurring payments `list` are expected to pay or bring to `until`
// (`expectedOf`), by day.
export function expectedIn(list, until) {
  return list.flatMap(recurringPayment => expectedOf(recurringPayment, until)).sort((a, b) => a.day - b.day);
}

// The recurring payment's next occurrence after its account's statements (after today
// without any) that no transaction pays yet; null when it has ended.
export function nextOccurrence(recurringPayment) {
  const after = statementsEnd(recurringPayment['recurring-payment']?.account) ?? dayStart(new Date());
  const until = new Date(after.getFullYear() + 2, after.getMonth(), after.getDate());
  return expectedOf(recurringPayment, until).find(entry => entry.day > after)?.day ?? null;
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

// An account's balance once what is pending is posted and the payments
// `expected` (`expectedOf`, its recurring payments') are made: its ledger `book`'s
// balance with both; null without a balance.
export function balanceOn(book, expected) {
  if (book.balance == null) return null;
  const ahead = expected.reduce((sum, entry) => sum + cents(entry.amount), 0);
  return units(cents(book.balance) + cents(book.pendingSum) + ahead);
}

// Whether the recurring payment has ended: its `until` before `today`.
export function recurringPaymentEnded(recurringPayment, today = dayStart(new Date())) {
  const until = parseDate(recurringPayment['recurring-payment']?.until);
  return !!until && until < today;
}

// How many times a year the recurring payment's schedule repeats; null for one the page
// doesn't read.
export function perYear(recurringPayment) {
  return timesAYear(recurringPayment['recurring-payment']?.schedule ?? []);
}

// How often the recurring payment repeats, in a word: "monthly", "twice a month", "4
// times a year".
export function howOften(recurringPayment) {
  const word = { week: t('often.week'), month: t('often.month'), quarter: t('often.quarter'), half: t('often.half'), year: t('often.year') };
  const period = periodOf(recurringPayment['recurring-payment']?.schedule)?.period;
  if (period) return word[period];
  const times = perYear(recurringPayment);
  if (times === 24) return t('often.twice_a_month');
  return times ? t('often.times_a_year', { n: Math.round(times * 10) / 10 }) : t('often.on_its_schedule');
}
