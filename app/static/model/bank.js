// Bank accounts and their statements (vault.md, Bank Account and
// Transactions): an account's statements (`statementsOf`) and its ledger,
// the booked bookings with the balance after each (`ledger`). The Bank lens
// (features/lenses/bank.js) and a statements watch's pane
// (features/workspace/folders.js) show them.

import { parseDate } from '../core/format.js';
import { entities } from './entities.js';

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
