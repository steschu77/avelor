import { equal, test } from './harness.js';
import { vault } from './vault.js';
import { transactionOf, ledger, linkProjects, partyRef, statementsEnd, statementsOf } from '../../static/model/bank.js';
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

test('the transactions of a day are by their statements, then as those list them', () => {
  vault(
    account('checking', 0), statement('s2', 'checking', [transaction('x', '2027-01-31', 1)], { date: '2027-02-01' }),
    statement('s1', 'checking', [transaction('y', '2027-01-31', 1), transaction('z', '2027-01-31', 1)]),
  );
  entities.get('s2').date = new Date(2027, 1, 1);
  equal(ledger(of('checking')).entries.map(entry => entry.transaction.id), ['y', 'z', 'x']);
});
