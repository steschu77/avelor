import { equal, test } from './harness.js';
import { addDays, isoInstant, localDay, parseDate, relativeDay } from '../../static/core/format.js';

test("a day is the local day's start, not UTC's", () => {
  const day = parseDate('2027-03-28');
  equal([day.getFullYear(), day.getMonth(), day.getDate(), day.getHours()], [2027, 2, 28, 0]);
});

test('a date-time is the instant it names, nothing is none', () => {
  equal(parseDate('2027-07-08T23:05:00Z').getTime(), Date.UTC(2027, 6, 8, 23, 5));
  equal([parseDate(''), parseDate(null)], [undefined, undefined]);
});

test('a local day as a date input has it', () => {
  equal(localDay(new Date(2027, 0, 5, 23, 59)), '2027-01-05');
});

test('an instant as the host writes it, to the second', () => {
  equal(isoInstant(new Date(Date.UTC(2027, 6, 8, 23, 5, 0, 123))), '2027-07-08T23:05:00Z');
});

test('a day as seen from today', () => {
  const today = new Date();
  equal([0, 1, -1, 3, -3].map(n => relativeDay(addDays(today, n))), ['today', 'tomorrow', 'yesterday', 'in 3 days', '3 days ago']);
  equal([relativeDay(undefined), relativeDay(new Date(NaN))], ['', '']);
});
