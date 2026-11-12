import { equal, test } from './harness.js';
import { entity, vault } from './vault.js';
import {
  NO_PROJECT, assistantConversations, calendarEntityOf, entities, entityOf, knownThreads, matchesFilter, parseContact,
  projectNames, projectScope, projects, projectsFor, projectsInOrder, projectsOf, sourceKeyOf, threadMatchesFilter, threadMembers,
  threadOf, threadProjects, threads, watches,
} from '../../static/model/entities.js';
import { store } from '../../static/model/state.js';

const project = (id, more = {}) => ({ id, kind: 'project', title: id, ...more });
const doc = (id, more = {}) => ({ id, kind: 'document', title: id, ...more });
const ids = list => list.map(one => one.id);
const FILTER = { selected_project: null, selected_source: null, selected_labels: [], word_filter: null };
const shown = filter => [...entities.values()].filter(one => matchesFilter(one, { ...FILTER, ...filter })).map(one => one.id);

test('the starred projects come first, the archived last, else as they arrived', () => {
  vault(project('b'), project('gone by', { archived: true }), project('a'), project('star', { starred: true }));
  equal(ids(projectsInOrder()), ['star', 'b', 'a', 'gone by']);
});

test("an entity's projects are those in the vault", () => {
  vault(project('p'));
  equal([projectsOf({ projects: ['p', 'deleted'] }), projectsOf({})], [['p'], []]);
});

test('without a filter everything is shown', () => {
  vault(project('p'), doc('a', { projects: ['p'] }), doc('b'));
  equal(shown({}), ['a', 'b']);
});

test('a project as the filter, and none', () => {
  vault(project('p'), project('q'), doc('a', { projects: ['p'] }), doc('b', { projects: ['q', 'p'] }), doc('c'), doc('d', { projects: ['deleted'] }));
  equal(shown({ selected_project: 'p' }), ['a', 'b']);
  equal(shown({ selected_project: NO_PROJECT }), ['c', 'd']);
});

test('labels as the filter are all carried', () => {
  vault(doc('a', { labels: ['x', 'y'] }), doc('b', { labels: ['x'] }), doc('c'));
  equal([shown({ selected_labels: ['x'] }), shown({ selected_labels: ['x', 'y'] })], [['a', 'b'], ['a']]);
});

test('words and a source as the filter', () => {
  vault(doc('a', { source: { created: {} } }), doc('b', { source: { scan: { watch: 'w' } } }));
  equal(shown({ word_filter: { hits: new Map([['b', {}]]) } }), ['b']);
  equal(shown({ selected_source: 'created' }), ['a']);
  equal(shown({ selected_source: 'watch:w', word_filter: { hits: new Map([['a', {}]]) } }), []);
});

test("the store's filter is the one without another", () => {
  vault(project('p'), doc('a', { projects: ['p'] }), doc('b'));
  store.setState({ selected_project: 'p' });
  equal([...entities.values()].map(one => matchesFilter(one)), [true, false]);
});

test("a thread's members are by date, those without one first, else as they were made", () => {
  vault(
    doc('4', { threads: ['t'], date: '2027-07-09' }), doc('3', { threads: ['t'], date: '2027-07-08' }), doc('2', { threads: ['t'], date: '2027-07-08' }),
    doc('9', { threads: ['t'] }), doc('1', { threads: ['other'] }),
  );
  equal(ids(threadMembers('t')), ['9', '2', '3', '4']);
});

// A vault with the threads t and u.
function threaded(...metas) {
  threads.clear();
  threads.set('t', { id: 't' });
  threads.set('u', { id: 'u' });
  vault(...metas);
}

test("an entity's threads are those in the vault", () => {
  threaded(doc('a', { threads: ['dissolved', 'u', 't'] }), doc('b'));
  equal([knownThreads(entities.get('a')), knownThreads(entities.get('b'))], [['u', 't'], []]);
});

test("an entity's thread is the first in the vault", () => {
  threaded(doc('a', { threads: ['dissolved', 'u', 't'] }), doc('b'));
  equal([threadOf('a'), threadOf('b'), threadOf('gone')], ['u', undefined, undefined]);
});

const MEMBERS = [project('p'), project('q'), doc('1', { threads: ['t'], projects: ['q'] }), doc('2', { threads: ['t'], projects: ['p', 'q', 'deleted'] })];

test("a thread's projects are its members', each once", () => {
  vault(...MEMBERS);
  equal(threadProjects('t'), ['q', 'p']);
});

test('a thread is in the filter with one of its members', () => {
  vault(...MEMBERS);
  equal([threadMatchesFilter('t', { ...FILTER, selected_project: 'p' }), threadMatchesFilter('t', { ...FILTER, selected_project: NO_PROJECT })], [true, false]);
});

test('something new starts in the projects of what it is for, else in the one selected', () => {
  threaded(project('p'), project('q'), doc('a', { projects: ['q'] }), doc('b'), doc('m', { threads: ['t'], projects: ['q'] }));
  store.setState({ selected_project: 'p' });
  equal(projectsFor('a'), { projects: ['q'], scope: null });
  equal(projectsFor('t'), { projects: ['q'], scope: null });
  equal([projectsFor('b'), projectsFor(null)], [{ projects: ['p'], scope: 'p' }, { projects: ['p'], scope: 'p' }]);
  store.setState({ selected_project: NO_PROJECT });
  equal([projectsFor('b'), projectScope()], [{ projects: [], scope: null }, null]);
});

test("the names of an entity's projects, while none is the filter", () => {
  vault(project('p'), project('q'));
  equal(projectNames({ projects: ['p', 'q', 'deleted'] }, { selected_project: null }), 'p \u00b7 q');
  equal(projectNames({ projects: ['p'] }, { selected_project: 'p' }), '');
});

test('the source of what was made in the app, and of what none brought', () => {
  equal([sourceKeyOf({ source: { created: {} } }), sourceKeyOf({ source: 'created' }), sourceKeyOf({ source: { web: {} } }), sourceKeyOf({})], ['created', 'created', null, null]);
});

test("the source of an email is its account's", () => {
  equal([sourceKeyOf({ kind: 'email', source: { remote: { account: 'g' } } }), sourceKeyOf({ kind: 'email', source: null })], ['g\nemail', null]);
});

test("the source of an event is the Google calendar its own syncs with", () => {
  vault(
    { id: 'family', kind: 'calendar', source: { remote: { account: 'g', remote: [{ 'calendar-id': 'fam@google' }] } } },
    { id: 'own', kind: 'calendar', source: { created: {} } }, { id: 'half', kind: 'calendar', source: { remote: { account: 'g', remote: [] } } },
  );
  equal(['family', 'own', 'half', 'gone'].map(id => sourceKeyOf({ kind: 'event', event: { calendar: id }, source: null })), ['g\ncalendar\nfam@google', null, null, null]);
});

test('the source of a scan or an import is its folder watch', () => {
  equal([sourceKeyOf({ source: { scan: { watch: 'w1' } } }), sourceKeyOf({ source: { scan: {} } })], ['watch:w1', null]);
});

test("the source of a statement is its account's watch, whichever imported it", () => {
  watches.clear();
  watches.set('w0', { id: 'w0', type: 'images', account: 'bank' });
  watches.set('w1', { id: 'w1', type: 'statements', account: 'savings' });
  watches.set('w3', { id: 'w3', type: 'statements', account: 'bank' });
  const imported = account => sourceKeyOf({ statement: { account }, source: { import: { watch: 'w2' } } });
  equal([imported('bank'), imported('other')], ['watch:w3', 'watch:w2']);
});

test("the calendar of the vault that syncs with one of Google's", () => {
  vault(
    { id: 'other', kind: 'calendar', source: { remote: { account: 'h', remote: [{ 'calendar-id': 'fam@google' }] } } },
    { id: 'family', kind: 'calendar', source: { remote: { account: 'g', remote: [{ 'sync-token': 'x' }, { 'calendar-id': 'fam@google' }] } } },
  );
  equal([calendarEntityOf('g', 'fam@google')?.id, calendarEntityOf('g', 'work@google')], ['family', null]);
});

test('the conversations with an AI on an entity, the newest first', () => {
  const chat = (id, date, more = {}) => ({ id, kind: 'conversation', date, source: { ai: {} }, linked_to: 'letter', ...more });
  vault(
    chat('1', '2027-07-08'), chat('3', '2027-07-09'), chat('2', '2027-07-09'), chat('4', '2027-07-10', { source: { import: {} } }),
    chat('5', '2027-07-10', { linked_to: null }),
  );
  equal([ids(assistantConversations('letter')), ids(assistantConversations(null))], [['3', '2', '1'], ['5']]);
});

test('a contact of the vault, one named with an email, and one as written', () => {
  vault({ id: 'maya', kind: 'contact', title: 'Maya Yahnek' }, { id: 'nameless', kind: 'contact', title: '' });
  equal(parseContact('@maya'), { id: 'maya', name: 'Maya Yahnek' });
  equal(parseContact('@nameless').name, 'untitled');
  equal(parseContact('@gone'), { name: '@gone' });
  equal(parseContact('Maya <maya@example.org>'), { name: 'Maya', email: 'maya@example.org' });
  equal(parseContact('Shady Pines Properties'), { name: 'Shady Pines Properties' });
});

test('an entity as the page keeps it', () => {
  const day = entityOf({ id: 'a', date: '2027-07-08', event: { end: '2027-07-10' } });
  equal([day.doc_id, day.date, day.end, day.all_day], ['a', new Date(2027, 6, 8), new Date(2027, 6, 10), true]);
  const task = entityOf({ id: 'b', date: '2027-07-08T10:00:00Z', task: { due: '2027-07-09', done: '2027-07-08T11:00:00Z' } });
  equal([task.all_day, task.due, task.done.getTime()], [false, new Date(2027, 6, 9), Date.UTC(2027, 6, 8, 11)]);
  equal(entityOf({ id: 'c' }).date, undefined);
});

test("the tests' entities are as the page keeps them", () => {
  for (const meta of [{ id: 'a', date: '2027-07-08', event: { end: '2027-07-10' } }, { id: 'b', date: '2027-07-08T10:00:00Z', task: { due: '2027-07-09' } }, { id: 'c' }]) {
    equal(entity(meta), entityOf(meta));
  }
  equal(projects.size, 0);
});
