import { accEl } from '../../core/dom.js';
import { DATE_FORMAT, MONTH, WEEKDAY } from '../../core/format.js';
import { ordinal, t } from '../../core/text.js';
import { entities } from '../../model/entities.js';
import { isOverdue } from '../../model/calendar.js';
import { byDay, icalDate, parseSchedule, ruleSupported } from '../../model/schedule.js';
import { iconEl } from '../../ui/icons.js';

// ----------------------------------------------------------------------------
// A task's tag: "Due", or "Overdue" and filled -- the only emphasis the grid
// uses.
export function taskTag(doc) {
    const overdue = isOverdue(doc);
    return iconEl(overdue ? '#i-warning-filled' : '#i-warning', 'cal__event-icon');
}

// ----------------------------------------------------------------------------
export function dueTag(text) {
    const tag = accEl('span', 'cal__event-icon', text);
    return tag;
}

// ----------------------------------------------------------------------------
// A task's statuses, as the vault keeps them and in words.
export const TASK_STATUSES = [
    ['todo', t('task.todo')], ['in-progress', t('task.in_progress')],
    ['waiting', t('task.waiting')], ['done', t('task.done')],
];

// ----------------------------------------------------------------------------
// A recurring payment's rule in words, e.g. "Every 2 weeks on Mon, Wed, 10 times"; one
// the page doesn't expand, or that mixes BYDAY and BYMONTHDAY, as written
// (a row of the Timeline lens).
const EVERY = {
    DAILY: n => t('schedule.every_day', { n }),
    WEEKLY: n => t('schedule.every_week', { n }),
    MONTHLY: n => t('schedule.every_month', { n }),
    YEARLY: n => t('schedule.every_year', { n }),
};
// Sunday first, as RRULE's days are counted (4 Jan 2026 was a Sunday).
const WEEKDAY_NAMES = [...Array(7)].map((_, i) => WEEKDAY.format(new Date(2026, 0, 4 + i)));
const MONTH_NAMES = [...Array(12)].map((_, i) => MONTH.format(new Date(2026, i, 1)));

// ----------------------------------------------------------------------------
export function scheduleText(lines) {
    const { rule, written } = parseSchedule(lines);
    if (!ruleSupported(rule) || (rule.BYDAY && rule.BYMONTHDAY)) return written ?? lines.join(' ');
    const interval = Math.max(1, Number(rule.INTERVAL) || 1);
    let text = EVERY[rule.FREQ](interval);
    const list = (value, name) => value.split(',').map(name).join(', ');
    if (rule.BYMONTH) text += t('schedule.in_months', { months: list(rule.BYMONTH, m => MONTH_NAMES[Number(m) - 1] ?? m) });
    if (rule.BYDAY) text += t('schedule.on_days', { days: list(rule.BYDAY, byDayText) });
    if (rule.BYMONTHDAY) text += t('schedule.on_days', { days: list(rule.BYMONTHDAY, monthDayText) });
    if (rule.COUNT) text += t('schedule.times', { n: Number(rule.COUNT) });
    if (rule.UNTIL) text += t('schedule.until', { date: DATE_FORMAT.format(icalDate(rule.UNTIL, true)) });
    return text;
}

// ----------------------------------------------------------------------------
// A BYDAY entry in words: "Mon", "the 2nd Tue", "the last Fri", "the 2nd
// last Fri".
function byDayText(entry) {
    const { weekday, nth } = byDay(entry);
    const name = WEEKDAY_NAMES[weekday] ?? entry;
    if (!nth) return name;
    if (nth === -1) return t('schedule.last_weekday', { day: name });
    return nth > 0 ? t('schedule.nth_weekday', { nth: ordinal(nth), day: name })
        : t('schedule.nth_last_weekday', { nth: ordinal(-nth), day: name });
}

// ----------------------------------------------------------------------------
// A BYMONTHDAY entry in words: "day 15", "the last day", "the 2nd last day".
function monthDayText(entry) {
    const n = Number(entry);
    if (n === -1) return t('schedule.last_day');
    return n < 0 ? t('schedule.nth_last_day', { nth: ordinal(-n) }) : t('schedule.month_day', { n });
}

// ----------------------------------------------------------------------------
// A contact as the vault keeps it (a guest, an assignee): a contact of the
// vault (`@id`) as its id and title, "Name <email>" as the name and the
// email, else as written.
export function parseContact(ref) {
    const id = /^@(.+)$/.exec(ref)?.[1];
    if (id && entities.has(id)) return { id, name: entities.get(id).title || t('untitled') };
    const named = /^(.+?)\s*<([^<>]+)>$/.exec(ref);
    return named ? { name: named[1], email: named[2] } : { name: ref };
}
