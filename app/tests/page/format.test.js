import { equal, test } from './harness.js';
import {
  DATE_FORMAT, addDays, amountText, compareText, dayStart, daysBetween, durationText, fileSize, isoInstant, localDay,
  monthEndOf, monthStartOf, pageDate, parseAmount, parseDate, relativeDay, sameDay, signedAmount, weekStartOf,
  zonedInstant, zonedTime,
} from '../../static/core/format.js';
import { locale } from '../../static/core/text.js';

const instant = (day, time, zone) => isoInstant(zonedInstant(day, time, zone));

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

test("an instant's day and time in a zone", () => {
  const at = new Date(Date.UTC(2027, 6, 8, 23, 5));
  equal(zonedTime(at, 'America/Chicago'), { day: '2027-07-08', time: '18:05' });
  equal(zonedTime(at, 'Europe/Berlin'), { day: '2027-07-09', time: '01:05' });
  equal(zonedTime(new Date(Date.UTC(2027, 6, 8, 22)), 'Europe/Berlin'), { day: '2027-07-09', time: '00:00' });
  equal(zonedTime(new Date(2027, 6, 8, 9, 5), null), { day: '2027-07-08', time: '09:05' });
});

test('the instant of a wall time in a zone, midnight without a time', () => {
  equal(instant('2027-07-09', '09:20', 'Europe/Berlin'), '2027-07-09T07:20:00Z');
  equal(instant('2027-07-09', '', 'Europe/Berlin'), '2027-07-08T22:00:00Z');
  equal(zonedInstant('2027-07-09', '09:20', null), new Date(2027, 6, 9, 9, 20));
});

test('wall times around the hour left out in spring', () => {
  equal(instant('2027-03-28', '01:59', 'Europe/Berlin'), '2027-03-28T00:59:00Z');
  equal(instant('2027-03-28', '03:00', 'Europe/Berlin'), '2027-03-28T01:00:00Z');
  equal(instant('2027-03-14', '01:30', 'America/Chicago'), '2027-03-14T07:30:00Z');
  equal(instant('2027-03-14', '03:00', 'America/Chicago'), '2027-03-14T08:00:00Z');
});

test('a wall time of the hour left out is an hour later', () => {
  equal(instant('2027-03-28', '02:30', 'Europe/Berlin'), '2027-03-28T01:30:00Z');
});

test('each hour of the year is the wall time it was told as', () => {
  for (const zone of ['Europe/Berlin', 'America/Chicago', 'Pacific/Auckland', 'Asia/Kolkata']) {
    for (let hour = 0; hour < 365 * 24; hour++) {
      const told = zonedTime(new Date(Date.UTC(2027, 0, 1, hour)), zone);
      equal(zonedTime(zonedInstant(told.day, told.time, zone), zone), told);
    }
  }
});

test('a duration in hours and minutes', () => {
  const minutes = n => durationText(n * 6e4);
  equal([minutes(495), minutes(45), minutes(120), minutes(0), minutes(-45)],
    ['time.hours time.minutes', 'time.minutes', 'time.hours', 'time.minutes', 'time.minutes']);
});

test('a day as seen from today', () => {
  const today = new Date();
  equal([0, 1, -1, 3, -3].map(n => relativeDay(addDays(today, n))), ['today', 'tomorrow', 'yesterday', 'in 3 days', '3 days ago']);
  equal([relativeDay(undefined), relativeDay(new Date(NaN))], ['', '']);
});

test('no date is never', () => {
  equal(pageDate(null), 'date.never');
});

test('an amount typed as it is written', () => {
  for (const n of [0, 0.5, -60, 1150, 1234567.89, -0.01]) equal(parseAmount(amountText(n)), n);
  equal([parseAmount('\u221260'), parseAmount(' 60 '), parseAmount(''), parseAmount('sixty')], [-60, 60, null, null]);
});

test('an amount without one, and with its sign', () => {
  equal(amountText(null), '');
  equal([signedAmount(-9.99)[0], signedAmount(9.99)[0], signedAmount(0)[0]], ['\u2212', '+', '+']);
});

test("a file's size in its unit", () => {
  equal([812, 1023, 1024, 1024 * 1024 - 1, 1024 * 1024].map(fileSize), ['size.bytes', 'size.bytes', 'size.kb', 'size.kb', 'size.mb']);
});

test("the dates and the order are the page's language's, not the system's", () => {
  equal(DATE_FORMAT.resolvedOptions().locale, new Intl.DateTimeFormat(locale).resolvedOptions().locale);
  equal(['b', 'a', 'B'].sort(compareText), ['a', 'b', 'B']);
});
