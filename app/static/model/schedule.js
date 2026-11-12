// Schedules (vault.md#event, vault.md#recurring): the RFC 5545 lines of an
// event's or a recurring payment's `schedule` (`RRULE:`, `EXDATE:`, as Google
// keeps them), expanded on the wall clock from the first start -- in the
// event's zone, else this PC's -- so 10:00 stays 10:00 across DST there. Supported: FREQ DAILY/WEEKLY/MONTHLY/YEARLY with
// INTERVAL, COUNT, UNTIL, BYDAY (weekdays; with an ordinal such as 2TU or
// -1FR in a month), BYMONTHDAY, BYMONTH, and EXDATE. Any other rule shows the
// first occurrence only. For the calendar (model/calendar.js) and the
// recurring payments (model/bank.js).

import { isoInstant, localDay, zonedInstant, zonedTime } from '../core/format.js';
import { t } from '../core/text.js';

const WEEKDAYS = ['SU', 'MO', 'TU', 'WE', 'TH', 'FR', 'SA'];
const RRULE_PARTS = new Set(['FREQ', 'INTERVAL', 'COUNT', 'UNTIL', 'BYDAY', 'BYMONTHDAY', 'BYMONTH', 'WKST']);
const RRULE_FREQS = ['DAILY', 'WEEKLY', 'MONTHLY', 'YEARLY'];
const RRULE_MAX_PERIODS = 20000;

// ----------------------------------------------------------------------------
// A schedule's lines: its RRULE's parts by name (null without one), the line
// as written, and the local days (`dayKey`) its EXDATEs leave out.
export function parseSchedule(lines) {
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
// The occurrence starts of the schedule `lines` from `first`, before
// `before`. They are counted on the wall clock of `zone` (an IANA zone, a
// timed event's), else of this PC: each candidate a Date whose UTC fields
// hold its wall date and time, made an instant only once it may be shown,
// from `after` on (the days before it still count toward a COUNT).
export function scheduleStarts(first, lines, before, { zone = null, after = null } = {}) {
    const { rule, excluded } = parseSchedule(lines);
    if (!ruleSupported(rule)) return first < before ? [first] : [];

    const interval = Math.max(1, Number(rule.INTERVAL) || 1);
    const count = rule.COUNT ? Number(rule.COUNT) : Infinity;
    const until = rule.UNTIL ? icalDate(rule.UNTIL, true) : null;
    const days = rule.BYDAY ? rule.BYDAY.split(',').map(byDay) : null;
    const monthDays = rule.BYMONTHDAY ? rule.BYMONTHDAY.split(',').map(Number) : null;
    const months = rule.BYMONTH ? rule.BYMONTH.split(',').map(m => Number(m) - 1) : null;
    const wall = wallOf(first, zone);
    const at = (y, m, d) => new Date(Date.UTC(y, m, d, wall.getUTCHours(), wall.getUTCMinutes(), wall.getUTCSeconds()));
    // a day's margin: no zone is a day away from this PC's wall clock
    const shown = after ? wallOf(after, zone).getTime() - 864e5 : -Infinity;

    const starts = [];
    let generated = 0;
    for (let period = 0; period < RRULE_MAX_PERIODS; period++) {
        for (const candidate of periodStarts(rule.FREQ, period * interval)) {
            if (candidate < wall || (months && !months.includes(candidate.getUTCMonth()))) continue;
            if (generated >= count) return starts;
            if (candidate.getTime() < shown) {
                generated++;
                continue;
            }
            const start = instantOf(candidate, zone);
            if (start >= before || (until && start > until)) return starts;
            generated++;
            if (!excluded.has(dayKey(start))) starts.push(start);
        }
    }
    return starts;

    // The candidate starts of the `n`th period after the first's, in order.
    function periodStarts(freq, n) {
        const y = wall.getUTCFullYear(), m = wall.getUTCMonth(), d = wall.getUTCDate();
        if (freq === 'DAILY') {
            const day = at(y, m, d + n);
            return !days || days.some(b => b.weekday === day.getUTCDay()) ? [day] : [];
        }
        if (freq === 'WEEKLY') {
            const monday = d - ((wall.getUTCDay() + 6) % 7) + 7 * n;
            const weekdays = days ? days.map(b => b.weekday) : [wall.getUTCDay()];
            return weekdays
                .map(w => at(y, m, monday + (w + 6) % 7))
                .sort((a, b) => a - b);
        }
        if (freq === 'MONTHLY') {
            const month = new Date(Date.UTC(y, m + n, 1));
            return monthStarts(month.getUTCFullYear(), month.getUTCMonth());
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
            dates = [wall.getUTCDate()];
        }
        return dates
            .filter(date => date >= 1 && date <= length)
            .sort((a, b) => a - b)
            .map(date => at(y, m, date));
    }
}

// ----------------------------------------------------------------------------
// The wall date and time of `date` in `zone`, else this PC's, in a Date's
// UTC fields (`scheduleStarts`); `instantOf` turns one back.
function wallOf(date, zone) {
    if (!zone) {
        return new Date(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate(),
            date.getHours(), date.getMinutes(), date.getSeconds()));
    }
    const { day, time } = zonedTime(date, zone);
    return new Date(`${day}T${time}:${String(date.getSeconds()).padStart(2, '0')}Z`);
}

function instantOf(wall, zone) {
    if (!zone) {
        return new Date(wall.getUTCFullYear(), wall.getUTCMonth(), wall.getUTCDate(),
            wall.getUTCHours(), wall.getUTCMinutes(), wall.getUTCSeconds());
    }
    const [day, time] = wall.toISOString().slice(0, 16).split('T');
    return zonedInstant(day, time, zone);
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
export function dayKey(date) {
    return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

// ----------------------------------------------------------------------------
// A recurring payment's schedule (model/bank.js): a period picked
// (`PERIODS`) and a week's days or its day as one RRULE line
// (`scheduleLine`, `periodOf`), and how many times a year a schedule repeats
// (`timesAYear`).
export const PERIODS = [
  ['week', t('period.week')], ['month', t('period.month')], ['quarter', t('period.quarter')], ['half', t('period.half')],
  ['year', t('period.year')],
];

// The RRULE line of the schedule `{period, days, day}` (a period of
// PERIODS): a week's on its `days` (RRULE's MO..SU, else `from`'s weekday),
// the others on the day of the month `day` (else `from`'s), a year's in
// `from`'s month.
export function scheduleLine({ period, days = null, day = null }, from) {
    day ??= from.getDate();
    switch (period) {
        case 'week': return `RRULE:FREQ=WEEKLY;BYDAY=${weekOrder(days ?? [weekdayOf(from)]).join(',')}`;
        case 'quarter': return `RRULE:FREQ=MONTHLY;INTERVAL=3;BYMONTHDAY=${day}`;
        case 'half': return `RRULE:FREQ=MONTHLY;INTERVAL=6;BYMONTHDAY=${day}`;
        case 'year': return `RRULE:FREQ=YEARLY;BYMONTH=${from.getMonth() + 1};BYMONTHDAY=${day}`;
        default: return `RRULE:FREQ=MONTHLY;BYMONTHDAY=${day}`;
    }
}

// The schedule `{period, days, day}` of the lines `lines`, as `scheduleLine`
// writes them; null for any other schedule.
export function periodOf(lines) {
    const { rule } = parseSchedule(lines ?? []);
    if (!rule || lines.length !== 1) return null;
    const parts = Object.keys(rule).sort().join();
    const day = Number(rule.BYMONTHDAY);
    if (rule.FREQ === 'WEEKLY') {
        const days = rule.BYDAY?.split(',') ?? [];
        const plain = days.length && days.every(code => WEEKDAYS.includes(code));
        return parts === 'BYDAY,FREQ' && plain ? { period: 'week', days, day: null } : null;
    }
    if (!(day >= 1 && day <= 31) || String(day) !== rule.BYMONTHDAY) return null;
    if (rule.FREQ === 'YEARLY') return parts === 'BYMONTH,BYMONTHDAY,FREQ' ? { period: 'year', days: null, day } : null;
    if (rule.FREQ !== 'MONTHLY') return null;
    const period = { 1: 'month', 3: 'quarter', 6: 'half' }[Number(rule.INTERVAL ?? 1)];
    const written = rule.INTERVAL ? 'BYMONTHDAY,FREQ,INTERVAL' : 'BYMONTHDAY,FREQ';
    return period && parts === written ? { period, days: null, day } : null;
}

// The RRULE name of `date`'s weekday (`MO`).
export function weekdayOf(date) {
    return WEEKDAYS[date.getDay()];
}

// RRULE's weekday names `days` once each, Monday first.
function weekOrder(days) {
    const monday = code => (WEEKDAYS.indexOf(code) + 6) % 7;
    return [...new Set(days)].sort((a, b) => monday(a) - monday(b));
}

// How many times a year the schedule `lines` repeats; null for a rule the
// page doesn't expand.
export function timesAYear(lines) {
    const { rule } = parseSchedule(lines);
    if (!ruleSupported(rule)) return null;
    const interval = Math.max(1, Number(rule.INTERVAL) || 1);
    const count = value => (value ? value.split(',').length : 1);
    const each = {
        DAILY: 1,
        WEEKLY: count(rule.BYDAY),
        MONTHLY: count(rule.BYMONTHDAY || rule.BYDAY),
        YEARLY: count(rule.BYMONTH) * count(rule.BYMONTHDAY || rule.BYDAY),
    }[rule.FREQ];
    return ({ DAILY: 365, WEEKLY: 52, MONTHLY: 12, YEARLY: 1 }[rule.FREQ] / interval) * each;
}

// ----------------------------------------------------------------------------
// An event's schedule (vault.md#event), as its card and New offer it: once,
// or every day, week (on its days), month or year on the start's date,
// ending never, after the day `until` or after `count` times.
export const EVENT_PERIODS = [
  ['once', t('period.once')], ['day', t('period.day')], ['week', t('period.week')], ['month', t('period.month')],
  ['year', t('period.year')],
];
const EVENT_FREQS = { day: 'DAILY', week: 'WEEKLY', month: 'MONTHLY', year: 'YEARLY' };

// The schedule `{period, days, until, count, rest}` of an event's lines
// `lines` starting at `start`, its days counted in `zone` (none: this PC's),
// as `eventScheduleLines` writes them, `rest` the EXDATE lines kept; null
// for one it doesn't (an interval, a month's weekday, an RDATE).
export function eventRuleOf(lines, start, zone = null) {
    const rest = (lines ?? []).filter(line => !line.toUpperCase().startsWith('RRULE:'));
    const rules = (lines ?? []).filter(line => line.toUpperCase().startsWith('RRULE:'));
    if (!rules.length && !rest.length) return { period: 'once', days: null, until: null, count: null, rest };
    if (rules.length !== 1 || rest.some(line => !line.toUpperCase().startsWith('EXDATE'))) return null;
    const { rule } = parseSchedule(rules);
    const period = Object.keys(EVENT_FREQS).find(p => EVENT_FREQS[p] === rule.FREQ);
    if (!period) return null;
    const parts = ['FREQ', 'COUNT', 'UNTIL', 'WKST', ...(period === 'week' ? ['BYDAY'] : []), ...(period === 'month' ? ['BYMONTHDAY'] : [])];
    if (!Object.keys(rule).every(part => parts.includes(part)) || (rule.COUNT && rule.UNTIL)) return null;
    const wall = zonedTime(start, zone);
    if (rule.BYMONTHDAY && Number(rule.BYMONTHDAY) !== Number(wall.day.slice(8))) return null;
    const days = rule.BYDAY ? rule.BYDAY.split(',') : null;
    if (days && !days.every(code => WEEKDAYS.includes(code))) return null;
    const until = rule.UNTIL ? zonedTime(icalDate(rule.UNTIL, true), rule.UNTIL.length < 15 ? null : zone).day : null;
    return { period, days, until, count: rule.COUNT ? Number(rule.COUNT) : null, rest };
}

// The lines of the event schedule `rule` for an event starting at `start`:
// none once; else an RRULE, a week's on its days (else the start's
// weekday), up to the day `until` (the end of it in `zone` and in UTC for a
// timed event, the day for an all-day one, `allDay`) or `count` times; then
// the EXDATE lines it kept.
export function eventScheduleLines({ period, days = null, until = null, count = null, rest = [] }, start, zone, allDay) {
    if (period === 'once') return [];
    let line = `RRULE:FREQ=${EVENT_FREQS[period]}`;
    if (period === 'week') line += `;BYDAY=${weekOrder(days ?? [weekdayIn(start, zone)]).join(',')}`;
    if (count) line += `;COUNT=${count}`;
    else if (until && allDay) line += `;UNTIL=${until.replace(/-/g, '')}`;
    else if (until) line += `;UNTIL=${isoInstant(zonedInstant(until, '23:59', zone)).replace(/[-:]/g, '')}`;
    return [line, ...rest];
}

// The RRULE name of `date`'s weekday in `zone`, else this PC's.
function weekdayIn(date, zone) {
    const day = zone ? zonedTime(date, zone).day : localDay(date);
    return WEEKDAYS[new Date(`${day}T12:00:00Z`).getUTCDay()];
}
