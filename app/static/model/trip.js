// A trip's events (vault.md#event, panel.md#trip-fields): what part of a
// trip each is and how, planned or booked, the places they name and the time
// zones those had last, and what a row tells of one. A trip's route
// (panel.md#route): its nights, the stops and the legs between them, and
// what it lacks (`tripGaps`).

import { DAY_MONTH, addDays, dayStart, daysBetween, localDay, parseDate, timeFormatIn, zonedTime } from '../core/format.js';
import { t } from '../core/text.js';
import { entities, parseContact, projects, projectsOf } from './entities.js';

// ----------------------------------------------------------------------------
// The parts of a trip (`event.type`), what a journey is by (`event.by`), and
// planned or booked (`event.status`), as the vault keeps them and in words.
export const TRIP_TYPES = [
  ['journey', t('event_type.journey')], ['stay', t('event_type.stay')], ['activity', t('event_type.activity')],
  ['layover', t('event_type.layover')],
];
// An event's Type on its card: a plain one (`''`, no `type`), or a part of
// a trip.
export const EVENT_TYPES = [['', t('kind.event')], ...TRIP_TYPES];
export const MEANS = [
  ['flight', t('means.flight')], ['train', t('means.train')], ['bus', t('means.bus')], ['ferry', t('means.ferry')], ['car', t('means.car')],
];
export const STATES = [
  ['tentative', t('trip.planned'), { title: t('trip.planned_title') }],
  ['confirmed', t('trip.booked'), { title: t('trip.booked_title') }],
];

// This PC's time zone.
export const LOCAL_ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone;

// ----------------------------------------------------------------------------
// Whether `event` (a block) is planned: tentative; else it counts as booked.
export const isPlanned = event => event?.status === 'tentative';

// Whether `event` is a part of a trip by its nights, its days a stay's: a
// stay, or a layover (a night on the way between two journeys).
export const byNights = event => event?.type === 'stay' || event?.type === 'layover';

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
// booking; a stay's or a layover's place and nights; an activity's place
// and time.
export function tripSummary(doc) {
  const event = doc.event;
  const parts = [isPlanned(event) ? t('trip.planned') : null];
  if (event.type === 'journey') {
    if (!doc.all_day) parts.push(waysText(doc));
    parts.push(event.booking);
  } else if (byNights(event)) {
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

// Nights in words: "the night of 07/08", "the nights of 07/09 \u2013 07/14"; none
// without one.
export function nightsText(days) {
  if (!days.length) return null;
  if (days.length === 1) return t('trip.night_of', { day: DAY_MONTH.format(days[0]) });
  return t('trip.nights_of', { from: DAY_MONTH.format(days[0]), to: DAY_MONTH.format(days.at(-1)) });
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
// - a leg, `{ type: 'leg', state, parts, layovers, means, from, to, nights,
//   day }`: its journeys, the layovers it runs over (a night on the way
//   between two of them), their means, where it leaves from and arrives,
//   the nights it runs over and the day it leaves; one missing has its
//   places and its day only; sketched, its means and `index`, the stop it leads to
//   (the number of stops for the journey back).
// Dated, a stop and a leg also have `issues`: where the trip doesn't go on
// as it arrived (`meetings`).
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

// The zone an end of the trip event `doc` is meant in (this PC's if it
// names none); none for a day-only one.
function zoneAt(doc, end) {
  if (doc.all_day) return null;
  return (end === 'start' ? startZone(doc.event) : endZone(doc.event)) ?? LOCAL_ZONE;
}

// The events of the trip `id` that say what part of it they are.
function tripEvents(id) {
  return [...entities.values()].filter(doc => doc.kind === 'event' && doc.event?.type
    && doc.event.status !== 'cancelled' && !isNaN(doc.date) && projectsOf(doc).includes(id));
}

function datedParts(id, home, from, to) {
  const count = daysBetween(from, to);
  const at = day => daysBetween(from, day);
  const events = tripEvents(id);

  // A layover's nights don't part the journeys around them: they stay one leg,
  // which runs over them.
  const layovers = events.filter(doc => doc.event.type === 'layover');
  const layoverOn = night => layovers.find(doc => startDay(doc) <= night && night < endDay(doc));
  const legs = [];
  for (const doc of events.filter(doc => doc.event.type === 'journey').sort((a, b) => a.date - b.date)) {
    const day = startDay(doc);
    if (day < from || day > to) continue;
    const last = legs.at(-1);
    const between = last ? Array.from({ length: Math.max(0, daysBetween(last.arrives, day)) }, (_, i) => layoverOn(addDays(last.arrives, i))) : [];
    if (last && (day <= last.arrives || between.every(Boolean))) {
      last.parts.push(doc);
      last.layovers.push(...new Set(between.filter(doc => !last.layovers.includes(doc))));
      if (arrivalDay(doc) > last.arrives) last.arrives = arrivalDay(doc);
    } else {
      legs.push({ type: 'leg', parts: [doc], layovers: [], day, arrives: arrivalDay(doc) });
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
  for (const doc of events.filter(doc => byNights(doc.event))) {
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
  meetings(route, events.filter(doc => doc.event.type === 'activity'));
  return route;
}

// Where the trip goes on at a place, at a via or at the stop between two
// legs, it leaves in the zone it arrived in, and an activity at the stop
// is in that zone too; the parts of a leg meet at the same place. What
// doesn't is noted on the leg, or the stop of the activity, as
// `issues`: `{ kind: 'zones', place, arrive, leave }` (each `{ doc, zone }`)
// or `{ kind: 'apart', arrive, leave }` (each `{ doc, place }`). A leg
// missing leaves the trip nowhere in particular.
function meetings(route, activities) {
  let arrival = null;
  const zones = (place, arrive, leave) => (arrive.zone && leave.zone && arrive.zone !== leave.zone
    ? [{ kind: 'zones', place, arrive, leave }] : []);
  for (const part of route) {
    if (part.type === 'stop') {
      const here = arrival && placeKey(arrival.place) === placeKey(part.place) ? arrival : null;
      part.issues = !here ? [] : activities
        .filter(doc => startDay(doc) >= part.from && startDay(doc) <= part.to && (!doc.event.place || placeKey(doc.event.place) === placeKey(part.place)))
        .flatMap(doc => zones(part.place, here, { doc, zone: zoneAt(doc, 'start') }));
      continue;
    }
    if (part.state === 'missing') {
      arrival = null;
      continue;
    }
    part.issues = [];
    part.parts.forEach((doc, i) => {
      const before = i ? { doc: part.parts[i - 1], zone: zoneAt(part.parts[i - 1], 'end'), place: part.parts[i - 1].event.to ?? null } : arrival;
      if (!before) return;
      const leaves = doc.event.from ?? null;
      if (i && placeKey(before.place) && placeKey(leaves) && placeKey(before.place) !== placeKey(leaves)) {
        part.issues.push({ kind: 'apart', arrive: { doc: before.doc, place: before.place }, leave: { doc, place: leaves } });
      } else {
        part.issues.push(...zones(leaves ?? before.place, before, { doc, zone: zoneAt(doc, 'start') }));
      }
    });
    const last = part.parts.at(-1);
    arrival = { doc: last, zone: zoneAt(last, 'end'), place: last.event.to ?? null };
  }
}

// An issue of the route in words: "Zones differ in Davao: arrived in
// Manila time, leaves in Berlin time".
export function issueText(issue) {
  if (issue.kind === 'apart') {
    return t('trip.issue.apart', {
      arrives: issue.arrive.doc.title || t('trip.a_part'), to: issue.arrive.place,
      leaves: issue.leave.doc.title || t('trip.the_next'), from: issue.leave.place,
    });
  }
  const { doc, zone } = issue.leave;
  const args = { place: issue.place, arrived: zoneName(issue.arrive.zone), zone: zoneName(zone) };
  if (doc.event.type === 'activity') {
    const title = doc.title || t('trip.an_activity');
    return issue.place ? t('trip.issue.zones_activity_at', { ...args, title }) : t('trip.issue.zones_activity', { ...args, title });
  }
  return issue.place ? t('trip.issue.zones_leaves_at', args) : t('trip.issue.zones_leaves', args);
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
// leg a journey is a part of, `{ leg, index, legs, part, nights }`, or a
// layover it runs over (`layover: true`, no `part`); the stop a stay
// covers nights of, `{ stop, index, stops, nights }`; `nights` the days of
// those it covers itself; null if the route doesn't take it.
export function routeSpot(doc) {
  for (const id of tripsOf(doc)) {
    const route = routeOf(id, projects.get(id).project);
    if (!route || route.sketched) continue;
    const legs = route.parts.filter(part => part.type === 'leg');
    const stops = route.parts.filter(part => part.type === 'stop');
    const same = other => other.doc_id === doc.doc_id;
    const leg = legs.find(l => l.parts.some(same) || l.layovers?.some(same));
    if (leg && !leg.parts.some(same)) {
      const nights = leg.nights.map(night => night.day).filter(day => day >= startDay(doc) && day < endDay(doc));
      return { leg, index: legs.indexOf(leg), legs: legs.length, layover: true, nights };
    }
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

// What the route says of the zones of the trip event `doc`'s ends (the
// card's zone buttons): `{ start, end }`, the issue each end is in.
export function zoneIssuesOf(doc) {
  const found = {};
  for (const id of tripsOf(doc)) {
    const route = routeOf(id, projects.get(id).project);
    for (const part of route?.parts ?? []) {
      for (const issue of part.issues ?? []) {
        if (issue.kind !== 'zones') continue;
        if (issue.leave.doc.doc_id === doc.doc_id) found.start = issue;
        if (issue.arrive.doc.doc_id === doc.doc_id) found.end = issue;
      }
    }
  }
  return found;
}

// ----------------------------------------------------------------------------
// What the trip `id` (its block `project`) lacks, by the check that finds
// it (vault.md#project, panel.md#blueprint), in the order of its route:
// - `nights`: a run of a stop's nights without a stay (`night`: `stop`,
//   `place`, `from`, the morning after `to`), an event of the trip outside
//   its days (`outside`: `doc`);
// - `journeys`: a change of place without a leg (`missing`: `leg`), a part of
//   one planned (`planned`: `leg`, `doc`, the open `task` linked to it), parts
//   that don't meet or a zone that isn't the one arrived in (`apart`,
//   `zones`: `issue`, and `leg` unless an activity's);
// - `tickets`: a booking without a document linked to it or from it
//   (`ticket`: `doc`).
// `progress`: its nights covered `[covered, all]` and its journeys booked
// `[booked, all]`. While its days are open nothing is missing yet:
// `later`, the nights (about) and the journeys the sketch will need.
export function tripGaps(id, project) {
  const route = routeOf(id, project);
  if (!route) return null;
  if (route.sketched) return { gaps: [], progress: {}, later: laterOf(project.sketch) };
  const gaps = [];
  const nights = [0, 0];
  const journeys = [0, 0];
  const events = tripEvents(id);
  for (const part of route.parts) {
    if (part.type === 'stop') {
      nights[1] += part.nights.length;
      nights[0] += part.nights.filter(night => night.state !== 'missing').length;
      part.nights.forEach(night => {
        if (night.state !== 'missing') return;
        const run = gaps.at(-1);
        if (run?.kind === 'night' && run.stop === part && run.to.getTime() === night.day.getTime()) run.to = addDays(night.day, 1);
        else gaps.push({ rule: 'nights', kind: 'night', stop: part, place: part.place, from: night.day, to: addDays(night.day, 1) });
      });
      gaps.push(...(part.issues ?? []).map(issue => ({ rule: 'journeys', kind: issue.kind, issue, leg: null })));
      continue;
    }
    nights[1] += part.nights.length;
    nights[0] += part.nights.length;
    journeys[1]++;
    if (part.state === 'booked') journeys[0]++;
    if (part.state === 'missing') {
      gaps.push({ rule: 'journeys', kind: 'missing', leg: part });
      continue;
    }
    for (const doc of part.parts.filter(doc => isPlanned(doc.event))) {
      gaps.push({ rule: 'journeys', kind: 'planned', leg: part, doc, task: openTaskFor(doc) });
    }
    gaps.push(...part.issues.map(issue => ({ rule: 'journeys', kind: issue.kind, issue, leg: part })));
  }
  for (const doc of events.filter(doc => doc.event.booking && !hasTicket(doc)).sort((a, b) => a.date - b.date)) {
    gaps.push({ rule: 'tickets', kind: 'ticket', doc });
  }
  const outside = doc => startDay(doc) < route.from || startDay(doc) > route.to
    || (byNights(doc.event) && endDay(doc) > route.to);
  for (const doc of events.filter(outside).sort((a, b) => a.date - b.date)) {
    gaps.push({ rule: 'nights', kind: 'outside', doc });
  }
  return { gaps, progress: { nights, journeys }, later: null };
}

// The nights and the journeys a sketch will need once the dates are set: its
// length, else its stops' (a range or about one by its middle); a journey to
// each stop and one home.
function laterOf(sketch) {
  const stops = sketch?.stops ?? [];
  const counts = [nightsOf(sketch?.nights)].filter(Boolean);
  if (!counts.length) counts.push(...stops.map(stop => nightsOf(stop.nights)));
  const known = counts.length && counts.every(Boolean);
  const nights = known ? Math.round(counts.reduce((sum, c) => sum + (c.min + c.max) / 2, 0)) : null;
  return { nights, journeys: stops.length ? stops.length + 1 : 0 };
}

// The open task linked to the entity `doc` (a planned journey's "Book the
// train"), if there is one.
function openTaskFor(doc) {
  for (const other of entities.values()) {
    if (other.task && other.task.status !== 'done' && other.linked_to === doc.doc_id) return other;
  }
  return null;
}

// Whether a document is linked to the booked event `doc`, or it to one:
// its ticket, a confirmation. Not a task, a conversation or another event.
function hasTicket(doc) {
  const isTicket = other => other && other.kind !== 'event' && !other.task && !other.conversation;
  if (isTicket(entities.get(doc.linked_to))) return true;
  for (const other of entities.values()) {
    if (other.linked_to === doc.doc_id && isTicket(other)) return true;
  }
  return false;
}

// ----------------------------------------------------------------------------
// What setting the dates `{ from, to }` of the trip `id` (its block `project`)
// proposes from its sketch (ui.md#set-the-dates), each a row `{ key, title,
// event, why, from, to }` (the event's block, planned; `to` the morning after):
// - `journeys`: a journey per means of each leg, a day only on its day, from where
//   the trip is to the stop (home at both ends); over night it takes the
//   night;
// - `stays`: a stay per stop for its nights, in order: a stop takes the
//   nights sketched (a range its fewest, none sketched one), the last what
//   is left; one left without a night is none;
// - `off`: a day off range over the trip's days in the work calendar of
//   each traveler who has one (`work-of`).
// `calendar`: the trip's (its events'). `nights`: how many the days have,
// `sketched` what the sketch said (`nightsOf`).
export function setDatesProposal(id, project, days, calendar) {
  const from = parseDate(days.from);
  const to = parseDate(days.to);
  const sketch = project.sketch ?? {};
  const stops = sketch.stops ?? [];
  const home = project.facts?.home ? String(project.facts.home) : null;
  const planned = block => ({ ...(calendar && { calendar }), status: 'tentative', ...block });
  const journeys = [];
  const stays = [];
  const journeyRows = (by, a, b, day, overnight) => {
    const list = [].concat(by ?? []).map(String);
    const parts = list.length ? list : [null];
    const end = addDays(day, overnight ? 2 : 1);
    parts.forEach((means, i) => {
      const first = i === 0;
      const last = i === parts.length - 1;
      const word = MEANS.find(([value]) => value === means)?.[1] ?? t('trip.journey');
      const title = last ? t('trip.journey_to', { journey: word, place: b ?? t('trip.somewhere') })
        : first ? t('trip.journey_from', { journey: word, place: a ?? t('trip.somewhere') }) : word;
      const event = planned({ start: localDay(day), end: localDay(end), type: 'journey', ...(means && { by: means }) });
      if (first && a) event.from = a;
      if (last && b) event.to = b;
      const route = `${a ?? t('trip.somewhere_first')} \u2192 ${b ?? t('trip.somewhere')}`;
      const why = [route, parts.length > 1 ? t('trip.part_of', { at: i + 1, n: parts.length }) : null,
        overnight ? t('trip.over_night') : null].filter(Boolean).join(' \u00b7 ');
      journeys.push({ key: `journey:${journeys.length}`, title, event, why, from: day, to: end });
    });
  };
  let day = from;
  let left = daysBetween(from, to);
  stops.forEach((stop, i) => {
    const place = stop.place ? String(stop.place) : null;
    const before = i ? (stops[i - 1].place ? String(stops[i - 1].place) : null) : home;
    const overnight = Boolean(stop.overnight) && left > 0;
    journeyRows(stop.by, before, place, day, overnight);
    if (overnight) {
      day = addDays(day, 1);
      left--;
    }
    const count = nightsOf(stop.nights);
    const last = i === stops.length - 1;
    const n = last ? left : Math.min(left, count ? count.min : 1);
    if (n <= 0) return;
    const sketched = !count ? t('trip.sketch_none') : count.min !== count.max ? `${count.min}\u2013${count.max}`
      : count.about ? t('trip.about_n', { n: count.min }) : String(count.min);
    const fits = count && n >= count.min && n <= count.max;
    const as = fits ? t('trip.as_sketched') : last ? t('trip.left_sketch_said', { sketched }) : t('trip.sketch_said', { sketched });
    const note = stop.note ? String(stop.note) : null;
    stays.push({
      key: `stay:${i}`, title: note ? note[0].toUpperCase() + note.slice(1) : place ? t('trip.stay_in', { place }) : t('trip.stay_somewhere'),
      event: planned({ start: localDay(day), end: localDay(addDays(day, n)), type: 'stay', ...(place && { place }) }),
      why: [place, t('trip.nights', { n }), as].filter(Boolean).join(' \u00b7 '),
      from: day, to: addDays(day, n),
    });
    day = addDays(day, n);
    left -= n;
  });
  if (stops.length) journeyRows(sketch.back?.by, stops.at(-1).place ? String(stops.at(-1).place) : home, home, to, false);

  const off = workCalendarsOf(project).map(({ calendar: calendarDoc, traveler }) => ({
    key: `off:${calendarDoc.doc_id}`, title: t('trip.off', { name: projects.get(id)?.name ?? t('trip.the_trip') }),
    event: { calendar: calendarDoc.doc_id, status: 'tentative', start: localDay(from), end: localDay(addDays(to, 1)), work: 'off' },
    why: t('trip.work_calendar_of', { traveler, title: calendarDoc.title || t('untitled') }),
    from, to: addDays(to, 1),
  }));
  return { nights: daysBetween(from, to), sketched: nightsOf(sketch.nights), journeys, stays, off };
}

// The work calendars of the trip's travelers (`work-of`,
// vault.md#calendar), `{ calendar, traveler }`, the traveler by name.
export function workCalendarsOf(project) {
  const travelers = [].concat(project.facts?.travelers ?? []).map(String);
  const found = [];
  for (const calendar of entities.values()) {
    const owner = calendar.kind === 'calendar' ? calendar.calendar?.['work-of'] : null;
    const traveler = owner && travelers.find(ref => sameContact(ref, owner));
    if (traveler) found.push({ calendar, traveler: parseContact(traveler).name });
  }
  return found;
}

// Whether the contact refs `a` and `b` name the same person: the same
// contact, or the same name.
function sameContact(a, b) {
  const [x, y] = [parseContact(a), parseContact(b)];
  return x.id && y.id ? x.id === y.id : x.name.trim().toLowerCase() === y.name.trim().toLowerCase();
}
