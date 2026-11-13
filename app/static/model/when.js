// When (panel.md#when): the days a sketched trip could take. Its window
// (the sketch's, vault.md#project) as days, a lane per thing that limits
// them: the block's `lanes` (a calendar's all-day events, one event, spans
// typed in), then each traveler's work calendar by itself. A lane's days
// can't be (hatched), are worse (tinted) or a reason to be there (a star);
// one counted `only` can't be outside its events. Then the windows of the
// sketch's length that no hatched day touches, one per free stretch, and
// what each costs each traveler in workdays.

import { addDays, dayStart, daysBetween, parseDate } from '../core/format.js';
import { t } from '../core/text.js';
import { eventsBetween } from './calendar.js';
import { entities } from './entities.js';
import { nightsOf, workCalendarsOf } from './trip.js';

// How a lane's days count (`counts`), in words.
export const COUNTS = [
  ['cant', t('when.counts.cant')],
  ['worse', t('when.counts.worse')],
  ['reason', t('when.counts.reason')],
  ['only', t('when.counts.only')],
];
const KNOWN = new Set([...COUNTS.map(([value]) => value), 'none']);
const countsOf = value => (KNOWN.has(value) ? value : 'cant');

// The When of the trip's block `project` while its days are open, null
// once they are set or without a sketch: `{ from, to, days, nights,
// lanes, fits }`, `from` and `to` the window's first and last day (null
// without one), `nights` the sketch's length (`{ min, max }`, null if it
// says none);
// - a lane, `{ key, index, name, icon, source, work, counts, spans }`:
//   `index` its place in the block's lanes (null for a work calendar not
//   listed), `source` the calendar or event it shows, `work` the traveler
//   whose work calendar it is; a span `{ from, to, counts, title, detail,
//   doc, index }`, its first and last day within the window, `detail` what
//   lies around an `only` lane's day, `index` its place in the lane's
//   typed-in spans;
// - a fit, `{ letter, from, to, nights, reasons, worse, before, after,
//   cost }`: its first and last day, the reasons and the worse spans in
//   it, the nearest hatched spans around it, and per traveler with a work
//   calendar `{ traveler, workdays, off, remote }`.
export function whenOf(project) {
  const sketch = project?.sketch;
  if (!sketch || project.facts?.days) return null;
  const nights = sketchNights(sketch);
  const from = parseDate(sketch.window?.from);
  const to = parseDate(sketch.window?.to);
  if (!from || !to || isNaN(from) || isNaN(to) || to < from) {
    return { from: null, to: null, days: 0, nights, lanes: [], fits: [] };
  }
  const days = daysBetween(from, to) + 1;
  const works = workCalendarsOf(project);
  const lanes = lanesOf(project.lanes ?? [], from, to, works);
  return { from, to, days, nights, lanes, fits: fitsOf(from, days, nights, lanes, works) };
}

// How long the trip takes: the sketch's nights, else its stops' (one
// without its nights one) and those its journeys take over night.
function sketchNights(sketch) {
  const whole = nightsOf(sketch.nights);
  if (whole) return whole;
  const stops = sketch.stops ?? [];
  if (!stops.length) return null;
  let min = 0;
  let max = 0;
  for (const stop of stops) {
    const count = nightsOf(stop.nights) ?? { min: 1, max: 1 };
    const over = stop.overnight ? 1 : 0;
    min += count.min + over;
    max += count.max + over;
  }
  return { min, max, about: false };
}

// ----------------------------------------------------------------------------
// The lanes shown: those listed, then each work calendar not listed; one
// counted `none` is left out.
function lanesOf(listed, from, to, works) {
  const lanes = listed.map((lane, index) => laneOf(lane, index, from, to, works));
  for (const work of works) {
    if (!listed.some(lane => lane.calendar === work.calendar.doc_id)) {
      lanes.push(laneOf({ calendar: work.calendar.doc_id }, null, from, to, works));
    }
  }
  return lanes.filter(lane => lane.counts !== 'none');
}

function laneOf(lane, index, from, to, works) {
  const counts = countsOf(lane.counts);
  const id = lane.calendar ?? lane.event ?? null;
  const source = id ? entities.get(id) : null;
  const work = lane.calendar ? works.find(w => w.calendar.doc_id === lane.calendar)?.traveler ?? null : null;
  const name = lane.name || (work ? t('when.work_of', { name: work }) : null) || source?.title
    || (lane.calendar ? t('when.calendar_gone') : lane.event ? t('when.event_gone') : t('untitled'));
  const end = addDays(to, 1);
  let spans;
  if (lane.calendar || lane.event) {
    const events = !source ? [] : lane.calendar ? calendarEvents(lane.calendar, from, end) : eventsBetween(from, end, [source]);
    spans = counts === 'only'
      ? outside(events.map(doc => ({ ...daysOf(doc), title: doc.title || t('untitled') })), from, to, name)
      : events.map(doc => ({ ...daysOf(doc), counts: work ? workCounts(doc, counts) : counts, title: doc.title || t('untitled'), doc }));
  } else {
    spans = (lane.spans ?? []).map((span, i) => {
      const first = parseDate(span.from);
      const last = parseDate(span.to ?? span.from) ?? first;
      const own = countsOf(span.counts ?? counts);
      return { from: first, to: last, counts: own === 'only' || own === 'none' ? 'cant' : own, title: span.note || name, index: i };
    }).filter(span => span.from && !isNaN(span.from) && !isNaN(span.to));
  }
  spans = spans.filter(span => span.to >= from && span.from <= to)
    .map(span => ({ ...span, from: span.from < from ? from : span.from, to: span.to > to ? to : span.to }));
  const icon = lane.calendar ? '#i-calendar' : lane.event ? '#i-calendar' : '#i-edit';
  return { key: index === null ? `work:${lane.calendar}` : `lane:${index}`, index, name, icon, source, work, counts, spans };
}

// The calendar `id`'s all-day events from `from` to before `end`, a recurring event
// as its occurrences there; timed ones limit no days.
function calendarEvents(id, from, end) {
  const events = [...entities.values()].filter(doc => doc.kind === 'event' && doc.date && doc.all_day
    && doc.event?.calendar === id && doc.event?.status !== 'cancelled');
  return eventsBetween(from, end, events);
}

// An event's first and last day: an all-day end is the morning after.
function daysOf(doc) {
  const from = dayStart(doc.date);
  const last = !doc.end ? from : doc.all_day ? addDays(dayStart(doc.end), -1) : dayStart(new Date(doc.end - 1));
  return { from, to: last < from ? from : last };
}

// In a work calendar a day off is a reason (it costs no leave), a remote
// day worse; its other events count as the lane does.
function workCounts(doc, counts) {
  if (doc.event?.work === 'off') return 'reason';
  if (doc.event?.work === 'remote') return 'worse';
  return counts;
}

// The days of the window outside `inside` (each `{ from, to, title }`),
// hatched, named by the lane (`name`), with what lies around them.
function outside(inside, from, to, name) {
  const spans = [];
  const sorted = [...inside].sort((a, b) => a.from - b.from);
  let day = from;
  let before = null;
  for (const span of [...sorted, null]) {
    const until = span ? addDays(span.from, -1) : to;
    if (until >= day) {
      const detail = before && span ? t('when.between', { before: before.title, after: span.title })
        : before ? t('when.after', { name: before.title }) : span ? t('when.before', { name: span.title }) : t('when.none_of_its_days');
      spans.push({ from: day, to: until, counts: 'cant', title: name, detail });
    }
    if (!span) break;
    const next = addDays(span.to, 1);
    if (next > day) day = next;
    before = span;
  }
  return spans;
}

// ----------------------------------------------------------------------------
// One window per stretch of days no hatched span touches, as long as the
// sketch allows there and at least its fewest nights: placed to hold the
// most reasons, then the fewest worse days, then the fewest workdays, then
// the earliest; lettered in order.
function fitsOf(from, days, nights, lanes, works) {
  if (!nights) return [];
  const spans = lanes.flatMap(lane => lane.spans).map(span => ({
    ...span, first: daysBetween(from, span.from), last: daysBetween(from, span.to),
  }));
  const blocked = new Array(days).fill(false);
  const worse = new Array(days).fill(false);
  for (const span of spans) {
    for (let d = span.first; d <= span.last; d++) {
      if (span.counts === 'cant') blocked[d] = true;
      if (span.counts === 'worse') worse[d] = true;
    }
  }
  const work = works.map(({ calendar, traveler }) => ({ traveler, ...workDays(calendar.doc_id, from, days) }));
  const fits = [];
  for (let start = 0; start < days; start++) {
    if (blocked[start]) continue;
    let end = start;
    while (end + 1 < days && !blocked[end + 1]) end++;
    const n = Math.min(nights.max, end - start);
    if (n >= nights.min) fits.push(bestIn(start, end, n));
    start = end;
  }
  return fits.map((fit, i) => ({ ...fit, letter: String.fromCharCode(65 + i) }));

  function bestIn(start, end, n) {
    let best = null;
    for (let s = start; s + n <= end; s++) {
      const e = s + n;
      const reasons = spans.filter(span => span.counts === 'reason' && span.first <= e && span.last >= s);
      let worseDays = 0;
      for (let d = s; d <= e; d++) worseDays += worse[d] ? 1 : 0;
      const cost = work.map(w => costOf(w, s, e));
      const workdays = cost.reduce((sum, c) => sum + c.workdays, 0);
      const score = [-reasons.length, worseDays, workdays];
      if (!best || isBefore(score, best.score)) best = { s, e, reasons, cost, score };
    }
    const { s, e, reasons, cost } = best;
    const hatched = spans.filter(span => span.counts === 'cant');
    const before = hatched.filter(span => span.last < s).sort((a, b) => b.last - a.last)[0];
    const after = hatched.filter(span => span.first > e).sort((a, b) => a.first - b.first)[0];
    return {
      from: addDays(from, s), to: addDays(from, e), nights: n,
      reasons: reasons.map(span => span.title),
      worse: spans.filter(span => span.counts === 'worse' && span.first <= e && span.last >= s).map(span => span.title),
      before: before?.title ?? null, after: after?.title ?? null, cost,
    };
  }
}

const isBefore = (a, b) => {
  const i = a.findIndex((v, k) => v !== b[k]);
  return i >= 0 && a[i] < b[i];
};

// A work calendar's days off and remote days over the window, by its day.
function workDays(id, from, days) {
  const off = new Array(days).fill(false);
  const remote = new Array(days).fill(false);
  for (const doc of calendarEvents(id, from, addDays(from, days))) {
    const { from: first, to: last } = daysOf(doc);
    for (let d = Math.max(0, daysBetween(from, first)); d <= Math.min(days - 1, daysBetween(from, last)); d++) {
      if (doc.event?.work === 'off') off[d] = true;
      else if (doc.event?.work === 'remote') remote[d] = true;
    }
  }
  return { from, off, remote };
}

// What the days `s` to `e` cost the traveler of `work`: the workdays
// (Monday to Friday) neither off already nor remote, and those that are.
function costOf({ traveler, from, off, remote }, s, e) {
  const cost = { traveler, workdays: 0, off: 0, remote: 0 };
  for (let d = s; d <= e; d++) {
    const weekday = addDays(from, d).getDay();
    if (weekday === 0 || weekday === 6) continue;
    if (off[d]) cost.off++;
    else if (remote[d]) cost.remote++;
    else cost.workdays++;
  }
  return cost;
}
