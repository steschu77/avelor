import { equal, test } from './harness.js';
import { ordinal, t, textOf } from '../../static/core/text.js';

test('a text has its arguments filled in, a number as the locale writes it', () => {
  equal(t('test.plain'), 'plain');
  equal(t('test.named', { name: 'Page', n: 1234 }), 'Page of 1,234');
  equal(t('test.named', { name: 'Page' }), 'Page of {n}');
});

test('a key without a text is the text', () => {
  equal(t('test.gone'), 'test.gone');
});

test('a count picks its form', () => {
  equal([0, 1, 2].map(n => t('test.count', { n })), ['0 entries', '1 entry', '2 entries']);
});

test('an ordinal picks its form', () => {
  equal([1, 2, 3, 4, 11, 21].map(ordinal), ['1st', '2nd', '3rd', '4th', '11th', '21st']);
});

test("the host's text, its arguments texts too", () => {
  equal(textOf({ key: 'test.named', args: { name: { key: 'test.plain' }, n: { key: 'test.count', args: { n: 2 } } } }), 'plain of 2 entries');
  equal([textOf(null), textOf({})], ['', '']);
});

test("a list of the host's texts is joined", () => {
  equal(textOf([{ key: 'test.plain' }, { key: 'test.count', args: { n: 1 } }]), 'plain \u00b7 1 entry');
});
