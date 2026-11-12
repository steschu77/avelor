import { equal, test } from './harness.js';
import { isoInstant, localDay, parseDate } from '../../static/core/format.js';
import {
  byDay, eventRuleOf, eventScheduleLines, icalDate, parseSchedule, periodOf, ruleSupported, scheduleLine, scheduleStarts,
  timesAYear, weekdayOf,
} from '../../static/model/schedule.js';

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

test('the occurrences before a day are left out, and count', () => {
  const starts = scheduleStarts(parseDate('2027-01-04'), ['RRULE:FREQ=WEEKLY;COUNT=10'], FOREVER, { after: parseDate('2027-03-01') });
  equal(starts.map(localDay), ['2027-03-01', '2027-03-08']);
});

test("a timed one keeps its time on the clock of its zone", () => {
  const first = new Date('2027-03-22T09:00:00Z');
  const starts = scheduleStarts(first, ['RRULE:FREQ=WEEKLY;COUNT=3'], FOREVER, { zone: 'Europe/Berlin' });
  equal(starts.map(isoInstant), ['2027-03-22T09:00:00Z', '2027-03-29T08:00:00Z', '2027-04-05T08:00:00Z']);
});

test("the weekday of a timed one is its zone's", () => {
  const first = new Date('2027-07-08T23:05:00Z'); // Friday in Berlin, Thursday in Chicago
  const weekly = zone => scheduleStarts(first, ['RRULE:FREQ=WEEKLY;BYDAY=FR;COUNT=1'], FOREVER, { zone }).map(isoInstant);
  equal(weekly('Europe/Berlin'), ['2027-07-08T23:05:00Z']);
  equal(weekly('America/Chicago'), ['2027-07-09T23:05:00Z']);
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

test("a recurring payment's period as a rule, and back", () => {
  const from = new Date(2027, 2, 24); // a Wednesday
  const line = schedule => scheduleLine(schedule, from);
  equal(line({ period: 'week' }), 'RRULE:FREQ=WEEKLY;BYDAY=WE');
  equal(line({ period: 'week', days: ['SU', 'MO', 'MO'] }), 'RRULE:FREQ=WEEKLY;BYDAY=MO,SU');
  equal(line({ period: 'month' }), 'RRULE:FREQ=MONTHLY;BYMONTHDAY=24');
  equal(line({ period: 'quarter', day: 15 }), 'RRULE:FREQ=MONTHLY;INTERVAL=3;BYMONTHDAY=15');
  equal(line({ period: 'half', day: 1 }), 'RRULE:FREQ=MONTHLY;INTERVAL=6;BYMONTHDAY=1');
  equal(line({ period: 'year', day: 31 }), 'RRULE:FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=31');
  for (const schedule of [
    { period: 'week', days: ['MO', 'SU'], day: null }, { period: 'month', days: null, day: 24 },
    { period: 'quarter', days: null, day: 15 }, { period: 'half', days: null, day: 1 }, { period: 'year', days: null, day: 31 },
  ]) equal(periodOf([line(schedule)]), schedule);
});

test("a day's weekday as a rule names it", () => {
  equal([weekdayOf(new Date(2027, 2, 24)), weekdayOf(new Date(2027, 2, 28))], ['WE', 'SU']);
});

test('any other rule is no period', () => {
  for (const line of [
    'RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO', 'RRULE:FREQ=WEEKLY', 'RRULE:FREQ=MONTHLY;BYDAY=2TU', 'RRULE:FREQ=MONTHLY;BYMONTHDAY=-1',
    'RRULE:FREQ=MONTHLY;BYMONTHDAY=1,15', 'RRULE:FREQ=MONTHLY;INTERVAL=2;BYMONTHDAY=1', 'RRULE:FREQ=DAILY',
    'RRULE:FREQ=MONTHLY;BYMONTHDAY=1;COUNT=3', 'RRULE:FREQ=WEEKLY;BYDAY=MO,2TU',
  ]) equal(periodOf([line]), null);
  equal([periodOf([]), periodOf(undefined), periodOf(['RRULE:FREQ=MONTHLY;BYMONTHDAY=1', 'EXDATE:20270301'])], [null, null, null]);
});

test('how many times a year a schedule repeats', () => {
  equal([
    'RRULE:FREQ=DAILY', 'RRULE:FREQ=WEEKLY;BYDAY=MO,WE', 'RRULE:FREQ=WEEKLY;INTERVAL=2', 'RRULE:FREQ=MONTHLY;BYMONTHDAY=1,15',
    'RRULE:FREQ=MONTHLY;INTERVAL=3;BYMONTHDAY=1', 'RRULE:FREQ=YEARLY;BYMONTH=3,9;BYMONTHDAY=1', 'RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=1SU,2SU',
    'RRULE:FREQ=HOURLY',
  ].map(line => timesAYear([line])), [365, 104, 26, 24, 4, 2, 2, null]);
});

test("an event's schedule as its card has it", () => {
  const start = new Date('2027-07-08T23:05:00Z');
  const zone = 'Europe/Berlin';
  const rule = (...lines) => eventRuleOf(lines, start, zone);
  equal(rule(), { period: 'once', days: null, until: null, count: null, rest: [] });
  equal(rule('RRULE:FREQ=DAILY'), { period: 'day', days: null, until: null, count: null, rest: [] });
  equal(rule('RRULE:FREQ=WEEKLY;BYDAY=MO,FR;COUNT=8'), { period: 'week', days: ['MO', 'FR'], until: null, count: 8, rest: [] });
  equal(rule('RRULE:FREQ=YEARLY;UNTIL=20300708T215900Z', 'EXDATE:20280708T230500Z'),
    { period: 'year', days: null, until: '2030-07-08', count: null, rest: ['EXDATE:20280708T230500Z'] });
  equal(rule('RRULE:FREQ=MONTHLY;BYMONTHDAY=9').period, 'month'); // the 9th in Berlin
});

test('a schedule its card has no form for', () => {
  const start = new Date('2027-07-08T23:05:00Z');
  for (const lines of [
    ['RRULE:FREQ=WEEKLY;INTERVAL=2'], ['RRULE:FREQ=MONTHLY;BYDAY=2TU'], ['RRULE:FREQ=MONTHLY;BYMONTHDAY=8'],
    ['RRULE:FREQ=DAILY;COUNT=3;UNTIL=20300708'], ['RRULE:FREQ=WEEKLY;BYDAY=MO,2TU'], ['RRULE:FREQ=HOURLY'],
    ['RRULE:FREQ=DAILY', 'RRULE:FREQ=WEEKLY'], ['RRULE:FREQ=DAILY', 'RDATE:20270801'],
  ]) equal(eventRuleOf(lines, start, 'Europe/Berlin'), null);
});

test("an event's schedule as its lines", () => {
  const start = new Date('2027-07-08T23:05:00Z');
  const lines = (schedule, zone = 'Europe/Berlin', allDay = false) => eventScheduleLines(schedule, start, zone, allDay);
  equal(lines({ period: 'once', rest: ['EXDATE:20270708'] }), []);
  equal(lines({ period: 'week' }), ['RRULE:FREQ=WEEKLY;BYDAY=FR']);
  equal(lines({ period: 'week' }, 'America/Chicago'), ['RRULE:FREQ=WEEKLY;BYDAY=TH']);
  equal(lines({ period: 'week', days: ['FR', 'MO'], until: '2027-07-30' }), ['RRULE:FREQ=WEEKLY;BYDAY=MO,FR;UNTIL=20270730T215900Z']);
  equal(lines({ period: 'day', until: '2027-07-30' }, null, true), ['RRULE:FREQ=DAILY;UNTIL=20270730']);
  equal(lines({ period: 'month', count: 5, until: '2027-07-30', rest: ['EXDATE:20270808'] }), ['RRULE:FREQ=MONTHLY;COUNT=5', 'EXDATE:20270808']);
});

test("a timed event's last day comes back as it was set", () => {
  const start = new Date('2027-07-08T23:05:00Z');
  for (const zone of ['Europe/Berlin', 'America/Chicago', 'Pacific/Auckland', null]) {
    const schedule = { period: 'day', days: null, until: '2027-12-31', count: null, rest: [] };
    equal(eventRuleOf(eventScheduleLines(schedule, start, zone, false), start, zone), schedule);
  }
});

test('a timed one to a day is on that day too', () => {
  const starts = scheduleStarts(new Date(2027, 2, 30, 10), ['RRULE:FREQ=DAILY;UNTIL=20270401'], FOREVER);
  equal(starts.map(localDay), ['2027-03-30', '2027-03-31', '2027-04-01']);
});

test('monthly on a day that is one of some weekdays', () => {
  equal(days('2027-08-13', 'RRULE:FREQ=MONTHLY;BYDAY=FR,MO;BYMONTHDAY=13;COUNT=2'), ['2027-08-13', '2027-09-13']);
});

test("an all-day event's last day comes back as it was set", () => {
  equal(eventRuleOf(['RRULE:FREQ=DAILY;UNTIL=20270730'], new Date(2027, 6, 8), null).until, '2027-07-30');
  equal(eventRuleOf(['RRULE:FREQ=DAILY;UNTIL=20270730'], new Date('2027-07-08T23:05:00Z'), 'Pacific/Auckland').until, '2027-07-30');
});
