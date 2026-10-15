import { equal, test } from './harness.js';
import { vault } from './vault.js';
import { NO_PROJECT, entities, matchesFilter, projectsInOrder, projectsOf } from '../../static/model/entities.js';
import { store } from '../../static/model/state.js';

const project = (id, more = {}) => ({ id, kind: 'project', title: id, ...more });
const doc = (id, more = {}) => ({ id, kind: 'document', title: id, ...more });
const ids = list => list.map(one => one.id);
const FILTER = { selected_project: null };
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

test("the store's filter is the one without another", () => {
  vault(project('p'), doc('a', { projects: ['p'] }), doc('b'));
  store.setState({ selected_project: 'p' });
  equal([...entities.values()].map(one => matchesFilter(one)), [true, false]);
});
