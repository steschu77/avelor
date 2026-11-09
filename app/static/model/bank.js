// Bank accounts and their statements (vault.md, Bank Account and
// Transactions): an account's statements (`statementsOf`) and its ledger,
// the booked bookings with the balance after each (`ledger`); a booking's
// party (`partyOf`), the other half of a transfer (`pairOf`), its name
// (`bookingName`) and its projects through its link (`linkProjects`).
// The Bank lens (features/lenses/bank.js), a booking's card
// (features/panel/booking.js) and a statements watch's pane
// (features/workspace/folders.js) show them.

import { parseDate } from '../core/format.js';
import { entities, projects, projectsOf, threadProjects, threads } from './entities.js';

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
// the projects of the entity or thread it links to; none without a link.
export function linkProjects(booking) {
  const to = booking?.['linked-to'];
  if (!to) return [];
  if (projects.has(to)) return [to];
  if (threads.has(to)) return threadProjects(to);
  const doc = entities.get(to);
  return doc ? projectsOf(doc) : [];
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
