// What the calendar shows: the dated events in the vault panel's filter,
// each series expanded into its occurrences (the RFC 5545 lines Google
// keeps, with their exceptions), and the open tasks on their due day.

import { dayStart, parseDate } from '../core/format.js';
import { entities, matchesFilter } from './entities.js';

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
// The events to show, a series once (see `eventsBetween`).
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
// Local-time date helpers
export const sameDay = (a, b) =>
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate();
const weekFirstDay = 1; // Monday as the first day of the week
export const weekStartOf = d => addDays(d, -((d.getDay() - weekFirstDay + 7) % 7));
export const addDays = (d, n) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);

// ----------------------------------------------------------------------------
// Event helpers
export function eventsOnDay(d, evs) {
    return eventsBetween(dayStart(d), addDays(d, 1), evs);
}

// ----------------------------------------------------------------------------
// The events of `evs` overlapping [s, e), each series (`event.recurrence`)
// as its occurrences there, by start.
export function eventsBetween(s, e, evs) {
    return evs.flatMap(ev => occurrencesOf(ev, s, e)).sort(compareEventsByDate);
}

// ----------------------------------------------------------------------------
// `ev` if it overlaps [s, e); for a series, a copy per occurrence that does,
// with its own `date` and `end` (and `occurrence` set, so it isn't
// expanded again). An occurrence Google moved, retitled or cancelled is as
// its exception has it (`exception`): a moved one shows where it went, also
// from a day after `e`.
function occurrencesOf(ev, s, e) {
    const rules = ev.occurrence ? null : ev.event?.recurrence;
    if (!rules?.length) {
        return evtStart(ev) < e && evtEnd(ev) > s ? [ev] : [];
    }
    const duration = evtEnd(ev).getTime() - evtStart(ev).getTime();
    const exceptions = exceptionsOf(ev);
    let before = e;
    for (const x of exceptions.values()) {
        if (x.start && x.start < e && x.original >= before) before = addDays(x.original, 1);
    }
    return recurrenceStarts(evtStart(ev), rules, before)
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
// A series' exceptions (`event.exceptions`) by the local day of the
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
// Series: the RFC 5545 lines Google keeps (`RRULE:`, `EXDATE:`), expanded
// in local time from the first start, so 10:00 stays 10:00 across DST.
// Supported: FREQ DAILY/WEEKLY/MONTHLY/YEARLY with INTERVAL, COUNT, UNTIL,
// BYDAY (weekdays; with an ordinal such as 2TU or -1FR in a month), BYMONTHDAY,
// BYMONTH, and EXDATE. Any other rule shows the first occurrence only.
const WEEKDAYS = ['SU', 'MO', 'TU', 'WE', 'TH', 'FR', 'SA'];
const RRULE_PARTS = new Set(['FREQ', 'INTERVAL', 'COUNT', 'UNTIL', 'BYDAY', 'BYMONTHDAY', 'BYMONTH', 'WKST']);
const RRULE_FREQS = ['DAILY', 'WEEKLY', 'MONTHLY', 'YEARLY'];
const RRULE_MAX_PERIODS = 20000;

// ----------------------------------------------------------------------------
// A series' lines: its RRULE's parts by name (null without one), the line
// as written, and the local days (`dayKey`) its EXDATEs leave out.
export function parseRecurrence(lines) {
    let rule = null;
    let written = null;
    const excluded = new Set();
    for (const line of lines) {
        const colon = line.indexOf(':');
        const name = line.slice(0, colon).split(';')[0].toUpperCase();
        const value = line.slice(colon + 1);
        if (name === 'RRULE') {
            rule = Object.fromEntries(value.split(';').map(part => part.split('=')));
            written = line;
        } else if (name === 'EXDATE') {
            for (const date of value.split(',')) excluded.add(dayKey(icalDate(date)));
        }
    }
    return { rule, written, excluded };
}

// ----------------------------------------------------------------------------
// Whether `rule` is one the page expands (and puts in words). A yearly
// rule picks days by month (BYDAY or BYMONTHDAY across the whole year
// isn't supported).
export function ruleSupported(rule) {
    return !!rule && RRULE_FREQS.includes(rule.FREQ)
        && Object.keys(rule).every(part => RRULE_PARTS.has(part))
        && !(rule.FREQ === 'YEARLY' && !rule.BYMONTH && (rule.BYDAY || rule.BYMONTHDAY));
}

// ----------------------------------------------------------------------------
// The occurrence starts of the series beginning at `first`, before `before`.
function recurrenceStarts(first, lines, before) {
    const { rule, excluded } = parseRecurrence(lines);
    if (!ruleSupported(rule)) return first < before ? [first] : [];

    const interval = Math.max(1, Number(rule.INTERVAL) || 1);
    const count = rule.COUNT ? Number(rule.COUNT) : Infinity;
    const until = rule.UNTIL ? icalDate(rule.UNTIL, true) : null;
    const days = rule.BYDAY ? rule.BYDAY.split(',').map(byDay) : null;
    const monthDays = rule.BYMONTHDAY ? rule.BYMONTHDAY.split(',').map(Number) : null;
    const months = rule.BYMONTH ? rule.BYMONTH.split(',').map(m => Number(m) - 1) : null;
    const at = (y, m, d) => new Date(y, m, d, first.getHours(), first.getMinutes(), first.getSeconds());

    const starts = [];
    let generated = 0;
    for (let period = 0; period < RRULE_MAX_PERIODS; period++) {
        for (const start of periodStarts(rule.FREQ, period * interval)) {
            if (start < first || (months && !months.includes(start.getMonth()))) continue;
            if (start >= before || (until && start > until) || generated >= count) return starts;
            generated++;
            if (!excluded.has(dayKey(start))) starts.push(start);
        }
    }
    return starts;

    // The candidate starts of the `n`th period after the first's, in order.
    function periodStarts(freq, n) {
        const y = first.getFullYear(), m = first.getMonth(), d = first.getDate();
        if (freq === 'DAILY') {
            const day = at(y, m, d + n);
            return !days || days.some(b => b.weekday === day.getDay()) ? [day] : [];
        }
        if (freq === 'WEEKLY') {
            const monday = d - ((first.getDay() + 6) % 7) + 7 * n;
            const weekdays = days ? days.map(b => b.weekday) : [first.getDay()];
            return weekdays
                .map(w => at(y, m, monday + (w + 6) % 7))
                .sort((a, b) => a - b);
        }
        if (freq === 'MONTHLY') {
            const month = new Date(y, m + n, 1);
            return monthStarts(month.getFullYear(), month.getMonth());
        }
        // YEARLY
        const year = y + n;
        return (months ?? [m]).flatMap(month => monthStarts(year, month));
    }

    // The candidate starts in one month: by BYMONTHDAY (those of the BYDAY
    // weekdays, if both are given), by BYDAY, or the first's day (skipping
    // months without it, e.g. the 31st).
    function monthStarts(y, m) {
        const length = new Date(y, m + 1, 0).getDate();
        let dates;
        if (monthDays) {
            dates = monthDays
                .map(md => (md < 0 ? length + md + 1 : md))
                .filter(date => !days || days.some(b => b.weekday === new Date(y, m, date).getDay()));
        } else if (days) {
            dates = days.flatMap(b => weekdaysInMonth(y, m, length, b));
        } else {
            dates = [first.getDate()];
        }
        return dates
            .filter(date => date >= 1 && date <= length)
            .sort((a, b) => a - b)
            .map(date => at(y, m, date));
    }
}

// ----------------------------------------------------------------------------
// The days of month `m` that are the BYDAY entry `b`'s weekday: all of
// them, or only the nth (counted from the end if negative).
function weekdaysInMonth(y, m, length, b) {
    const firstWeekday = new Date(y, m, 1).getDay();
    const all = [];
    for (let date = 1 + ((b.weekday - firstWeekday + 7) % 7); date <= length; date += 7) {
        all.push(date);
    }
    if (!b.nth) return all;
    const date = b.nth > 0 ? all[b.nth - 1] : all[all.length + b.nth];
    return date ? [date] : [];
}

// ----------------------------------------------------------------------------
// A BYDAY entry: `MO`, `2TU`, `-1FR`.
export function byDay(entry) {
    const weekday = WEEKDAYS.indexOf(entry.slice(-2));
    return { weekday, nth: Number(entry.slice(0, -2)) || 0 };
}

// ----------------------------------------------------------------------------
// An iCalendar date (`20261005`) or date-time (`20261005T100000`, local;
// `...Z`, UTC) as a Date. A plain date as a bound (`endOfDay`) is the day's
// last second.
export function icalDate(text, endOfDay = false) {
    const n = (from, to) => Number(text.slice(from, to));
    const [y, m, d] = [n(0, 4), n(4, 6) - 1, n(6, 8)];
    if (text.length < 15) return endOfDay ? new Date(y, m, d, 23, 59, 59) : new Date(y, m, d);
    const [hh, mm, ss] = [n(9, 11), n(11, 13), n(13, 15)];
    return text.endsWith('Z') ? new Date(Date.UTC(y, m, d, hh, mm, ss)) : new Date(y, m, d, hh, mm, ss);
}

// ----------------------------------------------------------------------------
// A local day, for matching EXDATEs with occurrences.
function dayKey(date) {
    return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
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
