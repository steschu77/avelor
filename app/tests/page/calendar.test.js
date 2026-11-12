import { equal, test } from './harness.js';
import { vault } from './vault.js';
import { addDays, dayStart, isoInstant, localDay, parseDate } from '../../static/core/format.js';
import {
  calendarDayOf, dayOccurrences, dayTasks, eventsBetween, eventsOnDay, getDueTasks, getEvents, googleKept, googleLocked,
  googleType, isOverdue,
} from '../../static/model/calendar.js';
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

test('a timed recurring event keeps its time in its zone', () => {
  vault({ ...event('call', '2027-03-22T09:00:00Z', '2027-03-22T09:30:00Z', { schedule: ['RRULE:FREQ=WEEKLY;COUNT=2'], tz: 'Europe/Berlin' }) });
  equal(eventsBetween(parseDate('2027-03-20'), parseDate('2027-04-04'), getEvents()).map(ev => [isoInstant(ev.date), isoInstant(ev.end)]),
    [['2027-03-22T09:00:00Z', '2027-03-22T09:30:00Z'], ['2027-03-29T08:00:00Z', '2027-03-29T08:30:00Z']]);
});

test("a day's stack has an event once, a recurring one as its first occurrence there", () => {
  vault(timed('nightly', new Date(2027, 6, 5, 22), new Date(2027, 6, 6, 2), { schedule: ['RRULE:FREQ=DAILY'] }));
  const stack = dayOccurrences(parseDate('2027-07-08'));
  equal([stack.length, stack[0].date.getTime()], [1, new Date(2027, 6, 7, 22).getTime()]);
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

test("today's stack has the tasks due today and those overdue, another day's only its own", () => {
  const today = dayStart(new Date());
  vault(task('old', localDay(addDays(today, -9))), task('today', localDay(today)), task('tomorrow', localDay(addDays(today, 1))));
  const stack = day => Object.fromEntries(Object.entries(dayTasks(day)).map(([name, tasks]) => [name, ids(tasks)]));
  equal(stack(today), { due: ['today'], overdue: ['old'] });
  equal(stack(addDays(today, 1)), { due: ['tomorrow'], overdue: [] });
  equal(stack(addDays(today, -9)), { due: ['old'], overdue: [] });
});

test('the day the calendar shows an entity on', () => {
  vault(
    timed('event', new Date(2027, 6, 8, 22)), task('task', '2027-07-09T08:00:00Z'), task('done', '2027-07-09', { status: 'done' }),
    { id: 'letter', kind: 'document', title: '', date: '2027-07-08' },
  );
  equal(calendarDayOf(entities.get('event')), new Date(2027, 6, 8));
  equal(calendarDayOf(entities.get('task')), dayStart(new Date('2027-07-09T08:00:00Z')));
  equal([calendarDayOf(entities.get('done')), calendarDayOf(entities.get('letter'))], [null, null]);
});

test('Google keeps the events of a read-only calendar and of its own kinds', () => {
  const remote = type => ({ source: { remote: { remote: [{ 'event-id': 'x' }, ...(type ? [{ 'event-type': type }] : [])] } } });
  vault(
    { id: 'holidays', kind: 'calendar', title: 'Holidays', access: 'read-only', calendar: {} },
    { id: 'family', kind: 'calendar', title: 'Family', access: 'full', calendar: {} },
    { ...event('holiday', '2027-07-08', '2027-07-09', { calendar: 'holidays' }), ...remote() },
    { ...event('birthday', '2027-07-08', '2027-07-09', { calendar: 'family' }), ...remote('birthday') },
    { ...event('mailed', '2027-07-08', '2027-07-09', { calendar: 'family' }), ...remote('fromGmail') },
    event('own', '2027-07-08', '2027-07-09', {}),
  );
  const of = check => ['holiday', 'birthday', 'mailed', 'own'].map(id => check(entities.get(id)));
  equal(of(googleLocked), [true, true, false, false]);
  equal(of(googleKept), [false, true, false, false]);
  equal(of(googleType), [null, 'birthday', 'fromGmail', null]);
  equal([googleLocked(null), googleLocked(entities.get('family'))], [false, false]);
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
