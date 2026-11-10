import { equal, test } from './harness.js';
import { vault } from './vault.js';
import { localDay, parseDate } from '../../static/core/format.js';
import {
  amountOn, amountsOf, transactionName, transactionOf, howOften, ledger, linkProjects, nextOccurrence, occurrences, pairOf, partyNames,
  partyRef, perYear, recurringPaymentEnded, recurringPaymentOf, recurringPaymentsIn, similarNames, statementsEnd, statementsOf,
  withAmount,
} from '../../static/model/bank.js';
import { entities, threads } from '../../static/model/entities.js';

const account = (id, balance) => ({ id, kind: 'bank-account', title: id, 'bank-account': { balance } });
const statement = (id, of, transactions, more = {}) => ({ id, kind: 'statement', title: id, date: '2027-01-01', statement: { account: of, transactions, ...more } });
const transaction = (id, date, amount, more = {}) => ({ id, title: id, date, amount, status: 'posted', ...more });
const recurring = (id, more = {}) => ({
  id, kind: 'recurring-payment', title: id, 'recurring-payment': { account: 'checking', schedule: ['RRULE:FREQ=MONTHLY;BYMONTHDAY=1'], from: '2027-01-01', ...more },
});
const of = id => entities.get(id);

const titles = entries => entries.map(entry => [entry.transaction.id, entry.balance]);

test("the balances are walked back from the account's", () => {
  vault(account('checking', 100), statement('s', 'checking', [transaction('b', '2027-01-03', 50), transaction('a', '2027-01-02', -20), transaction('c', '2027-01-03', -30)]));
  const book = ledger(of('checking'));
  equal(titles(book.entries), [['a', 80], ['b', 130], ['c', 100]]);
  equal([book.opening, book.balance, book.pending, book.pendingSum], [100, 100, [], 0]);
});

test("a statement's own balance sets it again at its last transaction", () => {
  vault(
    account('checking', 100), statement('s1', 'checking', [transaction('a', '2027-01-02', -20)], { balance: 40 }),
    statement('s2', 'checking', [transaction('b', '2027-02-02', 10)]),
  );
  equal(titles(ledger(of('checking')).entries), [['a', 40], ['b', 100]]);
  equal(ledger(of('checking')).opening, 60);
});

test('without a balance none is known, and nothing posted is the account as it is', () => {
  vault(account('checking'), statement('s', 'checking', [transaction('a', '2027-01-02', -20)]), account('savings', 7));
  const book = ledger(of('checking'));
  equal([titles(book.entries), book.opening, book.balance], [[['a', null]], null, null]);
  equal([ledger(of('savings')).balance, ledger(of('savings')).opening], [7, null]);
});

test('the sums do not drift', () => {
  vault(account('checking', 0.3), statement('s', 'checking', [transaction('a', '2027-01-02', 0.1), transaction('b', '2027-01-03', 0.2)]));
  equal([ledger(of('checking')).entries[0].balance, ledger(of('checking')).opening], [0.1, 0]);
});

test('what is pending stands apart, the newest first', () => {
  const pending = (id, date, amount) => transaction(id, date, amount, { status: 'pending' });
  vault(account('checking', 100), statement('s', 'checking', [transaction('a', '2027-01-02', -20), pending('p', '2027-01-03', -5), pending('q', '2027-01-04', -7.5)]));
  const book = ledger(of('checking'));
  equal([titles(book.entries), book.pending.map(entry => entry.transaction.id), book.pendingSum, book.balance], [[['a', 100]], ['q', 'p'], -12.5, 100]);
});

test('how far the statements go, what is pending too', () => {
  vault(account('checking'), statement('s2', 'checking', [transaction('p', '2027-02-03', -5, { status: 'pending' })]), statement('s1', 'checking', [transaction('a', '2027-01-02', -20)]));
  equal([statementsEnd('checking'), statementsEnd('gone'), statementsEnd(null)], [new Date(2027, 1, 3), null, null]);
  equal(statementsOf('checking').map(one => one.id), ['s2', 's1']);
});

test('a transaction of a statement', () => {
  vault(statement('s1', 'checking', [transaction('a', '2027-01-02', -20)]));
  equal([transactionOf(of('s1'), 'a').amount, transactionOf(of('s1'), 'p'), transactionOf(null, 'a')], [-20, null, null]);
});

test('the other side is who was paid, or who paid', () => {
  const sides = { sender: 'Maya', receiver: 'Shop' };
  equal([partyRef({ ...sides, amount: -5 }), partyRef({ ...sides, amount: 5 })], ['Shop', 'Maya']);
});

test("a transfer's halves name each other", () => {
  vault(
    account('checking'), account('savings'),
    statement('s1', 'checking', [transaction('out', '2027-01-02', -20, { transfer: 'in' }), transaction('lone', '2027-01-03', -5, { transfer: 'in' }), transaction('none', '2027-01-04', -1)]),
    statement('s2', 'savings', [transaction('in', '2027-01-03', 20, { transfer: 'out' })]),
  );
  const half = id => pairOf(transactionOf(of('s1'), id));
  equal([half('out').transaction.id, half('out').account.id, half('out').statement.id], ['in', 'savings', 's2']);
  equal([half('lone'), half('none'), pairOf(null)], [null, null, null]);
});

test("a transfer is called after the other half's account, else the other side", () => {
  const pair = { account: { title: 'House fund' } };
  equal([transactionName({ amount: -5 }, pair), transactionName({ amount: 5 }, pair)], ['transaction.to_account', 'transaction.from_account']);
  equal([transactionName({ amount: -5, receiver: 'Shop' }), transactionName({ amount: -5, title: 'Lunch' }), transactionName({ amount: -5 })], ['Shop', 'Lunch', 'bank.transaction']);
});

// Two projects, a bill in the one, a thread and a rent in the other.
function linked() {
  threads.clear();
  threads.set('t', { id: 't' });
  vault(
    { id: 'p', kind: 'project', title: 'P' }, { id: 'q', kind: 'project', title: 'Q' }, { id: 'bill', kind: 'document', title: '', projects: ['p'] },
    { id: 'm', kind: 'email', title: '', threads: ['t'], projects: ['q'] }, { ...recurring('rent'), projects: ['q'] },
  );
}

test('the projects of a transaction are those of what it is linked to', () => {
  linked();
  equal(['p', 'bill', 't', 'gone', undefined].map(id => linkProjects({ 'linked-to': id })), [['p'], ['p'], ['q'], [], []]);
});

test("a transaction without a link is in its recurring payment's projects", () => {
  linked();
  equal([linkProjects({ 'recurring-payment': 'rent' }), linkProjects({ 'recurring-payment': 'rent', 'linked-to': 'bill' })], [['q'], ['p']]);
});

// A recurring payment of `checking` on the first of the month, its transactions
// and how far the statements go.
function rent(transactions, end, more = {}) {
  vault(
    account('checking'), recurring('rent', more),
    statement('s', 'checking', [...transactions.map(([id, date, amount]) => transaction(id, date, amount, { 'recurring-payment': 'rent' })), transaction('end', end, -1)]),
  );
  return of('rent');
}
const states = list => list.map(one => [localDay(one.day), one.state, one.transaction?.id ?? null]);

test('an occurrence is paid by the transaction near it, else missed', () => {
  const payment = rent([['jan', '2027-01-02', -1150], ['feb', '2027-02-08', -1150]], '2027-04-07');
  equal(states(occurrences(payment)), [['2027-03-01', 'missed', null], ['2027-02-01', 'paid', 'feb'], ['2027-01-01', 'paid', 'jan']]);
});

test("an occurrence the statements don't go past is open, one they go a week past is missed", () => {
  equal(states(occurrences(rent([['jan', '2027-01-02', -1150]], '2027-02-07'))), [['2027-01-01', 'paid', 'jan']]);
  equal(states(occurrences(rent([['jan', '2027-01-02', -1150]], '2027-02-08'))), [['2027-02-01', 'missed', null], ['2027-01-01', 'paid', 'jan']]);
});

test('an occurrence is paid ahead of its day, also past the statements', () => {
  equal(states(occurrences(rent([['jan', '2026-12-30', -1150], ['feb', '2027-01-29', -1150]], '2027-01-30'))),
    [['2027-02-01', 'paid', 'feb'], ['2027-01-01', 'paid', 'jan']]);
});

test('a transaction too far from any occurrence, or beside a paid one, is extra', () => {
  const payment = rent([['mid', '2027-01-16', -10], ['again', '2027-01-03', -1150], ['jan', '2027-01-02', -1150]], '2027-01-20');
  equal(states(occurrences(payment)), [['2027-01-16', 'extra', 'mid'], ['2027-01-03', 'extra', 'again'], ['2027-01-01', 'paid', 'jan']]);
});

test('a weekly one reaches half way to the next', () => {
  const payment = rent([['mon', '2027-03-22', -10], ['fri', '2027-04-02', -10]], '2027-04-03',
    { schedule: ['RRULE:FREQ=WEEKLY;BYDAY=MO'], from: '2027-03-22' });
  equal(states(occurrences(payment)), [['2027-04-05', 'paid', 'fri'], ['2027-03-22', 'paid', 'mon']]);
});

test('a payment off its amount is told, unless it varies that much', () => {
  const amounts = [{ from: '2027-01-01', amount: -1150 }];
  const transactions = [['jan', '2027-01-02', -1150], ['feb', '2027-02-02', -1155]];
  equal(states(occurrences(rent(transactions, '2027-02-03', { amounts })))[0], ['2027-02-01', 'off', 'feb']);
  equal(states(occurrences(rent(transactions, '2027-02-03', { amounts, vary: 5 })))[0], ['2027-02-01', 'paid', 'feb']);
});

test('without statements or a first day there are no occurrences', () => {
  vault(account('checking'), recurring('rent'));
  equal(occurrences(of('rent')), []);
  equal(occurrences(rent([], '2027-02-03', { from: undefined })), []);
});

test('the occurrences end with the last day', () => {
  equal(states(occurrences(rent([], '2027-06-01', { until: '2027-02-01' }))), [['2027-02-01', 'missed', null], ['2027-01-01', 'missed', null]]);
});

test('the amount on a day is the last from that day or before, else the first', () => {
  vault(recurring('rent', { amounts: [{ from: '2027-03-01', amount: -1200 }, { from: '2027-01-01', amount: -1150 }] }));
  const on = day => amountOn(of('rent'), parseDate(day));
  equal([on('2026-12-01'), on('2027-01-01'), on('2027-02-28'), on('2027-03-01')], [-1150, -1150, -1150, -1200]);
  equal(amountOn(recurring('none'), parseDate('2027-01-01')), null);
});

test('an amount from a day on takes the place of the one from that day', () => {
  const role = { amounts: [{ from: '2027-03-01', amount: -1200 }, { from: '2027-01-01', amount: -1150 }], vary: 3 };
  equal(withAmount(role, '2027-03-01', -1300), { amounts: [{ from: '2027-01-01', amount: -1150 }, { from: '2027-03-01', amount: -1300 }], vary: 3 });
  equal(withAmount(role, '2027-02-01', -1160).amounts.map(entry => entry.from), ['2027-01-01', '2027-02-01', '2027-03-01']);
  equal(amountsOf(undefined), []);
});

test('a recurring payment has ended after its last day', () => {
  const payment = recurring('rent', { until: '2027-02-01' });
  equal([recurringPaymentEnded(payment, new Date(2027, 1, 1)), recurringPaymentEnded(payment, new Date(2027, 1, 2)), recurringPaymentEnded(recurring('rent'), new Date())],
    [false, true, false]);
  equal(nextOccurrence(rent([], '2027-06-01', { until: '2027-02-01' })), null);
});

test('how often a recurring payment repeats', () => {
  const often = (...schedule) => howOften(recurring('r', { schedule }));
  equal([
    often('RRULE:FREQ=WEEKLY;BYDAY=MO'), often('RRULE:FREQ=MONTHLY;BYMONTHDAY=1'), often('RRULE:FREQ=MONTHLY;INTERVAL=3;BYMONTHDAY=1'),
    often('RRULE:FREQ=MONTHLY;INTERVAL=6;BYMONTHDAY=1'), often('RRULE:FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=1'),
    often('RRULE:FREQ=MONTHLY;BYMONTHDAY=1,15'), often('RRULE:FREQ=WEEKLY;INTERVAL=2'), often('RRULE:FREQ=HOURLY'),
  ], ['often.week', 'often.month', 'often.quarter', 'often.half', 'often.year', 'often.twice_a_month', 'often.times_a_year', 'often.on_its_schedule']);
  equal([perYear(recurring('r')), perYear(recurring('r', { schedule: ['RRULE:FREQ=HOURLY'] }))], [12, null]);
});

test("a party's names are its own and those of its transactions", () => {
  vault(
    account('checking'), { id: 'landlord', kind: 'contact', title: 'Shady Pines' }, recurring('rent', { party: '@landlord' }),
    statement('s', 'checking', [transaction('a', '2027-01-02', -1, { receiver: ' SHADY Pines Properties ', 'recurring-payment': 'rent' }), transaction('b', '2027-01-02', -1, { receiver: 'Shop' })]),
  );
  equal([...partyNames(of('rent'))], ['shady pines', 'shady pines properties']);
});

test('names are similar when one starts with the other', () => {
  equal([similarNames('streamflix', 'streamflix inc.'), similarNames('streamflix inc.', 'streamflix'), similarNames('streamflix', 'streamflix'),
    similarNames('streamflix', 'flix'), similarNames('', 'flix')], [true, true, false, false, false]);
});

test('the recurring payments of some accounts, and the one a transaction belongs to', () => {
  vault(recurring('rent'), recurring('saving', { account: 'savings' }), { id: 'letter', kind: 'document', title: '' });
  equal([recurringPaymentsIn().map(one => one.id), recurringPaymentsIn(new Set(['savings'])).map(one => one.id)], [['rent', 'saving'], ['saving']]);
  equal([recurringPaymentOf({ 'recurring-payment': 'rent' })?.id, recurringPaymentOf({ 'recurring-payment': 'letter' }), recurringPaymentOf({})], ['rent', null, null]);
});

test('the transactions of a day are by their statements, then as those list them', () => {
  vault(
    account('checking', 0), statement('s2', 'checking', [transaction('x', '2027-01-31', 1)], { date: '2027-02-01' }),
    statement('s1', 'checking', [transaction('y', '2027-01-31', 1), transaction('z', '2027-01-31', 1)]),
  );
  entities.get('s2').date = new Date(2027, 1, 1);
  equal(ledger(of('checking')).entries.map(entry => entry.transaction.id), ['y', 'z', 'x']);
});

test('a transaction reaches half the smaller gap around an occurrence', () => {
  const twice = { schedule: ['RRULE:FREQ=MONTHLY;BYMONTHDAY=1,10'] };
  equal(states(occurrences(rent([['six', '2027-01-06', -10]], '2027-01-12', twice))), [['2027-01-10', 'paid', 'six'], ['2027-01-01', 'missed', null]]);
  equal(states(occurrences(rent([['late', '2027-01-15', -10]], '2027-01-16', twice))), [['2027-01-15', 'extra', 'late'], ['2027-01-01', 'missed', null]]);
});

test('a transaction as far from two occurrences pays the first', () => {
  const payment = rent([['between', '2027-01-03', -10]], '2027-01-03', { schedule: ['RRULE:FREQ=DAILY;INTERVAL=4'] });
  equal(states(occurrences(payment)), [['2027-01-01', 'paid', 'between']]);
});

test('the next occurrence is after the statements, not on their last day', () => {
  equal(localDay(nextOccurrence(rent([['jan', '2027-01-02', -1150]], '2027-02-01'))), '2027-03-01');
});
