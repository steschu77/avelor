import { DATE_FORMAT } from '../../core/format.js';
import { entities } from '../../model/entities.js';
import {
    byDay, icalDate, parseRecurrence, ruleSupported,
} from '../../model/calendar.js';

// ----------------------------------------------------------------------------
// A task's statuses, as the vault keeps them and in words.
export const TASK_STATUSES = [['todo', 'To do'], ['in-progress', 'In progress'], ['waiting', 'Waiting'], ['done', 'Done']];

// ----------------------------------------------------------------------------
// A series' rule in words, e.g. "Every 2 weeks on Mon, Wed, 10 times"; one
// the page doesn't expand, or that mixes BYDAY and BYMONTHDAY, as written
// (a row of the Timeline lens).
const RRULE_UNITS = { DAILY: 'day', WEEKLY: 'week', MONTHLY: 'month', YEARLY: 'year' };
const WEEKDAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTH_NAMES = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

// ----------------------------------------------------------------------------
export function repeatsText(lines) {
    const { rule, written } = parseRecurrence(lines);
    if (!ruleSupported(rule) || (rule.BYDAY && rule.BYMONTHDAY)) return written ?? lines.join(' ');
    const unit = RRULE_UNITS[rule.FREQ];
    const interval = Math.max(1, Number(rule.INTERVAL) || 1);
    let text = interval === 1 ? `Every ${unit}` : `Every ${interval} ${unit}s`;
    const list = (value, name) => value.split(',').map(name).join(', ');
    if (rule.BYMONTH) text += ` in ${list(rule.BYMONTH, m => MONTH_NAMES[Number(m) - 1] ?? m)}`;
    if (rule.BYDAY) text += ` on ${list(rule.BYDAY, byDayText)}`;
    if (rule.BYMONTHDAY) text += ` on ${list(rule.BYMONTHDAY, monthDayText)}`;
    if (rule.COUNT) text += `, ${rule.COUNT} times`;
    if (rule.UNTIL) text += `, until ${DATE_FORMAT.format(icalDate(rule.UNTIL, true))}`;
    return text;
}

// ----------------------------------------------------------------------------
// A BYDAY entry in words: "Mon", "the 2nd Tue", "the last Fri", "the 2nd
// last Fri".
function byDayText(entry) {
    const { weekday, nth } = byDay(entry);
    const name = WEEKDAY_NAMES[weekday] ?? entry;
    if (!nth) return name;
    if (nth === -1) return `the last ${name}`;
    return nth > 0 ? `the ${ordinal(nth)} ${name}` : `the ${ordinal(-nth)} last ${name}`;
}

// ----------------------------------------------------------------------------
// A BYMONTHDAY entry in words: "day 15", "the last day", "the 2nd last day".
function monthDayText(entry) {
    const n = Number(entry);
    if (n === -1) return 'the last day';
    return n < 0 ? `the ${ordinal(-n)} last day` : `day ${n}`;
}

// ----------------------------------------------------------------------------
function ordinal(n) {
    const suffix = n % 100 >= 11 && n % 100 <= 13 ? 'th' : { 1: 'st', 2: 'nd', 3: 'rd' }[n % 10] ?? 'th';
    return `${n}${suffix}`;
}

// ----------------------------------------------------------------------------
// A contact as the vault keeps it (a guest, an assignee): a contact of the
// vault (`@id`) as its id and title, "Name <email>" as the name and the
// email, else as written.
export function parseContact(ref) {
    const id = /^@(.+)$/.exec(ref)?.[1];
    if (id && entities.has(id)) return { id, name: entities.get(id).title || 'Untitled' };
    const named = /^(.+?)\s*<([^<>]+)>$/.exec(ref);
    return named ? { name: named[1], email: named[2] } : { name: ref };
}
