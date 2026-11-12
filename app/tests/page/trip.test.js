import { equal, test } from './harness.js';
import { vault } from './vault.js';
import { localDay, parseDate } from '../../static/core/format.js';
import { entities } from '../../static/model/entities.js';
import { isTripEvent, placeZone, tripPlaces, tripSummary, zoneName } from '../../static/model/trip.js';

// A trip from Springfield, from `from` to `to`, and its events on days:
// `to` the morning after the last.
const DAYS = { from: '2027-07-08', to: '2027-07-12' };
const trip = (facts = {}, more = {}) => ({ id: 'trip', kind: 'project', title: 'Trip', project: { blueprint: 'travel', facts: { home: 'Springfield', days: DAYS, ...facts }, ...more } });
const part = (id, type, date, end, more = {}, meta = {}) => ({
  id, kind: 'event', title: id, date, projects: ['trip'], event: { type, start: date, end, status: 'confirmed', ...more }, ...meta,
});
const journey = (id, day, from, to, more = {}) => part(id, 'journey', day, nextDay(day), { from, to, by: 'train', ...more });
const stay = (id, from, to, place, more = {}) => part(id, 'stay', from, to, { place, ...more });
const nextDay = day => localDay(new Date(parseDate(day).getFullYear(), parseDate(day).getMonth(), parseDate(day).getDate() + 1));
const PLANNED = { status: 'tentative' };

// A timed journey, its ends in their own zones.
const flight = (id, start, end, from, to, zone, endZone) => part(id, 'journey', start, end, { from, to, by: 'flight', tz: zone, 'end-tz': endZone });

test('the zone a place had last, and the places and zones named', () => {
  vault(
    flight('old', '2026-07-08T10:00:00Z', '2026-07-08T12:00:00Z', 'Springfield', 'Hamburg', 'America/Chicago', 'Europe/London'),
    flight('new', '2027-07-08T10:00:00Z', '2027-07-08T12:00:00Z', 'Springfield', 'hamburg ', 'America/Chicago', 'Europe/Berlin'),
    stay('hotel', '2027-07-08', '2027-07-12', 'Altona'),
  );
  equal([placeZone('Hamburg'), placeZone(' SPRINGFIELD'), placeZone('Altona'), placeZone('Rome'), placeZone('')], ['Europe/Berlin', 'America/Chicago', null, null, null]);
  equal(tripPlaces(), { places: ['Altona', 'Hamburg', 'Springfield', 'hamburg'], zones: ['America/Chicago', 'Europe/Berlin', 'Europe/London'] });
  equal([zoneName('America/New_York'), zoneName('UTC')], ['New York', 'UTC']);
});

test("a trip's event says what part it is, or is in a trip", () => {
  vault(trip(), { id: 'other', kind: 'project', title: 'Other', project: { blueprint: 'move' } });
  const event = (type, ...ids) => isTripEvent({ kind: 'event', event: { type }, projects: ids });
  equal([event('stay'), event(undefined, 'trip'), event(undefined, 'other'), event(undefined), isTripEvent({ kind: 'document', projects: ['trip'] })], [true, true, false, false, false]);
});

test('what a row tells of a stay', () => {
  vault(stay('hotel', '2027-07-08', '2027-07-12', 'Hamburg', PLANNED), part('tour', 'activity', '2027-07-10', '2027-07-11', { place: 'Altona' }), journey('train', '2027-07-08', 'a', 'b', { booking: 'X7' }));
  equal(['hotel', 'tour', 'train'].map(id => tripSummary(entities.get(id))), ['trip.planned \u00b7 Hamburg \u00b7 trip.nights', 'Altona', 'X7']);
});
