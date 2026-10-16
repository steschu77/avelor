import { equal, test } from './harness.js';
import { localDay, parseDate } from '../../static/core/format.js';
import { byDay, icalDate, parseSchedule, ruleSupported, scheduleStarts } from '../../static/model/schedule.js';

const FOREVER = new Date(2040, 0, 1);

// The days the all-day schedule of `lines` falls on from `first`.
function days(first, ...lines) {
  return scheduleStarts(parseDate(first), lines, FOREVER).map(localDay);
}

test('every day, across the change of the clock', () => {
  equal(days('2027-03-26', 'RRULE:FREQ=DAILY;COUNT=4'), ['2027-03-26', '2027-03-27', '2027-03-28', '2027-03-29']);
});

test('every other day to a day, which is in it', () => {
  equal(days('2027-03-26', 'RRULE:FREQ=DAILY;INTERVAL=2;UNTIL=20270401'), ['2027-03-26', '2027-03-28', '2027-03-30', '2027-04-01']);
});

test('every day on some weekdays', () => {
  equal(days('2027-03-25', 'RRULE:FREQ=DAILY;BYDAY=MO,FR;COUNT=3'), ['2027-03-26', '2027-03-29', '2027-04-02']);
});

test('weekly on its weekdays, in the order of the week', () => {
  equal(days('2027-03-22', 'RRULE:FREQ=WEEKLY;BYDAY=FR,MO,SU;COUNT=5'), ['2027-03-22', '2027-03-26', '2027-03-28', '2027-03-29', '2027-04-02']);
});

test("a week's days before the first aren't in it", () => {
  equal(days('2027-03-24', 'RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=3'), ['2027-03-24', '2027-03-29', '2027-03-31']);
});

test("weekly without days is on the first's weekday, every other week", () => {
  equal(days('2027-03-28', 'RRULE:FREQ=WEEKLY;INTERVAL=2;COUNT=3'), ['2027-03-28', '2027-04-11', '2027-04-25']);
});

test('monthly on a day leaves out the months without it', () => {
  equal(days('2027-01-31', 'RRULE:FREQ=MONTHLY;COUNT=4'), ['2027-01-31', '2027-03-31', '2027-05-31', '2027-07-31']);
});

test('monthly on days of the month, the last counted from its end', () => {
  equal(days('2027-01-15', 'RRULE:FREQ=MONTHLY;BYMONTHDAY=-1,15;COUNT=4'), ['2027-01-15', '2027-01-31', '2027-02-15', '2027-02-28']);
});

test('monthly on a weekday, the second and the last', () => {
  equal(days('2027-01-12', 'RRULE:FREQ=MONTHLY;BYDAY=2TU;COUNT=3'), ['2027-01-12', '2027-02-09', '2027-03-09']);
  equal(days('2027-01-29', 'RRULE:FREQ=MONTHLY;BYDAY=-1FR;COUNT=3'), ['2027-01-29', '2027-02-26', '2027-03-26']);
});

test('monthly on a fifth weekday leaves out the months without one', () => {
  equal(days('2027-01-29', 'RRULE:FREQ=MONTHLY;BYDAY=5FR;COUNT=2'), ['2027-01-29', '2027-04-30']);
});

test('monthly on a day that is a weekday', () => {
  equal(days('2027-08-13', 'RRULE:FREQ=MONTHLY;BYDAY=FR;BYMONTHDAY=13;COUNT=2'), ['2027-08-13', '2028-10-13']);
});

test('yearly on February 29 leaves out the years without it', () => {
  equal(days('2028-02-29', 'RRULE:FREQ=YEARLY;COUNT=2'), ['2028-02-29', '2032-02-29']);
});

test('yearly in some months', () => {
  equal(days('2027-03-01', 'RRULE:FREQ=YEARLY;BYMONTH=3,9;BYMONTHDAY=1;COUNT=3'), ['2027-03-01', '2027-09-01', '2028-03-01']);
});

test('a date left out still counts', () => {
  equal(days('2027-03-22', 'RRULE:FREQ=WEEKLY;COUNT=3', 'EXDATE;VALUE=DATE:20270329'), ['2027-03-22', '2027-04-05']);
});

test('a rule the page does not expand is its first occurrence', () => {
  equal(days('2027-03-22', 'RRULE:FREQ=HOURLY'), ['2027-03-22']);
  equal(days('2027-03-22', 'RRULE:FREQ=MONTHLY;BYSETPOS=1'), ['2027-03-22']);
  equal(days('2027-03-22', 'EXDATE:20270322'), ['2027-03-22']);
  equal(scheduleStarts(parseDate('2027-03-22'), ['RRULE:FREQ=HOURLY'], parseDate('2027-03-22')), []);
});

test('the occurrences end before a day', () => {
  equal(scheduleStarts(parseDate('2027-03-22'), ['RRULE:FREQ=DAILY'], parseDate('2027-03-24')).map(localDay), ['2027-03-22', '2027-03-23']);
});

test("a timed one without a zone keeps its time on this PC's clock", () => {
  const starts = scheduleStarts(new Date(2027, 0, 4, 10, 30), ['RRULE:FREQ=WEEKLY;COUNT=52'], FOREVER);
  equal([...new Set(starts.map(start => `${start.getHours()}:${start.getMinutes()}`))], ['10:30']);
  equal(starts.length, 52);
});

test('a schedule in its parts', () => {
  const { rule, written, excluded } = parseSchedule(['RRULE:FREQ=WEEKLY;BYDAY=MO,WE', 'EXDATE;TZID=Europe/Berlin:20270329T100000,20270405T100000']);
  equal(rule, { FREQ: 'WEEKLY', BYDAY: 'MO,WE' });
  equal([written, excluded.size], ['RRULE:FREQ=WEEKLY;BYDAY=MO,WE', 2]);
  equal(parseSchedule([]).rule, null);
});

test('the rules the page expands', () => {
  const supported = line => ruleSupported(parseSchedule([line]).rule);
  equal([
    supported('RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO;WKST=MO'), supported('RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=1SU'),
    supported('RRULE:FREQ=YEARLY;BYDAY=1SU'), supported('RRULE:FREQ=SECONDLY'), supported('RRULE:FREQ=DAILY;BYHOUR=9'), ruleSupported(null),
  ], [true, true, false, false, false, false]);
});

test("a rule's weekday, with its place in the month", () => {
  equal([byDay('MO'), byDay('2TU'), byDay('-1FR')], [{ weekday: 1, nth: 0 }, { weekday: 2, nth: 2 }, { weekday: 5, nth: -1 }]);
});

test("a calendar's date and date-time", () => {
  equal(icalDate('20270322'), new Date(2027, 2, 22));
  equal(icalDate('20270322', true), new Date(2027, 2, 22, 23, 59, 59));
  equal(icalDate('20270322T100530'), new Date(2027, 2, 22, 10, 5, 30));
  equal(icalDate('20270322T100530Z'), new Date(Date.UTC(2027, 2, 22, 10, 5, 30)));
});

test('a timed one to a day is on that day too', () => {
  const starts = scheduleStarts(new Date(2027, 2, 30, 10), ['RRULE:FREQ=DAILY;UNTIL=20270401'], FOREVER);
  equal(starts.map(localDay), ['2027-03-30', '2027-03-31', '2027-04-01']);
});

test('monthly on a day that is one of some weekdays', () => {
  equal(days('2027-08-13', 'RRULE:FREQ=MONTHLY;BYDAY=FR,MO;BYMONTHDAY=13;COUNT=2'), ['2027-08-13', '2027-09-13']);
});
