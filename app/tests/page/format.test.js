import { equal, test } from './harness.js';
import { parseDate } from '../../static/core/format.js';

test("a day is the local day's start, not UTC's", () => {
  const day = parseDate('2027-03-28');
  equal([day.getFullYear(), day.getMonth(), day.getDate(), day.getHours()], [2027, 2, 28, 0]);
});

test('a date-time is the instant it names, nothing is none', () => {
  equal(parseDate('2027-07-08T23:05:00Z').getTime(), Date.UTC(2027, 6, 8, 23, 5));
  equal([parseDate(''), parseDate(null)], [undefined, undefined]);
});
