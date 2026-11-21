// Dates and counts as the page shows them.

import { locale, t } from './text.js';

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

// How many days `b`'s local day is after `a`'s, negative before it.
export const daysBetween = (a, b) => Math.round((dayStart(b) - dayStart(a)) / 864e5);

// Local-time date helpers
export const sameDay = (a, b) =>
  a.getFullYear() === b.getFullYear() &&
  a.getMonth() === b.getMonth() &&
  a.getDate() === b.getDate();
const weekFirstDay = 1; // Monday as the first day of the week
export const weekStartOf = d => addDays(d, -((d.getDay() - weekFirstDay + 7) % 7));
export const monthStartOf = d => new Date(d.getFullYear(), d.getMonth(), 1);
export const monthEndOf = d => new Date(d.getFullYear(), d.getMonth() + 1, 0);
export const addDays = (d, n) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);

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
const pad = n => String(n).padStart(2, '0');

// `date`'s local day as `YYYY-MM-DD`, the value of an <input type="date">.
export function localDay(date) {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

// `date`'s local time of day as `hh:mm`, the value of an <input type="time">.
function localTime(date) {
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

// ----------------------------------------------------------------------------
// Wall times in a time zone (an IANA name, `America/Chicago`; none: this
// PC's), as a trip's events keep theirs (vault.md#event): an instant's day
// and time there, and the instant a day and time there name.
const zoneFormats = new Map(); // zone -> its formatter

function zoneParts(date, zone) {
  let f = zoneFormats.get(zone);
  if (!f) {
    f = new Intl.DateTimeFormat('en-US', {
      timeZone: zone, hourCycle: 'h23', year: 'numeric', month: '2-digit', day: '2-digit',
      hour: '2-digit', minute: '2-digit', second: '2-digit',
    });
    zoneFormats.set(zone, f);
  }
  return Object.fromEntries(f.formatToParts(date).map(part => [part.type, part.value]));
}

// `date`'s day and time of day in `zone`: `{ day: 'YYYY-MM-DD', time: 'hh:mm' }`.
export function zonedTime(date, zone) {
  if (!zone) return { day: localDay(date), time: localTime(date) };
  const p = zoneParts(date, zone);
  return { day: `${p.year}-${p.month}-${p.day}`, time: `${p.hour}:${p.minute}` };
}

// How far `zone`'s wall time is ahead of UTC at `ms`, in ms.
function zoneOffset(ms, zone) {
  const p = zoneParts(new Date(ms), zone);
  const wall = Date.UTC(Number(p.year), Number(p.month) - 1, Number(p.day), Number(p.hour), Number(p.minute), Number(p.second));
  return wall - Math.floor(ms / 1000) * 1000;
}

// The instant `day` (YYYY-MM-DD) at `time` (hh:mm, midnight without one)
// names in `zone`. The offset is taken twice, so a wall time just after a
// change of it lands right.
export function zonedInstant(day, time, zone) {
  const [y, m, d] = day.split('-').map(Number);
  const [h, mi] = (time || '00:00').split(':').map(Number);
  if (!zone) return new Date(y, m - 1, d, h, mi);
  const wall = Date.UTC(y, m - 1, d, h, mi);
  const first = wall - zoneOffset(wall, zone);
  return new Date(wall - zoneOffset(first, zone));
}

// How long `ms` is: "8 h 15 min", "45 min", "2 h".
export function durationText(ms) {
  const minutes = Math.round(Math.abs(ms) / 6e4);
  const h = Math.floor(minutes / 60);
  const min = minutes % 60;
  return [h ? t('time.hours', { n: h }) : '', min || !h ? t('time.minutes', { n: min }) : '']
    .filter(Boolean).join(' ');
}

// `date` as the host writes an instant: UTC in ISO 8601, to the second.
export function isoInstant(date) {
  return date.toISOString().replace(/\.\d{3}Z$/, 'Z');
}

// ----------------------------------------------------------------------------
// `date`'s day as seen from today: "today", "tomorrow", "in 3 days",
// "yesterday", "3 days ago"; empty without a date.
const RELATIVE_DAY = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
export function relativeDay(date) {
  if (!date || isNaN(date)) return '';
  return RELATIVE_DAY.format(daysBetween(new Date(), date), 'day');
}

// ----------------------------------------------------------------------------
// Every date format of the page, here and through shared instances
// (`toLocale*String` builds a formatter per call); named by what they show,
// the examples in US English.
const format = options => new Intl.DateTimeFormat(locale, options);

export const DATE_FORMAT = format();                                    // 10/14/2026
export const DAY_MONTH = format({ day: '2-digit', month: '2-digit' }); // 10/14
// A time of day, wherever the page shows one: 9:05 AM.
export const TIME_FORMAT = format({ hour: 'numeric', minute: '2-digit' });

// TIME_FORMAT's, in `zone` (none: this PC's): a trip's times, each in its
// own place's time.
const zoneTimes = new Map(); // zone -> its formatter

export function timeFormatIn(zone) {
  if (!zone) return TIME_FORMAT;
  let f = zoneTimes.get(zone);
  if (!f) {
    f = format({ hour: 'numeric', minute: '2-digit', timeZone: zone });
    zoneTimes.set(zone, f);
  }
  return f;
}
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

// A date in a column of emails, "Oct 05": the year only when it isn't this
// one, and with the time if `withTime`.
export function shortMonthDate(date, withTime) {
  const thisYear = date.getFullYear() === new Date().getFullYear();
  const day = (thisYear ? DAY2_SHORT_MONTH : DAY2_SHORT_MONTH_YEAR).format(date);
  return withTime ? `${day}, ${TIME_FORMAT.format(date)}` : day;
}

// ----------------------------------------------------------------------------
// A date of the Workspace and import pages (`iso`: an account connected or
// synced, a watch added, a scan taken), like "Oct 3, 2026", with the time
// if `withTime`; "Never" without one.
export function pageDate(iso, withTime) {
  if (!iso) return t('date.never');
  return (withTime ? DAY_MONTH_YEAR_TIME : DAY_MONTH_YEAR).format(new Date(iso));
}

// ----------------------------------------------------------------------------
// Two texts in the page's order, through a shared collator (`localeCompare`
// builds one per call).
export const compareText = new Intl.Collator(locale).compare;

// ----------------------------------------------------------------------------
// A sum of money in `currency`, an ISO code ("$1,234.56"); with two decimals
// if it names none.
export function money(amount, currency) {
  try {
    return new Intl.NumberFormat(locale, { style: 'currency', currency }).format(amount);
  } catch {
    return DECIMAL.format(amount);
  }
}

// An amount in a column of them, two decimals without the currency
// ("1,234.56"; '' for none); `signedAmount` with its sign and a true minus
// ("+1,234.56", "\u22129.99"), `signedMoney` in `currency` ("\u2212$9.99").
// The Bank lens's and a transaction's card.
const DECIMAL = new Intl.NumberFormat(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const sign = n => (n < 0 ? '\u2212' : '+');
export const amountText = n => (n == null ? '' : DECIMAL.format(n));
export const signedAmount = n => `${sign(n)}${DECIMAL.format(Math.abs(n))}`;
export const signedMoney = (n, currency) => `${sign(n)}${money(Math.abs(n), currency)}`;

// An amount typed in as `amountText` writes it ("1,150.00", "1.150,00" in
// German; "-60", with a typographic minus too) as a number; null for
// anything else.
const [GROUP, POINT] = ['group', 'decimal'].map(type => DECIMAL.formatToParts(12345.6).find(p => p.type === type)?.value);
export function parseAmount(text) {
  const grouped = text.replace(/\s/g, '').replace(/\u2212/g, '-');
  const plain = (GROUP ? grouped.split(GROUP).join('') : grouped).replace(POINT ?? '.', '.');
  const n = Number(plain);
  return plain && Number.isFinite(n) ? n : null;
}

// ----------------------------------------------------------------------------
// A file's size: "812 KB", "2.4 MB".
export function fileSize(bytes) {
  if (bytes < 1024) return t('size.bytes', { n: bytes });
  if (bytes < 1024 * 1024) return t('size.kb', { n: Math.round(bytes / 1024) });
  return t('size.mb', { n: Math.round(bytes / 1024 / 1024 * 10) / 10 });
}
