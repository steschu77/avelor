import { equal, test } from './harness.js';
import { cutAtWords, fit, foundBy, prepare, wordsOf } from '../../static/core/match.js';

const fits = (text, name, also) => fit(wordsOf(text), prepare(name, also));

test('every word is in the name, in any order, case and accents aside', () => {
  equal([fits('CAFE', 'Caf\u00e9 receipt') > 0, fits('receipt cafe', 'Caf\u00e9 receipt') > 0, fits('cafe bill', 'Caf\u00e9 receipt') > 0], [true, true, false]);
});

test("the name's start counts most, then a word's start, then the inside, then the other words", () => {
  const order = [fits('new', 'New task'), fits('new', 'A new task'), fits('new', 'Renewal'), fits('new', 'Task', ['new'])];
  equal(order, [4.5, 3.5, 2, 1]);
});

test('a whole word counts more than one it starts', () => {
  equal([fits('task', 'Task list'), fits('tas', 'Task list')], [4.5, 4]);
});

test('without words everything fits', () => {
  equal(fits('', 'Anything'), 1);
});

test('a sign typed is the sign', () => {
  equal([fits('a.c', 'abc'), fits('(b', 'a (b)') > 0], [0, true]);
});

test('the other words a name was found by', () => {
  const words = wordsOf('scan zip');
  equal([foundBy(words, prepare('Rescan', ['zip', 'rar'])), foundBy(wordsOf('scan'), prepare('Rescan', ['scanner']))], [['zip'], []]);
});

test('a name cut where the words are in it', () => {
  equal(cutAtWords(wordsOf('ca'), 'Caf\u00e9 cab'), [{ text: 'Ca', hit: true }, { text: 'f\u00e9 ', hit: false }, { text: 'ca', hit: true }, { text: 'b', hit: false }]);
  equal(cutAtWords(wordsOf('ana'), 'banana'), [{ text: 'b', hit: false }, { text: 'anana', hit: true }]);
  equal(cutAtWords([], 'plain'), [{ text: 'plain', hit: false }]);
});

test('a folded name is as long as the name', () => {
  for (const name of ['\u0130stanbul', 'Stra\u00dfe', '\u1e9e', 'fi\ufb01', 'ok \ud83d\ude00', 'e\u0301']) equal(prepare(name).low.length, name.length);
});
