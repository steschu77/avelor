// Schedules: the RFC 5545 lines of an event's `schedule` (`RRULE:`, `EXDATE:`,
// as Google keeps them), expanded in local time from the first start, so 10:00
// stays 10:00 across DST. Supported: FREQ DAILY/WEEKLY/MONTHLY/YEARLY with
// INTERVAL, COUNT, UNTIL, BYDAY (weekdays; with an ordinal such as 2TU or -1FR
// in a month), BYMONTHDAY, BYMONTH, and EXDATE. Any other rule shows the first
// occurrence only. For the calendar (model/calendar.js).

const WEEKDAYS = ['SU', 'MO', 'TU', 'WE', 'TH', 'FR', 'SA'];
const RRULE_PARTS = new Set(['FREQ', 'INTERVAL', 'COUNT', 'UNTIL', 'BYDAY', 'BYMONTHDAY', 'BYMONTH', 'WKST']);
const RRULE_FREQS = ['DAILY', 'WEEKLY', 'MONTHLY', 'YEARLY'];
const RRULE_MAX_PERIODS = 20000;

// ----------------------------------------------------------------------------
// A recurring event's lines: its RRULE's parts by name (null without one), the line
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
// The occurrence starts of the recurring event beginning at `first`, before `before`.
export function scheduleStarts(first, lines, before) {
    const { rule, excluded } = parseSchedule(lines);
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
export function dayKey(date) {
    return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}
