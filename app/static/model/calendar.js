// What the calendar shows (calendar.md): the dated events in the vault
// panel's filter, each recurring event expanded into its occurrences (the RFC 5545
// lines Google keeps, with their exceptions), and the open tasks on their
// due day -- for the grid (features/lenses/calendar.js), the panel's day
// view (features/panel/panel.js) and the selection (`calendarDayOf`,
// features/selection.js).

import { addDays, dayStart, parseDate, sameDay } from '../core/format.js';
import { entities, matchesFilter } from './entities.js';
import { dayKey, scheduleStarts } from './schedule.js';

// ----------------------------------------------------------------------------
function compareEventsByDate(a, b) {
    return a.date.getTime() - b.date.getTime();
}

// ----------------------------------------------------------------------------
// An entity the calendar shows: a dated event in the vault panel's filter;
// events deleted in Google (status `cancelled`) aren't.
function shownInCalendar(doc) {
    return doc?.kind === 'event' && !!doc.date && doc.event?.status !== 'cancelled' && matchesFilter(doc);
}

// ----------------------------------------------------------------------------
// The events to show, a recurring event once (see `eventsBetween`).
export function getEvents() {
    return [...entities.values()]
        .filter(doc => shownInCalendar(doc))
        .sort(compareEventsByDate);
}

// ----------------------------------------------------------------------------
// An entity the calendar shows as a task on its due day: an open task (a
// task role with a due date, not done) in the vault panel's filter, of any
// kind but an event, which its start places.
function dueInCalendar(doc) {
    return doc?.kind !== 'event' && !isNaN(doc?.due) && doc.task?.status !== 'done' && matchesFilter(doc);
}

// ----------------------------------------------------------------------------
// The tasks the calendar shows, by due date, then title.
export function getDueTasks() {
    return [...entities.values()]
        .filter(doc => dueInCalendar(doc))
        .sort((a, b) => a.due - b.due || (a.title ?? '').localeCompare(b.title ?? ''));
}

// ----------------------------------------------------------------------------
// Whether the task `doc` is overdue: open, and due on a day before today (a
// task due today isn't, whatever its time).
export function isOverdue(doc, today = dayStart(new Date())) {
    return doc.task?.status !== 'done' && doc.due < today;
}

// ----------------------------------------------------------------------------
// The tasks the day stack lists for `day`: those due on it, and on today
// also those due before it, still open.
export function dayTasks(day) {
    const tasks = getDueTasks();
    const today = dayStart(new Date());
    return {
        due: tasks.filter(task => sameDay(task.due, day)),
        overdue: sameDay(day, today) ? tasks.filter(task => task.due < today) : [],
    };
}

// ----------------------------------------------------------------------------
// The day the calendar shows `doc` on: an event's (first) start, an open
// task's due day; null where it shows nowhere.
export function calendarDayOf(doc) {
    if (shownInCalendar(doc)) return dayStart(doc.date);
    if (dueInCalendar(doc)) return dayStart(doc.due);
    return null;
}

// ----------------------------------------------------------------------------
// The events on `day` as the panel's day stack shows them: each entity once
// (a recurring event as its occurrence there, the first if two overlap the day), by
// start.
export function dayOccurrences(day) {
    const seen = new Set();
    return eventsOnDay(day, getEvents()).filter(ev => !seen.has(ev.doc_id) && seen.add(ev.doc_id));
}

// ----------------------------------------------------------------------------
// Event helpers
export function eventsOnDay(d, evs) {
    return eventsBetween(dayStart(d), addDays(d, 1), evs);
}

// ----------------------------------------------------------------------------
// The events of `evs` overlapping [s, e), each recurring event (`event.schedule`)
// as its occurrences there, by start.
export function eventsBetween(s, e, evs) {
    return evs.flatMap(ev => occurrencesOf(ev, s, e)).sort(compareEventsByDate);
}

// ----------------------------------------------------------------------------
// `ev` if it overlaps [s, e); for a recurring event, a copy per occurrence that does,
// with its own `date` and `end` (and `occurrence` set, so it isn't
// expanded again). An occurrence Google moved, retitled or cancelled is as
// its exception has it (`exception`): a moved one shows where it went, also
// from a day after `e`.
function occurrencesOf(ev, s, e) {
    const rules = ev.occurrence ? null : ev.event?.schedule;
    if (!rules?.length) {
        return evtStart(ev) < e && evtEnd(ev) > s ? [ev] : [];
    }
    const duration = evtEnd(ev).getTime() - evtStart(ev).getTime();
    const exceptions = exceptionsOf(ev);
    let before = e;
    for (const x of exceptions.values()) {
        if (x.start && x.start < e && x.original >= before) before = addDays(x.original, 1);
    }
    return scheduleStarts(evtStart(ev), rules, before)
        .flatMap(start => {
            const exception = exceptions.get(dayKey(start));
            if (exception?.cancelled) return [];
            const date = exception?.start ?? start;
            const end = exception?.end ?? new Date(date.getTime() + duration);
            return [{ ...ev, date, end, title: exception?.title ?? ev.title, occurrence: true, exception }];
        })
        .filter(occ => occ.date < e && occ.end > s);
}

// ----------------------------------------------------------------------------
// A recurring event's exceptions (`event.exceptions`) by the local day of the
// occurrence each replaces (`dayKey`, as EXDATEs are matched): `original`,
// `start` and `end` as Dates, `cancelled`, `title`, `location`.
function exceptionsOf(ev) {
    const byDay = new Map();
    for (const x of ev.event?.exceptions ?? []) {
        const original = parseDate(x.original);
        if (!original || isNaN(original)) continue;
        byDay.set(dayKey(original), {
            original,
            start: parseDate(x.start),
            end: parseDate(x.end),
            cancelled: x.status === 'cancelled',
            title: x.title,
            location: x.location,
        });
    }
    return byDay;
}

// ----------------------------------------------------------------------------
function evtStart(ev) {
    return ev.date;
}

// ----------------------------------------------------------------------------
const DEFAULT_DURATION = 60 * 60 * 1000; // 1 hour
function evtEnd(ev) {
    return new Date(ev.end?.getTime() || (ev.date.getTime() + DEFAULT_DURATION));
}
