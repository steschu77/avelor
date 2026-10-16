import { equal, test } from './harness.js';
import { vault } from './vault.js';
import { addDays, dayStart, isoInstant, localDay, parseDate } from '../../static/core/format.js';
import { eventsBetween, eventsOnDay, getDueTasks, getEvents, isOverdue } from '../../static/model/calendar.js';
import { entities } from '../../static/model/entities.js';
import { store } from '../../static/model/state.js';

// An event on its days, `end` the morning after the last; or at its times.
const event = (id, date, end, more = {}) => ({ id, kind: 'event', title: id, date, event: { end, ...more } });
const timed = (id, start, end, more) => event(id, isoInstant(start), end && isoInstant(end), more);
const task = (id, due, more = {}) => ({ id, kind: 'task', title: id, task: { status: 'open', due, ...more } });
const on = day => eventsOnDay(parseDate(day), getEvents()).map(ev => ev.id);
const ids = list => list.map(doc => doc.id);

test('a timed event is on each day it runs into', () => {
  vault(timed('night', new Date(2027, 6, 8, 22), new Date(2027, 6, 9, 2)));
  equal([on('2027-07-07'), on('2027-07-08'), on('2027-07-09'), on('2027-07-10')], [[], ['night'], ['night'], []]);
});

test("an event that ends at midnight isn't on the next day", () => {
  vault(timed('late', new Date(2027, 6, 8, 22), new Date(2027, 6, 9)));
  equal([on('2027-07-08'), on('2027-07-09')], [['late'], []]);
});

test('an event without an end takes an hour', () => {
  vault(timed('short', new Date(2027, 6, 8, 23, 30)));
  equal([on('2027-07-08'), on('2027-07-09')], [['short'], ['short']]);
});

test('an all-day event ends the morning after its last day', () => {
  vault(event('days', '2027-07-08', '2027-07-10'));
  equal([on('2027-07-07'), on('2027-07-08'), on('2027-07-09'), on('2027-07-10')], [[], ['days'], ['days'], []]);
});

test('the events come by their start', () => {
  vault(timed('noon', new Date(2027, 6, 8, 12)), timed('early', new Date(2027, 6, 8, 8)), event('all day', '2027-07-08', '2027-07-09'));
  equal(on('2027-07-08'), ['all day', 'early', 'noon']);
});

test('a cancelled event, one without a date and one outside the filter are not shown', () => {
  vault(
    event('shown', '2027-07-08', '2027-07-09', {}),
    event('cancelled', '2027-07-08', '2027-07-09', { status: 'cancelled' }),
    { id: 'undated', kind: 'event', title: '', event: {} },
    { ...event('other', '2027-07-08', '2027-07-09'), projects: ['p'] },
    { id: 'p', kind: 'project', title: 'P' },
    { id: 'letter', kind: 'document', title: '', date: '2027-07-08' },
  );
  equal(ids(getEvents()), ['shown', 'other']);
  store.setState({ selected_project: 'p' });
  equal(ids(getEvents()), ['other']);
});

const WEEKLY = { schedule: ['RRULE:FREQ=WEEKLY;COUNT=3'] }; // July 5, 12 and 19
const occurrences = (from, to) => eventsBetween(parseDate(from), parseDate(to), getEvents())
  .map(ev => [localDay(ev.date), localDay(ev.end), ev.title]);

test('a recurring event is each of its occurrences, as long as the first', () => {
  vault(event('weekly', '2027-07-05', '2027-07-07', WEEKLY));
  equal(occurrences('2027-07-01', '2027-08-01'), [
    ['2027-07-05', '2027-07-07', 'weekly'], ['2027-07-12', '2027-07-14', 'weekly'], ['2027-07-19', '2027-07-21', 'weekly'],
  ]);
  equal(occurrences('2027-07-06', '2027-07-12'), [['2027-07-05', '2027-07-07', 'weekly']]);
});

test('an occurrence moved, retitled or cancelled is as its exception has it', () => {
  vault(event('weekly', '2027-07-05', '2027-07-06', { ...WEEKLY, exceptions: [
    { original: '2027-07-12', start: '2027-07-14', end: '2027-07-16', title: 'moved' },
    { original: '2027-07-19', status: 'cancelled' },
  ] }));
  equal(occurrences('2027-07-01', '2027-08-01'), [['2027-07-05', '2027-07-06', 'weekly'], ['2027-07-14', '2027-07-16', 'moved']]);
  equal([on('2027-07-12'), on('2027-07-15'), on('2027-07-19')], [[], ['weekly'], []]);
});

test('an occurrence moved into the days asked for is there, from before and from after', () => {
  vault(event('weekly', '2027-07-05', '2027-07-06', { ...WEEKLY, exceptions: [
    { original: '2027-07-05', start: '2027-07-14', end: '2027-07-15' },
    { original: '2027-07-19', start: '2027-07-15', end: '2027-07-16' },
  ] }));
  equal(occurrences('2027-07-13', '2027-07-17').map(([from]) => from), ['2027-07-14', '2027-07-15']);
});

test('the open tasks with a due day, by the day, then by title', () => {
  vault(
    task('b', '2027-07-09'), task('a', '2027-07-09'), task('first', '2027-07-08'), task('done', '2027-07-08', { status: 'done' }),
    task('whenever'), { ...event('meeting', '2027-07-08', '2027-07-09'), task: { status: 'open', due: '2027-07-08' } },
  );
  equal(ids(getDueTasks()), ['first', 'a', 'b']);
});

test('a task due today is not overdue, whatever its time', () => {
  const today = dayStart(new Date());
  vault(
    task('yesterday', localDay(addDays(today, -1))), task('today', localDay(today)), task('now', isoInstant(new Date(today.getTime() + 1))),
    task('done', localDay(addDays(today, -1)), { status: 'done' }),
  );
  equal([...entities.values()].map(doc => isOverdue(doc)), [true, false, false, false]);
});

test('an occurrence is not expanded again', () => {
  vault(event('weekly', '2027-07-05', '2027-07-06', WEEKLY));
  const july = eventsBetween(parseDate('2027-07-01'), parseDate('2027-08-01'), getEvents());
  equal(eventsOnDay(parseDate('2027-07-12'), july).map(ev => localDay(ev.date)), ['2027-07-12']);
});

test('an occurrence that starts as the days end, or ends as they start, is not in them', () => {
  vault(event('weekly', '2027-07-05', '2027-07-06', WEEKLY));
  equal([occurrences('2027-07-06', '2027-07-12'), occurrences('2027-07-13', '2027-07-19')], [[], []]);
});
