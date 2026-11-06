import { equal, test } from './harness.js';
import {
  addDays, amountText, dayStart, daysBetween, fileSize, isoInstant, localDay, monthEndOf, monthStartOf, pageDate, parseDate,
  relativeDay, sameDay, signedAmount, weekStartOf,
} from '../../static/core/format.js';

test("a day is the local day's start, not UTC's", () => {
  const day = parseDate('2027-03-28');
  equal([day.getFullYear(), day.getMonth(), day.getDate(), day.getHours()], [2027, 2, 28, 0]);
});

test('a date-time is the instant it names, nothing is none', () => {
  equal(parseDate('2027-07-08T23:05:00Z').getTime(), Date.UTC(2027, 6, 8, 23, 5));
  equal([parseDate(''), parseDate(null)], [undefined, undefined]);
});

test('each day of the year is a day after the one before', () => {
  const first = new Date(2027, 0, 1);
  for (let n = 0; n < 366; n++) {
    const day = addDays(first, n);
    equal([daysBetween(first, day), day.getHours(), daysBetween(addDays(day, 1), day)], [n, 0, -1]);
  }
});

test('the days between two times are those between their days', () => {
  equal(daysBetween(new Date(2027, 2, 27, 23, 59), new Date(2027, 2, 28, 0, 1)), 1);
  equal(daysBetween(new Date(2027, 2, 28, 0, 1), new Date(2027, 2, 28, 23, 59)), 0);
});

test('where a day, a week and a month start and end', () => {
  equal(dayStart(new Date(2027, 2, 28, 13, 45)), new Date(2027, 2, 28));
  equal([sameDay(new Date(2027, 2, 28, 1), new Date(2027, 2, 28, 23)), sameDay(new Date(2027, 2, 28), new Date(2026, 2, 28))], [true, false]);
  equal([weekStartOf(new Date(2027, 2, 28)), weekStartOf(new Date(2027, 2, 22))], [new Date(2027, 2, 22), new Date(2027, 2, 22)]);
  equal([monthStartOf(new Date(2028, 1, 10)), monthEndOf(new Date(2028, 1, 10))], [new Date(2028, 1, 1), new Date(2028, 1, 29)]);
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

test('no date is never', () => {
  equal(pageDate(null), 'date.never');
});

test('an amount without one, and with its sign', () => {
  equal(amountText(null), '');
  equal([signedAmount(-9.99)[0], signedAmount(9.99)[0], signedAmount(0)[0]], ['\u2212', '+', '+']);
});

test("a file's size in its unit", () => {
  equal([812, 1023, 1024, 1024 * 1024 - 1, 1024 * 1024].map(fileSize), ['size.bytes', 'size.bytes', 'size.kb', 'size.kb', 'size.mb']);
});
