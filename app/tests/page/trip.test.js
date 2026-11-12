import { equal, test } from './harness.js';
import { vault } from './vault.js';
import { localDay, parseDate } from '../../static/core/format.js';
import { entities, projects } from '../../static/model/entities.js';
import {
  isPlanned, isTripEvent, nightsOf, placeZone, routeOf, routeSpot, tripPlaces, tripSummary, zoneName,
} from '../../static/model/trip.js';

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

const route = () => routeOf('trip', projects.get('trip').project);

// The route in short: a leg by its state and ends, a stop by its state,
// place and nights.
function shape(parts = route().parts) {
  return parts.map(one => (one.type === 'leg' ? `${one.state} ${one.from} > ${one.to}` : `${one.state} ${one.place} ${one.nights.length}`));
}

const THERE = journey('there', '2027-07-08', 'Springfield', 'Hamburg');
const BACK = journey('back', '2027-07-12', 'Hamburg', 'Springfield');
const HOTEL = stay('hotel', '2027-07-08', '2027-07-12', 'Hamburg');

test("a sketch's nights: so many, about, or a range", () => {
  equal([3, '3', '~4', '2-5', '2\u20135', ' ~ 6 '].map(nightsOf), [
    { min: 3, max: 3, about: false }, { min: 3, max: 3, about: false }, { min: 4, max: 4, about: true },
    { min: 2, max: 5, about: false }, { min: 2, max: 5, about: false }, { min: 6, max: 6, about: true },
  ]);
  equal([nightsOf('5-2').max, nightsOf('some'), nightsOf(null), nightsOf('')], [5, null, null, null]);
});

test('a project that is no trip has no route', () => {
  equal([routeOf('p', { blueprint: 'move' }), routeOf('p', undefined)], [null, null]);
});

test('a trip there, a stay, and back', () => {
  vault(trip(), THERE, HOTEL, BACK);
  equal(shape(), ['booked Springfield > Hamburg', 'booked Hamburg 4', 'booked Hamburg > Springfield']);
  const [, stop] = route().parts;
  equal([localDay(stop.from), localDay(stop.to), stop.stays.map(doc => doc.id)], ['2027-07-08', '2027-07-12', ['hotel']]);
});

test('a trip of no events is a leg there missing nothing it can know', () => {
  vault(trip());
  equal(shape(), ['missing null 4']);
});

test('nights without a stay are missing where the trip is', () => {
  vault(trip(), THERE, stay('hotel', '2027-07-08', '2027-07-10', 'Hamburg'), BACK);
  equal(shape(), ['booked Springfield > Hamburg', 'partial Hamburg 4', 'booked Hamburg > Springfield']);
  equal(route().parts[1].nights.map(night => night.state), ['booked', 'booked', 'missing', 'missing']);
});

test('a change of place without a leg is a leg missing', () => {
  vault(trip(), THERE, stay('hotel', '2027-07-08', '2027-07-10', 'Hamburg'), stay('inn', '2027-07-10', '2027-07-12', 'Berlin'), journey('back', '2027-07-12', 'Berlin', 'Springfield'));
  equal(shape(), ['booked Springfield > Hamburg', 'booked Hamburg 2', 'missing Hamburg > Berlin', 'booked Berlin 2', 'booked Berlin > Springfield']);
  equal(localDay(route().parts[2].day), '2027-07-10');
});

test('without a journey there or back those are missing', () => {
  vault(trip(), HOTEL);
  equal(shape(), ['missing Springfield > Hamburg', 'booked Hamburg 4', 'missing Hamburg > Springfield']);
  equal([localDay(route().parts[0].day), localDay(route().parts[2].day)], ['2027-07-08', '2027-07-12']);
});

test('places are the same whatever their case', () => {
  vault(trip(), journey('there', '2027-07-08', 'springfield', ' Hamburg'), HOTEL, BACK);
  equal(shape().length, 3);
});

test('a journey over night takes its night', () => {
  vault(trip(), journey('there', '2027-07-08', 'Springfield', 'Hamburg', { end: '2027-07-10' }), stay('hotel', '2027-07-09', '2027-07-12', 'Hamburg'), BACK);
  equal(shape(), ['booked Springfield > Hamburg', 'booked Hamburg 3', 'booked Hamburg > Springfield']);
  equal(route().parts[0].nights.map(night => localDay(night.day)), ['2027-07-08']);
});

test('journeys that go on the same day are one leg', () => {
  vault(trip(), journey('a', '2027-07-08', 'Springfield', 'Frankfurt', { by: 'flight' }), journey('b', '2027-07-08', 'Frankfurt', 'Hamburg'), HOTEL, BACK);
  equal(shape()[0], 'booked Springfield > Hamburg');
  equal([route().parts[0].means, route().parts[0].parts.map(doc => doc.id)], [['flight', 'train'], ['a', 'b']]);
});

test('a night between two journeys without a layover parts them', () => {
  vault(trip(), journey('a', '2027-07-08', 'Springfield', 'Paris'), journey('b', '2027-07-09', 'Paris', 'Hamburg'), stay('hotel', '2027-07-09', '2027-07-12', 'Hamburg'), BACK);
  equal(shape(), ['booked Springfield > Paris', 'missing Paris 1', 'booked Paris > Hamburg', 'booked Hamburg 3', 'booked Hamburg > Springfield']);
});

test('what is planned is planned, a stop only while no stay of a night is booked', () => {
  vault(trip(), { ...THERE, event: { ...THERE.event, ...PLANNED } }, stay('hold', '2027-07-08', '2027-07-12', 'Hamburg', PLANNED), BACK);
  equal(shape(), ['planned Springfield > Hamburg', 'planned Hamburg 4', 'booked Hamburg > Springfield']);
  entities.set('hotel', { ...entities.get('hold'), id: 'hotel', doc_id: 'hotel', event: HOTEL.event });
  equal(shape()[1], 'booked Hamburg 4');
  equal([isPlanned(PLANNED), isPlanned({ status: 'confirmed' }), isPlanned(null)], [true, false, false]);
});

test('a cancelled event is not part of the trip', () => {
  vault(trip(), THERE, HOTEL, BACK, stay('gone', '2027-07-08', '2027-07-12', 'Berlin', { status: 'cancelled' }));
  equal(shape()[1], 'booked Hamburg 4');
});

test('while its days are open the route is the sketch', () => {
  const sketch = { stops: [{ place: 'Hamburg', nights: '2-3', by: ['flight', 'train'], overnight: true }, { place: 'Berlin', nights: 2, note: 'at Anna' }], back: { by: 'train' } };
  vault(trip({ days: undefined }, { sketch }));
  const { sketched, parts } = route();
  equal([sketched, shape(parts)], [true, ['idea Springfield > Hamburg', 'idea Hamburg 3', 'idea Hamburg > Berlin', 'idea Berlin 2', 'idea Berlin > Springfield']]);
  equal([parts[0].means, parts[0].nights.length, parts[0].index, parts[4].means, parts[4].index], [['flight', 'train'], 1, 0, ['train'], 2]);
  equal([parts[1].nights.map(night => night.maybe), parts[3].note], [[false, false, true], 'at Anna']);
  vault(trip({ days: { from: '2027-07-08', to: '2027-07-08' } }));
  equal([route().sketched, route().parts], [true, []]);
});

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

test('only the events of the trip that are a part of it are on the route', () => {
  vault(
    trip(), THERE, HOTEL, BACK, { ...stay('other', '2027-07-08', '2027-07-12', 'Berlin'), projects: ['elsewhere'] },
    { ...stay('plain', '2027-07-08', '2027-07-12', 'Berlin'), event: { place: 'Berlin' } }, { ...HOTEL, id: 'note', kind: 'document' },
    journey('before', '2027-07-07', 'Springfield', 'Boston'), journey('after', '2027-07-13', 'Springfield', 'Boston'),
  );
  equal(shape(), ['booked Springfield > Hamburg', 'booked Hamburg 4', 'booked Hamburg > Springfield']);
});

test('the route is in the order of its days', () => {
  vault(trip(), BACK, stay('inn', '2027-07-10', '2027-07-12', 'Hamburg'), stay('hotel', '2027-07-08', '2027-07-10', 'Hamburg'), THERE);
  equal(shape(), ['booked Springfield > Hamburg', 'booked Hamburg 4', 'booked Hamburg > Springfield']);
});

test('without stays the nights are where the journeys take the trip', () => {
  vault(trip(), THERE, BACK);
  equal(shape(), ['booked Springfield > Hamburg', 'missing Hamburg 4', 'booked Hamburg > Springfield']);
  vault(trip({ home: undefined }), journey('on', '2027-07-10', 'Hamburg', 'Berlin'));
  equal(shape(), ['missing Hamburg 2', 'booked Hamburg > Berlin', 'missing Berlin 2']);
});

test('a leg is planned with one of its parts, a stop with one of its nights', () => {
  vault(
    trip(), journey('a', '2027-07-08', 'Springfield', 'Frankfurt', PLANNED), journey('b', '2027-07-08', 'Frankfurt', 'Hamburg'),
    stay('hold', '2027-07-08', '2027-07-10', 'Hamburg', PLANNED), stay('hotel', '2027-07-10', '2027-07-12', 'Hamburg'), BACK,
  );
  equal(shape(), ['planned Springfield > Hamburg', 'planned Hamburg 4', 'booked Hamburg > Springfield']);
});

test("a spot's nights are its own", () => {
  vault(
    trip(), journey('night', '2027-07-08', 'Springfield', 'Hamburg', { end: '2027-07-10' }), stay('hotel', '2027-07-09', '2027-07-10', 'Hamburg'),
    stay('inn', '2027-07-10', '2027-07-11', 'Hamburg'), stay('flat', '2027-07-11', '2027-07-12', 'Berlin'), journey('back', '2027-07-12', 'Berlin', 'Springfield'),
  );
  const spot = id => {
    const { index, stops, legs, nights } = routeSpot(entities.get(id));
    return [index, stops ?? legs, nights.map(localDay)];
  };
  equal(['night', 'hotel', 'inn', 'flat', 'back'].map(spot), [
    [0, 3, ['2027-07-08']], [0, 2, ['2027-07-09']], [0, 2, ['2027-07-10']], [1, 2, ['2027-07-11']], [2, 3, []],
  ]);
});
