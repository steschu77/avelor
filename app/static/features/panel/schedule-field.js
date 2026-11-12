// A schedule as a field and in words (vault.md#event, vault.md#recurring):
// the period, segmented, then a week's days or the day of the month
// (`scheduleField`), a recurring payment's card's (features/panel/details.js)
// and New recurring payment's (features/recurring-payment-new.js); an
// event's, once or every day, week, month or year, and where it ends
// (`eventScheduleField`, its card's and New's, features/new.js); a
// schedule in words (`scheduleText`), the card's, the Timeline's and the
// Recurring lens's.

import { accEl } from '../../core/dom.js';
import { DATE_FORMAT, MONTH, WEEKDAY, localDay } from '../../core/format.js';
import { ordinal, t } from '../../core/text.js';
import { EVENT_PERIODS, byDay, icalDate, parseSchedule, ruleSupported, weekdayOf } from '../../model/schedule.js';
import { fieldInput } from '../../ui/field.js';
import { accSeg, accSegMany, keyedSeg } from '../../ui/segmented.js';

// ----------------------------------------------------------------------------
// The schedule `rule` (`{period, days, day}`, as `periodOf` reads it) as a
// field: `periods`' segmented control, then a week's days, any of them,
// else the day of the month. A pick, or a day typed in (`bind(input,
// write)`: `editField` on a card, its change in New), gives `onChange` the
// rule it makes; another period starts on `from`'s weekday or day. `key`
// finds the controls again once their block is drawn anew.
export function scheduleField(rule, { periods, from, onChange, bind, key }) {
  const field = accEl('span', 'schedule-field');
  const days = rule.days ?? [weekdayOf(from)];
  const day = rule.day ?? from.getDate();
  field.append(keyedSeg(accSeg(periods, rule.period, period => onChange({ ...rule, period, days, day })), periods, key));
  if (rule.period === 'week') {
    field.append(keyedSeg(accSegMany(WEEK, days, next => onChange({ ...rule, days: next })), WEEK, `${key}:day`));
  } else {
    const input = fieldInput('number', `${key}:day`, String(day), 'form-input schedule-field__day');
    input.min = '1';
    input.max = '31';
    bind(input, value => {
      const n = Number(value);
      if (n >= 1 && n <= 31) onChange({ ...rule, day: n });
    });
    field.append(accEl('span', 'meta', t('schedule.on_day')), input);
  }
  return field;
}

// An event's schedule `rule` (`eventRuleOf`, model/schedule.js) as a field:
// once, or every day, week (then its days), month or year; then where it
// ends, never, after a day typed in or after a number of times. Each pick
// or entry (`bind(input, write)`) gives `onChange` the rule it makes; a
// week starts on `from`'s weekday, an end a month after `from` or after
// ten times.
const ENDS = [['none', t('schedule.no_end')], ['until', t('schedule.until_day')], ['count', t('schedule.count')]];

export function eventScheduleField(rule, { from, onChange, bind, key }) {
  const field = accEl('span', 'schedule-field');
  const days = rule.days ?? [weekdayOf(from)];
  field.append(keyedSeg(accSeg(EVENT_PERIODS, rule.period, period => onChange({ ...rule, period, days })), EVENT_PERIODS, key));
  if (rule.period === 'once') return field;
  if (rule.period === 'week') {
    field.append(keyedSeg(accSegMany(WEEK, days, next => onChange({ ...rule, days: next })), WEEK, `${key}:day`));
  }
  const end = rule.count ? 'count' : rule.until ? 'until' : 'none';
  const month = localDay(new Date(from.getFullYear(), from.getMonth() + 1, from.getDate()));
  field.append(keyedSeg(accSeg(ENDS, end, next => onChange({
    ...rule,
    until: next === 'until' ? month : null,
    count: next === 'count' ? 10 : null,
  })), ENDS, `${key}:end`));
  if (end === 'until') {
    const until = fieldInput('date', `${key}:until`, rule.until, 'form-input props__date');
    bind(until, value => { if (value) onChange({ ...rule, until: value }); });
    field.append(until);
  } else if (end === 'count') {
    const count = fieldInput('number', `${key}:count`, String(rule.count), 'form-input schedule-field__day');
    count.min = '1';
    bind(count, value => {
      const n = Number(value);
      if (n >= 1) onChange({ ...rule, count: Math.round(n) });
    });
    field.append(count);
  }
  return field;
}

// A week's days, Monday first (5 Jan 2026 was a Monday), by RRULE's names.
const WEEK = ['MO', 'TU', 'WE', 'TH', 'FR', 'SA', 'SU']
  .map((code, i) => [code, WEEKDAY.format(new Date(2026, 0, 5 + i))]);

// ----------------------------------------------------------------------------
// A recurring payment's rule in words, e.g. "Every 2 weeks on Mon, Wed, 10 times"; one
// the page doesn't expand, or that mixes BYDAY and BYMONTHDAY, as written
// (a card, a row of the Timeline lens).
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
