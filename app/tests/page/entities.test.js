import { equal, test } from './harness.js';
import { vault } from './vault.js';
import {
  NO_PROJECT, entities, knownThreads, matchesFilter, projectsInOrder, projectsOf, threadMatchesFilter, threadMembers, threadOf,
  threadProjects, threads,
} from '../../static/model/entities.js';
import { store } from '../../static/model/state.js';

const project = (id, more = {}) => ({ id, kind: 'project', title: id, ...more });
const doc = (id, more = {}) => ({ id, kind: 'document', title: id, ...more });
const ids = list => list.map(one => one.id);
const FILTER = { selected_project: null, selected_labels: [] };
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
