// Dates and counts as the page shows them.

// ----------------------------------------------------------------------------
// A host date: `YYYY-MM-DD` as that local day's start (not UTC midnight,
// which is another day west of UTC), a date-time as the instant it names.
export function parseDate(text) {
  if (!text) return undefined;
  if (text.length !== 10) return new Date(text);
  const [y, m, d] = text.split('-').map(Number);
  return new Date(y, m - 1, d);
}

// ----------------------------------------------------------------------------
// The start of `d`'s local day.
export const dayStart = d => new Date(d.getFullYear(), d.getMonth(), d.getDate());

// ----------------------------------------------------------------------------
// Run `fn` just after every local midnight, from now on: what shows today
// moves on (the calendar's grid, the dashboard).
export function everyMidnight(fn) {
  const now = new Date();
  const tomorrow = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
  setTimeout(() => {
    fn();
    everyMidnight(fn);
  }, tomorrow - now + 1000);
}

// ----------------------------------------------------------------------------
// `date`'s local day as `YYYY-MM-DD`, the value of an <input type="date">.
export function localDay(date) {
  const pad = n => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

// `date` as the host writes an instant: UTC in ISO 8601, to the second.
export function isoInstant(date) {
  return date.toISOString().replace(/\.\d{3}Z$/, 'Z');
}

// ----------------------------------------------------------------------------
// `date`'s day as seen from today: "today", "tomorrow", "in 3 days",
// "yesterday", "3 days ago"; empty without a date.
export function relativeDay(date) {
  if (!date || isNaN(date)) return '';
  const n = Math.round((dayStart(date) - dayStart(new Date())) / 864e5);
  if (n === 0) return 'today';
  if (n === 1) return 'tomorrow';
  if (n === -1) return 'yesterday';
  return n > 0 ? `in ${n} days` : `${-n} days ago`;
}

// ----------------------------------------------------------------------------
// Every date format of the page, here and through shared instances
// (`toLocale*String` builds a formatter per call); named by what they show,
// the examples in US English.
const format = options => new Intl.DateTimeFormat(undefined, options);

export const DATE_FORMAT = format();                                    // 10/14/2026
export const DAY_MONTH = format({ day: '2-digit', month: '2-digit' }); // 10/14
// A time of day, wherever the page shows one: 9:05 AM.
export const TIME_FORMAT = format({ hour: 'numeric', minute: '2-digit' });
export const WEEKDAY = format({ weekday: 'short' });                    // Tue
export const WEEKDAY_LONG = format({ weekday: 'long' });                // Tuesday
export const MONTH = format({ month: 'short' });                        // Oct
export const MONTH_YEAR = format({ month: 'long', year: 'numeric' });   // October 2026
export const SHORT_MONTH_YEAR = format({ month: 'short', year: 'numeric' }); // Oct 2026
export const DAY_SHORT_MONTH = format({ day: 'numeric', month: 'short' });   // Oct 14
export const DAY_LONG_MONTH = format({ day: 'numeric', month: 'long' });     // October 14
export const DAY_MONTH_YEAR = format({ day: 'numeric', month: 'short', year: 'numeric' }); // Oct 14, 2026
export const DAY_LONG_MONTH_YEAR = format({ day: 'numeric', month: 'long', year: 'numeric' }); // October 14, 2026
export const LONG_DAY = format({ weekday: 'long', day: 'numeric', month: 'long' }); // Tuesday, October 14
export const LONG_DATE = format({ weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' }); // Tuesday, October 14, 2026

const DAY_MONTH_YEAR_TIME = format({
  day: 'numeric', month: 'short', year: 'numeric', hour: 'numeric', minute: '2-digit',
});
const DAY2_SHORT_MONTH = format({ day: '2-digit', month: 'short' });
const DAY2_SHORT_MONTH_YEAR = format({ day: '2-digit', month: 'short', year: 'numeric' });

// A date in a list: day and month, with the year if it isn't this one (the
// dashboard's cards, a thread's days in the Documents lens).
export function shortDate(date) {
  return date.getFullYear() === new Date().getFullYear() ? DAY_MONTH.format(date) : DATE_FORMAT.format(date);
}

// A date in a column of mails, "Oct 05": the year only when it isn't this
// one, and with the time if `withTime`.
export function shortMonthDate(date, withTime) {
  const thisYear = date.getFullYear() === new Date().getFullYear();
  const day = (thisYear ? DAY2_SHORT_MONTH : DAY2_SHORT_MONTH_YEAR).format(date);
  return withTime ? `${day}, ${TIME_FORMAT.format(date)}` : day;
}

// ----------------------------------------------------------------------------
// A date of the Workspace pages (`iso`: an account connected or synced),
// like "Oct 3, 2026", with the time if `withTime`; "Never" without one.
export function pageDate(iso, withTime) {
  if (!iso) return 'Never';
  return (withTime ? DAY_MONTH_YEAR_TIME : DAY_MONTH_YEAR).format(new Date(iso));
}

// ----------------------------------------------------------------------------
// A count with its noun, "1 file", "3 files"; `many` for a plural that
// isn't `one` + "s".
export function plural(n, one, many = `${one}s`) {
  return `${n} ${n === 1 ? one : many}`;
}
