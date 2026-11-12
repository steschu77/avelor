// A trip's events (vault.md#event, panel.md#trip-fields): what part of a
// trip each is and how, planned or booked, the places they name and the time
// zones those had last, and what a row tells of one.

import { daysBetween, parseDate, timeFormatIn, zonedTime } from '../core/format.js';
import { t } from '../core/text.js';
import { entities, projects } from './entities.js';

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
