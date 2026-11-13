import { equal, test } from './harness.js';
import { vault } from './vault.js';
import { localDay, parseDate } from '../../static/core/format.js';
import { entities, projects } from '../../static/model/entities.js';
import {
  byNights, setDatesProposal, isPlanned, isTripEvent, issueText, nightsOf, nightsText, placeZone, routeOf, routeSpot, tripGaps,
  tripPlaces, tripSummary, zoneIssuesOf, zoneName,
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
const gaps = () => tripGaps('trip', projects.get('trip').project);

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

test('a layover keeps the journeys around it one leg, over its night', () => {
  vault(
    trip(), journey('a', '2027-07-08', 'Springfield', 'Paris'), part('layover', 'layover', '2027-07-08', '2027-07-09', { place: 'Paris' }),
    journey('b', '2027-07-09', 'Paris', 'Hamburg'), stay('hotel', '2027-07-09', '2027-07-12', 'Hamburg'), BACK,
  );
  equal(shape(), ['booked Springfield > Hamburg', 'booked Hamburg 3', 'booked Hamburg > Springfield']);
  equal([route().parts[0].layovers.map(doc => doc.id), route().parts[0].nights.length], [['layover'], 1]);
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

test('a place left in another zone than it was reached in is told', () => {
  vault(
    trip(), flight('there', '2027-07-08T10:00:00Z', '2027-07-08T12:00:00Z', 'Springfield', 'Hamburg', 'America/Chicago', 'Europe/Berlin'), HOTEL,
    flight('back', '2027-07-12T10:00:00Z', '2027-07-12T20:00:00Z', 'Hamburg', 'Springfield', 'Europe/London', 'America/Chicago'),
  );
  const [issue] = route().parts[2].issues;
  equal([issue.kind, issue.place, issue.arrive.zone, issue.leave.zone], ['zones', 'Hamburg', 'Europe/Berlin', 'Europe/London']);
  equal([zoneIssuesOf(entities.get('back')).start?.kind, zoneIssuesOf(entities.get('there')).end?.kind, zoneIssuesOf(entities.get('there')).start], ['zones', 'zones', undefined]);
  equal(issueText(issue), 'trip.issue.zones_leaves_at');
});

test('a leg goes on from where it arrived, else its parts are apart', () => {
  vault(
    trip(), flight('a', '2027-07-08T10:00:00Z', '2027-07-08T12:00:00Z', 'Springfield', 'Frankfurt', 'America/Chicago', 'Europe/Berlin'),
    flight('b', '2027-07-08T14:00:00Z', '2027-07-08T15:00:00Z', 'Munich', 'Hamburg', 'Europe/Berlin', 'Europe/Berlin'), HOTEL, BACK,
  );
  const [issue] = route().parts[0].issues;
  equal([issue.kind, issue.arrive.place, issue.leave.place], ['apart', 'Frankfurt', 'Munich']);
  equal(issueText(issue), 'trip.issue.apart');
});

test('an activity at a stop is in the zone the trip is in', () => {
  vault(
    trip(), flight('there', '2027-07-08T10:00:00Z', '2027-07-08T12:00:00Z', 'Springfield', 'Hamburg', 'America/Chicago', 'Europe/Berlin'), HOTEL, BACK,
    part('tour', 'activity', '2027-07-09T10:00:00Z', '2027-07-09T12:00:00Z', { place: 'Hamburg', tz: 'America/Chicago' }),
  );
  equal(route().parts[1].issues.map(issue => [issue.kind, issue.leave.doc.id]), [['zones', 'tour']]);
});

test('what a trip lacks, in the order of its route', () => {
  vault(
    trip(), journey('there', '2027-07-08', 'Springfield', 'Hamburg', PLANNED), stay('hotel', '2027-07-08', '2027-07-10', 'Hamburg'),
    { id: 'book', kind: 'task', title: 'Book the train', linked_to: 'there', task: { status: 'open' } },
    { id: 'old', kind: 'task', title: 'Ask', linked_to: 'there', task: { status: 'done' } },
  );
  const found = gaps();
  equal(found.gaps.map(gap => [gap.rule, gap.kind]), [['journeys', 'planned'], ['nights', 'night'], ['journeys', 'missing']]);
  equal([found.gaps[0].task.id, localDay(found.gaps[1].from), localDay(found.gaps[1].to), found.gaps[1].place], ['book', '2027-07-10', '2027-07-12', 'Hamburg']);
  equal([found.progress.nights, found.progress.journeys, found.later], [[2, 4], [0, 2], null]);
});

test('a booking without a document is a ticket missing', () => {
  const booked = { booking: 'X7K2PL' };
  vault(
    trip(), journey('there', '2027-07-08', 'Springfield', 'Hamburg', booked), HOTEL, journey('back', '2027-07-12', 'Hamburg', 'Springfield', booked),
    { id: 'ticket', kind: 'document', title: 'Ticket', linked_to: 'there' }, { id: 'task', kind: 'task', title: 'Print', linked_to: 'back', task: { status: 'open' } },
  );
  equal(gaps().gaps.map(gap => [gap.kind, gap.doc.id]), [['ticket', 'back']]);
});

test("a trip's event outside its days is told", () => {
  vault(trip(), THERE, HOTEL, BACK, stay('early', '2027-07-06', '2027-07-07', 'Hamburg'), part('party', 'activity', '2027-07-12', '2027-07-13', { place: 'Springfield' }));
  equal(gaps().gaps.map(gap => [gap.kind, gap.doc.id]), [['outside', 'early']]);
});

test('a trip with nothing missing', () => {
  vault(trip(), THERE, HOTEL, BACK);
  equal([gaps().gaps, gaps().progress.nights, gaps().progress.journeys], [[], [4, 4], [2, 2]]);
});

test('what a sketch will need once its dates are set', () => {
  const later = (sketch, facts = { days: undefined }) => {
    vault(trip(facts, { sketch }));
    return gaps().later;
  };
  equal(later({ nights: '6-8', stops: [{ place: 'Hamburg', nights: 2 }] }), { nights: 7, journeys: 2 });
  equal(later({ stops: [{ nights: '2-3' }, { nights: '~4' }] }), { nights: 7, journeys: 3 });
  equal([later({ stops: [{ nights: 2 }, {}] }), later({}), later(undefined)], [{ nights: null, journeys: 3 }, { nights: null, journeys: 0 }, { nights: null, journeys: 0 }]);
});

const WORK = { id: 'work', kind: 'calendar', title: 'Work', calendar: { 'work-of': '@maya' } };

test('setting the dates proposes the journeys, the stays and the days off', () => {
  const sketch = { nights: 4, stops: [{ place: 'Hamburg', nights: '1-2', by: ['flight', 'train'] }, { place: 'Berlin', note: 'anna\u2019s flat', by: 'train' }], back: { by: 'flight' } };
  vault(trip({ days: undefined, travelers: ['@maya'] }, { sketch }), { id: 'maya', kind: 'contact', title: 'Maya' }, WORK);
  const found = setDatesProposal('trip', projects.get('trip').project, DAYS, 'family');
  equal(found.journeys.map(row => [row.event.start, row.event.by, row.event.from ?? null, row.event.to ?? null]), [
    ['2027-07-08', 'flight', 'Springfield', null], ['2027-07-08', 'train', null, 'Hamburg'], ['2027-07-09', 'train', 'Hamburg', 'Berlin'], ['2027-07-12', 'flight', 'Berlin', 'Springfield'],
  ]);
  equal(found.stays.map(row => [row.title, row.event.start, row.event.end, row.event.place, row.event.status, row.event.calendar]), [
    ['trip.stay_in', '2027-07-08', '2027-07-09', 'Hamburg', 'tentative', 'family'], ['Anna\u2019s flat', '2027-07-09', '2027-07-12', 'Berlin', 'tentative', 'family'],
  ]);
  equal(found.off.map(row => row.event), [{ calendar: 'work', status: 'tentative', start: '2027-07-08', end: '2027-07-13', work: 'off' }]);
  equal([found.nights, found.sketched.min], [4, 4]);
  equal(found.journeys.map(row => [row.title, row.why]), [
    ['trip.journey_from', 'Springfield \u2192 Hamburg \u00b7 trip.part_of'], ['trip.journey_to', 'Springfield \u2192 Hamburg \u00b7 trip.part_of'],
    ['trip.journey_to', 'Hamburg \u2192 Berlin'], ['trip.journey_to', 'Berlin \u2192 Springfield'],
  ]);
  equal(found.stays.map(row => row.why), ['Hamburg \u00b7 trip.nights \u00b7 trip.as_sketched', 'Berlin \u00b7 trip.nights \u00b7 trip.left_sketch_said']);
});

test('a journey over night takes a night of the days, and a stop left without one none', () => {
  const sketch = { stops: [{ place: 'Hamburg', nights: 9, by: 'ferry', overnight: true }, { place: 'Berlin', nights: 2 }] };
  vault(trip({ days: undefined }, { sketch }));
  const found = setDatesProposal('trip', projects.get('trip').project, DAYS, null);
  equal(found.journeys.map(row => [row.event.start, row.event.end, row.event.by ?? null]), [['2027-07-08', '2027-07-10', 'ferry'], ['2027-07-12', '2027-07-13', null], ['2027-07-12', '2027-07-13', null]]);
  equal(found.stays.map(row => [row.event.start, row.event.end, row.event.calendar, row.why]), [['2027-07-09', '2027-07-12', undefined, 'Hamburg \u00b7 trip.nights \u00b7 trip.sketch_said']]);
});

test('where an event is on the route', () => {
  vault(
    trip(), journey('a', '2027-07-08', 'Springfield', 'Paris'), part('layover', 'layover', '2027-07-08', '2027-07-09', { place: 'Paris' }),
    journey('b', '2027-07-09', 'Paris', 'Hamburg'), stay('hotel', '2027-07-09', '2027-07-12', 'Hamburg'), BACK, part('tour', 'activity', '2027-07-10', '2027-07-11', {}),
  );
  const spot = id => {
    const { leg, stop, ...rest } = routeSpot(entities.get(id)) ?? {};
    return leg || stop ? { at: leg ? 'leg' : 'stop', ...rest, nights: rest.nights.map(localDay) } : null;
  };
  equal(spot('b'), { at: 'leg', index: 0, legs: 2, part: 1, nights: [] });
  equal(spot('layover'), { at: 'leg', index: 0, legs: 2, layover: true, nights: ['2027-07-08'] });
  equal(spot('hotel'), { at: 'stop', index: 0, stops: 1, nights: ['2027-07-09', '2027-07-10', '2027-07-11'] });
  equal(spot('tour'), null);
});

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

test('a stay and a layover are by their nights', () => {
  equal(['stay', 'layover', 'journey', 'activity'].map(type => byNights({ type })), [true, true, false, false]);
});

test('what a row tells of a stay', () => {
  vault(stay('hotel', '2027-07-08', '2027-07-12', 'Hamburg', PLANNED), part('tour', 'activity', '2027-07-10', '2027-07-11', { place: 'Altona' }), journey('train', '2027-07-08', 'a', 'b', { booking: 'X7' }));
  equal(['hotel', 'tour', 'train'].map(id => tripSummary(entities.get(id))), ['trip.planned \u00b7 Hamburg \u00b7 trip.nights', 'Altona', 'X7']);
});

test('nights in words', () => {
  equal([nightsText([]), nightsText([new Date(2027, 6, 8)]), nightsText([new Date(2027, 6, 8), new Date(2027, 6, 9)])], [null, 'trip.night_of', 'trip.nights_of']);
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

test('a layover that ends before the next journey parts them', () => {
  vault(
    trip(), journey('a', '2027-07-08', 'Springfield', 'Paris'), part('layover', 'layover', '2027-07-08', '2027-07-09', { place: 'Paris' }),
    journey('b', '2027-07-10', 'Paris', 'Hamburg'), stay('hotel', '2027-07-10', '2027-07-12', 'Hamburg'), BACK,
  );
  equal(shape(), ['booked Springfield > Paris', 'partial Paris 2', 'booked Paris > Hamburg', 'booked Hamburg 2', 'booked Hamburg > Springfield']);
});

test('a journey on a day only, or after a leg missing, says nothing of zones', () => {
  const back = flight('back', '2027-07-12T10:00:00Z', '2027-07-12T20:00:00Z', 'Hamburg', 'Springfield', 'Europe/London', 'America/Chicago');
  vault(trip(), THERE, HOTEL, back);
  equal(route().parts[2].issues, []);
  vault(
    trip(), flight('there', '2027-07-08T10:00:00Z', '2027-07-08T12:00:00Z', 'Springfield', 'Berlin', 'America/Chicago', 'Europe/Berlin'), HOTEL, back,
  );
  equal(route().parts.filter(one => one.issues?.length), []);
});

test("an activity is a stop's on its days and at its place, or without one", () => {
  const activity = (id, start, place) => part(id, 'activity', start, start, { place, tz: 'America/Chicago' });
  vault(
    trip(), flight('there', '2027-07-08T10:00:00Z', '2027-07-08T12:00:00Z', 'Springfield', 'Hamburg', 'America/Chicago', 'Europe/Berlin'), HOTEL, BACK,
    activity('first', '2027-07-08T18:00:00Z', 'Hamburg'), activity('last', '2027-07-12T18:00:00Z'), activity('elsewhere', '2027-07-09T18:00:00Z', 'Berlin'),
    activity('before', '2027-07-07T18:00:00Z'), activity('after', '2027-07-13T18:00:00Z', 'Hamburg'),
  );
  equal(route().parts[1].issues.map(issue => issue.leave.doc.id), ['first', 'last']);
});

test("a leg arrives in its last part's zone", () => {
  vault(
    trip(), flight('a', '2027-07-08T10:00:00Z', '2027-07-08T12:00:00Z', 'Springfield', 'Frankfurt', 'America/Chicago', 'Europe/Berlin'),
    flight('b', '2027-07-08T14:00:00Z', '2027-07-08T15:00:00Z', 'Frankfurt', 'London', 'Europe/Berlin', 'Europe/London'), stay('hotel', '2027-07-08', '2027-07-12', 'London'),
    flight('back', '2027-07-12T10:00:00Z', '2027-07-12T20:00:00Z', 'London', 'Springfield', 'Europe/Berlin', 'America/Chicago'),
  );
  equal([route().parts[0].issues, route().parts[2].issues.map(issue => issue.arrive.zone)], [[], ['Europe/London']]);
});

test('parts of a leg without their places are not apart', () => {
  vault(
    trip(), flight('a', '2027-07-08T10:00:00Z', '2027-07-08T12:00:00Z', 'Springfield', undefined, 'America/Chicago', 'Europe/Berlin'),
    flight('b', '2027-07-08T14:00:00Z', '2027-07-08T15:00:00Z', 'Munich', 'Hamburg', 'Europe/Berlin', 'Europe/Berlin'), HOTEL, BACK,
  );
  equal(route().parts[0].issues, []);
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

test('the tickets and what is outside come by their days', () => {
  const booked = { booking: 'X7K2PL' };
  vault(
    trip(), journey('back', '2027-07-12', 'Hamburg', 'Springfield', booked), { ...journey('there', '2027-07-08', 'Springfield', 'Hamburg', booked), linked_to: 'task' },
    { ...journey('tour', '2027-07-10', 'Hamburg', 'Hamburg', booked), linked_to: 'ticket' }, stay('late', '2027-07-11', '2027-07-14', 'Hamburg'),
    stay('early', '2027-07-06', '2027-07-07', 'Hamburg'), HOTEL, { id: 'ticket', kind: 'document', title: '' }, { id: 'task', kind: 'task', title: '', task: { status: 'open' } },
  );
  equal(gaps().gaps.filter(gap => gap.doc).map(gap => [gap.kind, gap.doc.id]), [['ticket', 'there'], ['ticket', 'back'], ['outside', 'early'], ['outside', 'late']]);
});

test('a sketch without places is somewhere', () => {
  vault(trip({ home: undefined, days: undefined }, { sketch: { stops: [{ nights: 2 }] } }));
  const found = setDatesProposal('trip', projects.get('trip').project, DAYS, null);
  equal(found.journeys.map(row => [row.title, row.why]), [['trip.journey_to', 'trip.somewhere_first \u2192 trip.somewhere'], ['trip.journey_to', 'trip.somewhere_first \u2192 trip.somewhere']]);
  equal(found.stays.map(row => [row.title, row.why]), [['trip.stay_somewhere', 'trip.nights \u00b7 trip.left_sketch_said']]);
});
