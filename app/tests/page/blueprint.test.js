import { equal, test } from './harness.js';
import { vault } from './vault.js';
import { projects } from '../../static/model/entities.js';
import {
  checksOf, factsOf, hasBlueprint, membersOf, newProjectBlock, withBlueprint, withFact,
} from '../../static/model/blueprint.js';

const TODAY = new Date(2027, 2, 15);
const project = block => ({ id: 'p', kind: 'project', title: 'P', project: block });
const doc = (id, more = {}) => ({ id, kind: 'document', title: id, projects: ['p'], ...more });
const task = (id, due, status = 'open') => doc(id, { kind: 'task', task: { status, due } });
const statement = (...transactions) => ({ id: 's', kind: 'statement', title: '', statement: { account: 'a', transactions } });
const check = () => checksOf('p', projects.get('p').project, TODAY);
const BLUEPRINT = { name: 'move', checks: ['deadlines', 'documents'], labels: ['move'] };

test('a project takes the checks and the labels of its blueprint', () => {
  equal(newProjectBlock(BLUEPRINT), { blueprint: 'move', checks: ['deadlines', 'documents'], labels: ['move'] });
  equal(newProjectBlock({ name: 'plain', checks: [], labels: [] }), { blueprint: 'plain' });
});

test('a blueprint put on a project leaves what it has', () => {
  equal(withBlueprint({ checks: ['budget', 'deadlines'], facts: { a: 1 } }, BLUEPRINT),
    { blueprint: 'move', checks: ['budget', 'deadlines', 'documents'], labels: ['move'], facts: { a: 1 } });
});

test('a project has something for its header to show', () => {
  equal([{ blueprint: 'move' }, { facts: { a: 1 } }, { documents: [{}] }, { payments: [{}] }, { checks: ['budget'] }, { facts: {}, documents: [] }, {}, null].map(hasBlueprint),
    [true, true, true, true, true, false, false, false]);
});

test("a project's own facts, as text", () => {
  equal(factsOf({ facts: { landlord: 'Shady Pines', rent: 1150 } }), [
    { name: 'landlord', label: 'landlord', type: 'text', value: 'Shady Pines' }, { name: 'rent', label: 'rent', type: 'text', value: 1150 },
  ]);
  equal(factsOf(null), []);
});

test('a fact is set, changed and, empty, removed', () => {
  equal(withFact({ blueprint: 'move' }, 'rent', 1150), { blueprint: 'move', facts: { rent: 1150 } });
  equal(withFact({ facts: { rent: 1150, a: 1 } }, 'rent', 0).facts, { rent: 0, a: 1 });
  equal(['', null, undefined, []].map(value => withFact({ facts: { rent: 1150 } }, 'rent', value)), [{}, {}, {}, {}]);
});

test('the members of a project', () => {
  vault(project({}), doc('a'), doc('b', { projects: ['other'] }), doc('c', { projects: ['other', 'p'] }));
  equal(membersOf('p').map(one => one.id), ['a', 'c']);
});

test('without checks nothing is missing', () => {
  vault(project({ documents: [{ name: 'Lease' }], facts: { budget: 1 } }), task('late', '2027-03-01'));
  equal([check().missing, check().progress], [[], {}]);
});

test('an expected document is missing until one meets it, and again once that is too old', () => {
  const documents = [{ name: 'Lease', by: ['lease'] }, { name: 'Payslip', by: ['old', 'gone'], within: '3m' }, { name: 'Passport', by: [] }, { name: 'Bill', by: ['old', 'new'], within: '6w' }];
  vault(project({ checks: ['documents'], documents }), doc('lease'), doc('old', { date: '2026-12-14' }), doc('new', { date: '2027-02-01' }));
  equal(check().missing.map(gap => [gap.expected.name, gap.index, gap.stale]), [['Payslip', 1, true], ['Passport', 2, false]]);
  equal(check().progress.documents, [2, 4]);
});

test('a span back counts its first day', () => {
  const met = (within, date) => {
    vault(project({ checks: ['documents'], documents: [{ name: 'x', by: ['a'], within }] }), doc('a', { date }));
    return check().progress.documents[0] === 1;
  };
  equal([met('90d', '2026-12-15'), met('90d', '2026-12-14'), met('2w', '2027-03-01'), met('2w', '2027-02-28'), met('3m', '2026-12-15'), met('1y', '2026-03-15'), met('1y', '2026-03-14')],
    [true, false, true, false, true, true, false]);
  equal(met('soon', '2001-01-01'), true);
});

test('a payment past its day is missing until a transaction is linked to what it is for', () => {
  const payments = [{ name: 'Later' }, { name: 'Fee', due: '2027-03-15' }, { name: 'Deposit', for: 'bill', due: '2027-03-01' }, { name: 'Rent', for: 'lease', due: '2027-03-14' }];
  vault(project({ checks: ['payments'], payments }), doc('bill'), doc('lease'), statement({ id: 'b', date: '2027-03-02', amount: -100, 'linked-to': 'bill' }));
  equal(check().missing.map(gap => gap.payment.name), ['Rent']);
  equal([check().progress.payments, check().next.map(item => item.payment.name)], [[1, 4], ['Rent', 'Fee', 'Later']]);
});

test('the payments awaited are next without their check too', () => {
  vault(project({ payments: [{ name: 'Rent', due: '2027-03-01' }] }));
  equal([check().missing, check().next.length, check().progress], [[], 1, {}]);
});

test('a budget is spent by the transactions linked into the project, transfers left out', () => {
  const transactions = [
    { id: 'a', date: '2027-03-01', amount: -80.1, 'linked-to': 'bill' }, { id: 'b', date: '2027-03-02', amount: 10, 'linked-to': 'p' },
    { id: 'c', date: '2027-03-02', amount: -500 }, { id: 'd', date: '2027-03-03', amount: -200, 'linked-to': 'p', transfer: 'e' },
  ];
  vault(
    project({ checks: ['budget'], facts: { budget: 70 } }), doc('bill'), statement(...transactions),
    { id: 's2', kind: 'statement', title: '', statement: { account: 'b', transactions: [{ id: 'e', date: '2027-03-03', amount: 200, transfer: 'd' }] } },
  );
  equal(check().progress.budget, { spent: 70.1, budget: 70 });
  equal(check().missing.map(gap => [gap.rule, gap.spent]), [['budget', 70.1]]);
});

test('a budget that is none is not checked', () => {
  for (const budget of [undefined, '', 'some', 0, -5]) {
    vault(project({ checks: ['budget'], facts: { budget } }));
    equal(check().progress, {});
  }
});

test('an open task past its day is missing, and the steps are the tasks done', () => {
  vault(project({ checks: ['deadlines'] }), task('late', '2027-03-14'), task('today', '2027-03-15'), task('done', '2027-03-01', 'done'), task('whenever'));
  equal(check().missing.map(gap => gap.doc.id), ['late']);
  equal(check().progress.steps, [1, 4]);
});

test('what is next: the open tasks by their day, those without one last, then the payments', () => {
  vault(project({ payments: [{ name: 'Rent', due: '2027-01-01' }] }), task('whenever'), task('b', '2027-04-02'), task('a', '2027-04-01'));
  equal(check().next.map(item => item.doc?.id ?? item.payment.name), ['a', 'b', 'whenever', 'Rent']);
});

test("a trip's checks are among a project's, only those it has", () => {
  const facts = { home: 'Springfield', days: { from: '2027-07-08', to: '2027-07-10' } };
  vault(project({ blueprint: 'travel', checks: ['nights'], facts }));
  equal([check().missing.map(gap => gap.rule), check().progress.nights], [['nights'], [0, 2]]);
  vault(project({ blueprint: 'travel', checks: ['journeys'], sketch: { nights: 3 } }));
  equal([check().missing, check().later], [[], { nights: 3, journeys: 0 }]);
});

test('a budget spent to its end is kept', () => {
  vault(project({ checks: ['budget'], facts: { budget: 70 } }), statement({ id: 'a', date: '2027-03-01', amount: -70, 'linked-to': 'p' }));
  equal([check().missing, check().progress.budget], [[], { spent: 70, budget: 70 }]);
});
