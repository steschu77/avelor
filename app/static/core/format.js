// Dates as the page shows them.

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
