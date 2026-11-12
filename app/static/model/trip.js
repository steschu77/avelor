// A trip's events (vault.md#event, panel.md#trip-fields): what part of a
// trip each is and how, planned or booked, the places they name and the time
// zones those had last, and what a row tells of one. A trip's route
// (panel.md#route): its nights, the stops and the legs between them.

import { addDays, dayStart, daysBetween, parseDate, timeFormatIn, zonedTime } from '../core/format.js';
import { t } from '../core/text.js';
import { entities, projects, projectsOf } from './entities.js';

// ----------------------------------------------------------------------------
// The parts of a trip (`event.type`), what a journey is by (`event.by`), and
// planned or booked (`event.status`), as the vault keeps them and in words.
export const TRIP_TYPES = [['journey', t('event_type.journey')], ['stay', t('event_type.stay')], ['activity', t('event_type.activity')]];
// An event's Type on its card: a plain one (`''`, no `type`), or a part of
// a trip.
export const EVENT_TYPES = [['', t('kind.event')], ...TRIP_TYPES];
export const MEANS = [['flight', t('means.flight')], ['train', t('means.train')], ['bus', t('means.bus')], ['ferry', t('means.ferry')], ['car', t('means.car')]];
export const STATES = [
  ['tentative', t('trip.planned'), { title: t('trip.planned_title') }],
  ['confirmed', t('trip.booked'), { title: t('trip.booked_title') }],
];

// This PC's time zone.
export const LOCAL_ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone;

// ----------------------------------------------------------------------------
// Whether `event` (a block) is planned: tentative; else it counts as booked.
export const isPlanned = event => event?.status === 'tentative';

// The zone each end of `event` is meant in: the start's `tz`, the
// end's own if it has one (`end-tz`); none: this PC's.
export const startZone = event => event?.tz ?? null;
export const endZone = event => event?.['end-tz'] ?? event?.tz ?? null;

// ----------------------------------------------------------------------------
// The trips the entity `doc` is in: its projects of the `travel` blueprint.
function tripsOf(doc) {
  return (doc?.projects ?? []).filter(id => projects.get(id)?.project?.blueprint === 'travel');
}

// Whether `doc` is a trip's event: it says what part of one it is, or it is
// in a trip.
export function isTripEvent(doc) {
  return doc?.kind === 'event' && (Boolean(doc.event?.type) || tripsOf(doc).length > 0);
}

// ----------------------------------------------------------------------------
// The trip events of the vault, with the places each names: a journey its
// `from` (in its start's zone) and `to` (its end's), a stay or an activity
// its `place`.
function* placesNamed() {
  for (const doc of entities.values()) {
    const event = doc.kind === 'event' ? doc.event : null;
    if (!event?.type || isNaN(doc.date)) continue;
    if (event.from) yield { doc, place: event.from, zone: startZone(event) };
    if (event.to) yield { doc, place: event.to, zone: endZone(event) };
    if (event.place) yield { doc, place: event.place, zone: startZone(event) };
  }
}

// The zone `place` had last: that of the latest trip event naming it with
// one; null if none does.
export function placeZone(place) {
  const name = place?.trim().toLowerCase();
  if (!name) return null;
  let found = null;
  for (const { doc, place: named, zone } of placesNamed()) {
    if (!zone || named.trim().toLowerCase() !== name) continue;
    if (!found || doc.date > found.date) found = { zone, date: doc.date };
  }
  return found?.zone ?? null;
}

// The places the vault's trip events name, and the zones they are in, each
// sorted: what a place's input and the zones' menu offer first.
export function tripPlaces() {
  const places = new Set();
  const zones = new Set();
  for (const { place, zone } of placesNamed()) {
    places.add(place.trim());
    if (zone) zones.add(zone);
  }
  return { places: [...places].sort(), zones: [...zones].sort() };
}

// A zone in words, its city: "Chicago" for `America/Chicago`.
export function zoneName(zone) {
  return (zone ?? LOCAL_ZONE).split('/').pop().replace(/_/g, ' ');
}

// ----------------------------------------------------------------------------
// What a row tells of the trip event `doc` (the Timeline): planned; a
// journey's times, each in its own place's zone, the places and the
// booking; a stay's place and nights; an activity's place and time.
export function tripSummary(doc) {
  const event = doc.event;
  const parts = [isPlanned(event) ? t('trip.planned') : null];
  if (event.type === 'journey') {
    if (!doc.all_day) parts.push(waysText(doc));
    parts.push(event.booking);
  } else if (event.type === 'stay') {
    const nights = doc.end ? daysBetween(doc.date, doc.end) : 0;
    parts.push(event.place, nights > 0 ? t('trip.nights', { n: nights }) : null);
  } else {
    parts.push(event.place, doc.all_day ? null : timeFormatIn(startZone(event)).format(doc.date));
  }
  return parts.filter(Boolean).join(' \u00b7 ');
}

// "6:05 PM Springfield \u2192 9:20 AM Frankfurt, next day".
function waysText(doc) {
  const { event } = doc;
  const at = (date, zone, place) => [timeFormatIn(zone).format(date), place].filter(Boolean).join(' ');
  const leaves = at(doc.date, startZone(event), event.from);
  if (!doc.end) return leaves;
  const days = daysBetween(parseDate(zonedTime(doc.date, startZone(event)).day),
    parseDate(zonedTime(doc.end, endZone(event)).day));
  const later = days === 1 ? t('trip.next_day') : days > 1 ? t('trip.days_later', { n: days }) : null;
  const arrives = at(doc.end, endZone(event), event.to);
  return later ? `${leaves} \u2192 ${arrives}, ${later}` : `${leaves} \u2192 ${arrives}`;
}

// ----------------------------------------------------------------------------
// A sketch's nights (vault.md#project): `n`, about `~n`, a range `n-m`, as
// `{ min, max, about }`; null if it names none.
export function nightsOf(value) {
  if (typeof value === 'number') return { min: value, max: value, about: false };
  const m = /^\s*(~)?\s*(\d+)\s*(?:[-\u2013]\s*(\d+))?\s*$/.exec(String(value ?? ''));
  if (!m) return null;
  const min = Number(m[2]);
  return { min, max: Math.max(min, Number(m[3] ?? min)), about: Boolean(m[1]) };
}

// ----------------------------------------------------------------------------
// The route of the trip `id`, its block `project`: from home and back, a
// stop per run of nights at one place and a leg between two stops. While
// its dates are set (the Days fact), it comes from its events: every night
// is covered by a stay or by a leg that runs over it, and every change of
// place is bridged by a leg, its journeys chained while no night lies
// between them (a via where they meet). While they are open, it is the
// sketch's. `{ home, sketched, from, to, parts }`, `parts` in order:
// - a stop, `{ type: 'stop', place, name, state, nights, stays, from, to }`:
//   a cell per night (`{ day, state, stays }`), the stays covering them,
//   its first night's day and the morning after its last; sketched,
//   `count` (`nightsOf`), `note` and `index`, its place in the sketch;
// - a leg, `{ type: 'leg', state, parts, journeys, from, to, nights, day }`:
//   its journeys, their journeys, where it leaves from and arrives, the
//   nights it runs over and the day it leaves; one missing has its places
//   and its day only; sketched, its means and `index`, the stop it leads to
//   (the number of stops for the journey back).
// The states: `booked`, `planned` (tentative), `missing` (a night without a
// stay, a change of place without a leg), `partial` (a stop with nights
// missing), `idea` (sketched). Null for a project that is no trip.
export function routeOf(id, project) {
  if (project?.blueprint !== 'travel') return null;
  const home = project.facts?.home ? String(project.facts.home) : null;
  const from = parseDate(project.facts?.days?.from);
  const to = parseDate(project.facts?.days?.to);
  if (from && to && to > from) return { home, sketched: false, from, to, parts: datedParts(id, home, from, to) };
  return { home, sketched: true, from: null, to: null, parts: sketchedParts(home, project.sketch) };
}

const placeKey = place => place?.trim().toLowerCase() || null;
const isBooked = doc => !isPlanned(doc.event);

// The day the trip event `doc` starts on, in its place's time; the day it
// ends on (a stay's morning after its last night, a day-only event's end
// excluded); the day a journey arrives.
function startDay(doc) {
  return doc.all_day ? dayStart(doc.date) : parseDate(zonedTime(doc.date, startZone(doc.event)).day);
}

function endDay(doc) {
  if (!doc.end) return doc.all_day ? addDays(startDay(doc), 1) : startDay(doc);
  return doc.all_day ? dayStart(doc.end) : parseDate(zonedTime(doc.end, endZone(doc.event)).day);
}

function arrivalDay(doc) {
  const day = doc.all_day ? addDays(endDay(doc), -1) : endDay(doc);
  return day < startDay(doc) ? startDay(doc) : day;
}

function datedParts(id, home, from, to) {
  const count = daysBetween(from, to);
  const at = day => daysBetween(from, day);
  const events = [...entities.values()].filter(doc => doc.kind === 'event' && doc.event?.type
    && doc.event.status !== 'cancelled' && !isNaN(doc.date) && projectsOf(doc).includes(id));

  const legs = [];
  for (const doc of events.filter(doc => doc.event.type === 'journey').sort((a, b) => a.date - b.date)) {
    const day = startDay(doc);
    if (day < from || day > to) continue;
    const last = legs.at(-1);
    if (last && day <= last.arrives) {
      last.parts.push(doc);
      if (arrivalDay(doc) > last.arrives) last.arrives = arrivalDay(doc);
    } else {
      legs.push({ type: 'leg', parts: [doc], day, arrives: arrivalDay(doc) });
    }
  }

  const nights = Array.from({ length: count }, (_, i) => ({ day: addDays(from, i), leg: null, stays: [] }));
  for (const leg of legs) {
    leg.state = leg.parts.every(isBooked) ? 'booked' : 'planned';
    leg.means = leg.parts.map(doc => doc.event.by ?? null);
    leg.from = leg.parts[0].event.from ?? null;
    leg.to = leg.parts.at(-1).event.to ?? null;
    leg.nights = [];
    for (let i = Math.max(0, at(leg.day)); i < Math.min(count, at(leg.arrives)); i++) {
      nights[i].leg = leg;
      leg.nights.push({ day: nights[i].day, state: leg.state, stays: [] });
    }
  }
  for (const doc of events.filter(doc => doc.event.type === 'stay')) {
    for (let i = Math.max(0, at(startDay(doc))); i < Math.min(count, at(endDay(doc))); i++) {
      if (!nights[i].leg) nights[i].stays.push(doc);
    }
  }

  // A night without a stay is where the trip was the night before: the
  // place of its stay, or where a leg arrived since; else where the next
  // leg leaves from.
  let here = null;
  let arrived = 0;
  for (const night of nights) {
    while (arrived < legs.length && legs[arrived].arrives <= night.day) here = legs[arrived++].to ?? here;
    if (night.stays.length) here = night.stays[0].event.place ?? here;
    night.place = night.stays.length ? night.stays[0].event.place ?? null
      : here ?? legs.find(leg => leg.day > night.day)?.from ?? null;
  }
  const parts = [];
  let stop = null;
  let next = 0;
  for (let i = 0; i <= count; i++) {
    while (next < legs.length && legs[next].day <= addDays(from, i)) {
      parts.push(legs[next++]);
      stop = null;
    }
    const night = nights[i];
    if (!night || night.leg) continue;
    const place = night.place;
    if (!stop || placeKey(stop.place) !== placeKey(place)) {
      stop = { type: 'stop', place, name: place ?? night.stays[0]?.title ?? null, nights: [], stays: [], from: night.day };
      parts.push(stop);
    }
    const state = !night.stays.length ? 'missing' : night.stays.some(isBooked) ? 'booked' : 'planned';
    stop.nights.push({ day: night.day, state, stays: night.stays });
    stop.stays.push(...night.stays.filter(doc => !stop.stays.includes(doc)));
    stop.to = addDays(night.day, 1);
  }

  // A change of place without a leg, home at both ends: a leg missing. A
  // leg leaves from where the trip is and takes it to where it arrives; a
  // place not known can't differ.
  const route = [];
  let place = home ?? null;
  const bridge = (a, b, day) => ({ type: 'leg', state: 'missing', parts: [], means: [], from: a, to: b, nights: [], day });
  for (const part of parts) {
    if (part.type === 'stop') part.state = stopState(part.nights);
    const [starts, day] = part.type === 'stop' ? [part.place, part.from] : [part.from, part.day];
    if (place && starts && placeKey(place) !== placeKey(starts)) route.push(bridge(place, starts, day));
    route.push(part);
    place = part.type === 'stop' ? part.place : part.to;
  }
  if (place && home && placeKey(place) !== placeKey(home)) route.push(bridge(place, home, to));
  return route;
}

function stopState(nights) {
  const missing = nights.filter(night => night.state === 'missing').length;
  if (missing === nights.length) return 'missing';
  if (missing) return 'partial';
  return nights.some(night => night.state === 'planned') ? 'planned' : 'booked';
}

function sketchedParts(home, sketch) {
  const stops = sketch?.stops ?? [];
  const parts = [];
  stops.forEach((stop, index) => {
    const place = stop.place ? String(stop.place) : null;
    parts.push({
      type: 'leg', state: 'idea', index, parts: [], means: [].concat(stop.by ?? []),
      from: index ? stops[index - 1].place ?? null : home, to: place,
      nights: stop.overnight ? [{ day: null, state: 'idea', stays: [] }] : [],
    });
    const count = nightsOf(stop.nights);
    parts.push({
      type: 'stop', state: 'idea', index, place, name: place, count, note: stop.note ? String(stop.note) : null,
      nights: Array.from({ length: count?.max ?? 0 }, (_, i) => ({ day: null, state: 'idea', maybe: i >= count.min, stays: [] })),
      stays: [], from: null, to: null,
    });
  });
  if (stops.length) {
    parts.push({
      type: 'leg', state: 'idea', index: stops.length, parts: [], means: [].concat(sketch.back?.by ?? []),
      from: stops.at(-1).place ?? null, to: home, nights: [],
    });
  }
  return parts;
}

// Where the trip event `doc` is on its trip's route (the card's Meets): the
// leg a journey is a part of, `{ leg, index, legs, part, nights }`; the
// stop a stay covers nights of, `{ stop, index, stops, nights }`, `nights`
// the days of those it covers itself; null if the route doesn't take it.
export function routeSpot(doc) {
  for (const id of tripsOf(doc)) {
    const route = routeOf(id, projects.get(id).project);
    if (!route || route.sketched) continue;
    const legs = route.parts.filter(part => part.type === 'leg');
    const stops = route.parts.filter(part => part.type === 'stop');
    const same = other => other.doc_id === doc.doc_id;
    const leg = legs.find(l => l.parts.some(same));
    if (leg) {
      const own = leg.parts.find(same);
      const nights = leg.nights.map(night => night.day).filter(day => day >= startDay(own) && day < arrivalDay(own));
      return { leg, index: legs.indexOf(leg), legs: legs.length, part: leg.parts.indexOf(own), nights };
    }
    const stop = stops.find(s => s.stays.some(same));
    if (stop) {
      const nights = stop.nights.filter(night => night.stays.some(same)).map(night => night.day);
      return { stop, index: stops.indexOf(stop), stops: stops.length, nights };
    }
  }
  return null;
}
