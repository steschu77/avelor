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
// Every date format of the page, here and through shared instances
// (`toLocale*String` builds a formatter per call); named by what they show,
// the examples in US English.
const format = options => new Intl.DateTimeFormat(undefined, options);

export const DATE_FORMAT = format();                                    // 10/14/2026
const DAY_MONTH = format({ day: '2-digit', month: '2-digit' });         // 10/14
// A time of day, wherever the page shows one: 9:05 AM.
export const TIME_FORMAT = format({ hour: 'numeric', minute: '2-digit' });

// A date in a list: day and month, with the year if it isn't this one (a
// thread's days in the Documents lens).
export function shortDate(date) {
  return date.getFullYear() === new Date().getFullYear() ? DAY_MONTH.format(date) : DATE_FORMAT.format(date);
}

// ----------------------------------------------------------------------------
// A count with its noun, "1 file", "3 files"; `many` for a plural that
// isn't `one` + "s".
export function plural(n, one, many = `${one}s`) {
  return `${n} ${n === 1 ? one : many}`;
}
