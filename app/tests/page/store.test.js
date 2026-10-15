import { equal, test } from './harness.js';
import { createStore } from '../../static/core/store.js';

test('a subscriber is called at once, then when its value changes', () => {
  const store = createStore({ a: 1, b: 1 });
  const seen = [];
  store.subscribe(state => state.a, value => seen.push(value));
  store.setState({ b: 2 });
  store.setState({ a: 2 });
  store.setState({ a: 2 });
  equal(seen, [1, 2]);
  equal(store.getState(), { a: 2, b: 2 });
});

test('a value is another one by identity', () => {
  const store = createStore({ list: [], n: NaN });
  const seen = [];
  store.subscribe(state => state.list, () => seen.push('list'));
  store.subscribe(state => state.n, () => seen.push('n'));
  store.setState({ list: [], n: NaN });
  equal(seen, ['list', 'n', 'list']);
});
