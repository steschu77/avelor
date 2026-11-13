import { equal, test } from './harness.js';
import { vault } from './vault.js';
import { isoInstant, localDay } from '../../static/core/format.js';
import { whenOf } from '../../static/model/when.js';

// July 2027: the 1st is a Thursday.
const sketch = (window, nights, more = {}) => ({ window, nights, ...more });
const JULY = { from: '2027-07-01', to: '2027-07-31' };
const spans = (counts, ...list) => ({ counts, name: counts, spans: list.map(([from, to, note]) => ({ from, to, note })) });
const days = (...list) => list.map(one => [localDay(one.from), localDay(one.to)]);
const fitsOf = project => whenOf(project).fits.map(fit => [fit.letter, localDay(fit.from), localDay(fit.to)]);
const event = (id, calendar, date, end, more = {}) => ({ id, kind: 'event', title: id, date, event: { calendar, end, ...more } });

test('a trip whose dates are set, or that is no sketch, has no When', () => {
  equal([whenOf({ sketch: sketch(JULY, 3), facts: { days: {} } }), whenOf({}), whenOf(null)], [null, null, null]);
});

test('without a window there is nothing to place', () => {
  const none = { from: null, to: null, days: 0, nights: { min: 3, max: 3, about: false }, lanes: [], fits: [] };
  for (const window of [undefined, { from: '2027-07-31', to: '2027-07-01' }, { from: '2027-07-01' }, { to: '2027-07-01' }, { from: 'x', to: '2027-07-01' }, { from: '2027-07-01', to: 'x' }]) {
    equal(whenOf({ sketch: sketch(window, 3) }), none);
  }
});

test('the earliest days fit a free window', () => {
  const when = whenOf({ sketch: sketch(JULY, 3) });
  equal([when.days, fitsOf({ sketch: sketch(JULY, 3) })], [31, [['A', '2027-07-01', '2027-07-04']]]);
});

test('a trip takes as many nights as fit, at least its fewest', () => {
  equal(fitsOf({ sketch: sketch({ from: '2027-07-01', to: '2027-07-05' }, '3-7') }), [['A', '2027-07-01', '2027-07-05']]);
  equal(fitsOf({ sketch: sketch({ from: '2027-07-01', to: '2027-07-03' }, '3-7') }), []);
  equal(fitsOf({ sketch: sketch(JULY, undefined) }), []);
});

test('without its own length the trip is as long as its stops and the journeys over night', () => {
  const stops = [{ nights: '2-3' }, {}, { nights: 2, overnight: true }];
  equal(whenOf({ sketch: sketch(JULY, undefined, { stops }) }).nights, { min: 6, max: 7, about: false });
});

test("the days that can't be part the window, a fit in each stretch", () => {
  const project = { sketch: sketch(JULY, 5), lanes: [spans('cant', ['2027-07-10', '2027-07-14', 'fair'], ['2027-07-20', undefined, 'visit'])] };
  equal(fitsOf(project), [['A', '2027-07-01', '2027-07-06'], ['B', '2027-07-21', '2027-07-26']]);
  const [first, second] = whenOf(project).fits;
  equal([[first.before, first.after], [second.before, second.after]], [[null, 'fair'], ['visit', null]]);
});

test('a lane is shown as far as the window goes', () => {
  const project = { sketch: sketch(JULY, 5), lanes: [spans('cant', ['2027-06-20', '2027-07-02'], ['2027-07-30', '2027-08-03'], ['2027-09-01', '2027-09-02'], ['x', 'y'])] };
  const [lane] = whenOf(project).lanes;
  equal([days(...lane.spans), lane.spans.map(span => span.index)], [[['2027-07-01', '2027-07-02'], ['2027-07-30', '2027-07-31']], [0, 1]]);
});

test('the days with most reasons are taken, then those with fewest worse days', () => {
  const reasons = { sketch: sketch(JULY, 3), lanes: [spans('reason', ['2027-07-20', '2027-07-20', 'concert'], ['2027-07-22', '2027-07-22', 'fair'])] };
  equal(fitsOf(reasons), [['A', '2027-07-19', '2027-07-22']]);
  equal(whenOf(reasons).fits[0].reasons, ['concert', 'fair']);
  const worse = { sketch: sketch({ from: '2027-07-01', to: '2027-07-08' }, 3), lanes: [spans('worse', ['2027-07-02', '2027-07-04', 'heat'])] };
  equal(fitsOf(worse), [['A', '2027-07-05', '2027-07-08']]);
});

test("a calendar's all-day events limit the days, a recurring one each time, a timed or cancelled one not", () => {
  vault(
    { id: 'school', kind: 'calendar', title: 'School' }, event('exam', 'school', '2027-07-05', '2027-07-06', { schedule: ['RRULE:FREQ=WEEKLY;COUNT=2'] }),
    event('talk', 'school', '2027-07-20T10:00:00Z', '2027-07-20T11:00:00Z'), event('off', 'school', '2027-07-21', '2027-07-22', { status: 'cancelled' }),
    event('other', 'family', '2027-07-25', '2027-07-26'),
  );
  const [lane] = whenOf({ sketch: sketch(JULY, 5), lanes: [{ calendar: 'school' }] }).lanes;
  equal([lane.name, lane.counts, days(...lane.spans)], ['School', 'cant', [['2027-07-05', '2027-07-05'], ['2027-07-12', '2027-07-12']]]);
});

test("a lane counted only leaves out the days outside its events, and says what lies around", () => {
  vault({ id: 'holidays', kind: 'calendar', title: 'Holidays' }, event('first', 'holidays', '2027-07-05', '2027-07-12'), event('second', 'holidays', '2027-07-20', '2027-07-28'));
  const project = { sketch: sketch(JULY, 5), lanes: [{ calendar: 'holidays', counts: 'only' }] };
  const [lane] = whenOf(project).lanes;
  equal(days(...lane.spans), [['2027-07-01', '2027-07-04'], ['2027-07-12', '2027-07-19'], ['2027-07-28', '2027-07-31']]);
  equal(lane.spans.map(span => span.detail), ['when.before', 'when.between', 'when.after']);
  equal(fitsOf(project), [['A', '2027-07-05', '2027-07-10'], ['B', '2027-07-20', '2027-07-25']]);
  equal(whenOf({ sketch: sketch(JULY, 5), lanes: [{ calendar: 'gone', counts: 'only' }] }).lanes[0].spans.map(span => span.detail), ['when.none_of_its_days']);
});

test('a lane of one event, and one that is gone', () => {
  vault(event('wedding', 'family', '2027-07-10', '2027-07-12'));
  const [lane, gone] = whenOf({ sketch: sketch(JULY, 5), lanes: [{ event: 'wedding', counts: 'reason' }, { event: 'deleted' }] }).lanes;
  equal([lane.name, days(...lane.spans), gone.name, gone.spans], ['wedding', [['2027-07-10', '2027-07-11']], 'when.event_gone', []]);
});

test('a lane that counts for nothing is left out, one unknown as days that can not be', () => {
  const lanes = whenOf({ sketch: sketch(JULY, 5), lanes: [spans('none', ['2027-07-01']), spans('never', ['2027-07-02'])] }).lanes;
  equal(lanes.map(lane => [lane.index, lane.counts]), [[1, 'cant']]);
});

test("a traveler's work calendar is a lane of its own, its days off cost no leave", () => {
  vault(
    { id: 'maya', kind: 'contact', title: 'Maya' }, { id: 'work', kind: 'calendar', title: 'Work', calendar: { 'work-of': '@maya' } },
    event('leave', 'work', '2027-07-05', '2027-07-07', { work: 'off' }), event('home', 'work', '2027-07-08', '2027-07-09', { work: 'remote' }),
  );
  const project = { sketch: sketch({ from: '2027-07-05', to: '2027-07-11' }, 6), facts: { travelers: ['@maya'] } };
  const [lane] = whenOf(project).lanes;
  equal([lane.key, lane.work, lane.spans.map(span => span.counts)], ['work:work', 'Maya', ['reason', 'worse']]);
  // Monday to Sunday: two days off, one remote, two to take.
  equal(whenOf(project).fits[0].cost, [{ traveler: 'Maya', workdays: 2, off: 2, remote: 1 }]);
});

test('of days alike those that cost the fewest workdays are taken', () => {
  vault({ id: 'maya', kind: 'contact', title: 'Maya' }, { id: 'work', kind: 'calendar', title: 'Work', calendar: { 'work-of': '@maya' } });
  equal(fitsOf({ sketch: sketch({ from: '2027-07-05', to: '2027-07-18' }, 2), facts: { travelers: ['@maya'] } }), [['A', '2027-07-09', '2027-07-11']]);
});

test('a window of one day', () => {
  equal(fitsOf({ sketch: sketch({ from: '2027-07-01', to: '2027-07-01' }, 0) }), [['A', '2027-07-01', '2027-07-01']]);
});

test("a lane's days on the window's first and last are in it", () => {
  const [lane] = whenOf({ sketch: sketch(JULY, 5), lanes: [spans('cant', ['2027-06-20', '2027-07-01'], ['2027-07-31', '2027-08-03'])] }).lanes;
  equal(days(...lane.spans), [['2027-07-01', '2027-07-01'], ['2027-07-31', '2027-07-31']]);
});

test('a span is named by its note, else by its lane', () => {
  const [lane, nameless] = whenOf({ sketch: sketch(JULY, 5), lanes: [spans('cant', ['2027-07-01', '2027-07-01', 'fair'], ['2027-07-02']), { spans: [{ from: '2027-07-03' }] }] }).lanes;
  equal([lane.spans.map(span => span.title), nameless.name], [['fair', 'cant'], 'untitled']);
});

test('the only days of several events, out of order, side by side and inside each other', () => {
  vault(event('late', 'c', '2027-07-12', '2027-07-20'), event('early', 'c', '2027-07-05', '2027-07-12'), event('inner', 'c', '2027-07-06', '2027-07-08'), { id: 'c', kind: 'calendar', title: 'C' });
  const [lane] = whenOf({ sketch: sketch(JULY, 5), lanes: [{ calendar: 'c', counts: 'only' }] }).lanes;
  equal(days(...lane.spans), [['2027-07-01', '2027-07-04'], ['2027-07-20', '2027-07-31']]);
});

test('a timed event to midnight is on its day alone', () => {
  vault(event('party', 'c', isoInstant(new Date(2027, 6, 10, 18)), isoInstant(new Date(2027, 6, 11))));
  equal(days(...whenOf({ sketch: sketch(JULY, 5), lanes: [{ event: 'party' }] }).lanes[0].spans), [['2027-07-10', '2027-07-10']]);
});

test('a work calendar listed is not listed again, its days off a reason all the same', () => {
  vault(
    { id: 'maya', kind: 'contact', title: 'Maya' }, { id: 'work', kind: 'calendar', title: 'Work', calendar: { 'work-of': '@maya' } },
    event('leave', 'work', '2027-07-05', '2027-07-06', { work: 'off' }), event('fair', 'work', '2027-07-08', '2027-07-09'),
  );
  const { lanes } = whenOf({ sketch: sketch(JULY, 5), facts: { travelers: ['@maya'] }, lanes: [{ calendar: 'work', counts: 'worse' }] });
  equal(lanes.map(lane => [lane.key, lane.name, lane.spans.map(span => span.counts)]), [['lane:0', 'when.work_of', ['reason', 'worse']]]);
});

test("a trip's first and last day count, and only the days in it", () => {
  const project = { sketch: sketch({ from: '2027-07-01', to: '2027-07-04' }, 3), lanes: [
    spans('reason', ['2027-07-01', '2027-07-01', 'first'], ['2027-07-04', '2027-07-04', 'last']), spans('worse', ['2027-07-01', '2027-07-01', 'rain'], ['2027-07-04', '2027-07-04', 'storm']),
  ] };
  equal([whenOf(project).fits[0].reasons, whenOf(project).fits[0].worse], [['first', 'last'], ['rain', 'storm']]);
  const longer = { sketch: sketch({ from: '2027-07-01', to: '2027-07-10' }, 3), lanes: [spans('worse', ['2027-07-01', '2027-07-05', 'rain'])] };
  equal([fitsOf(longer), whenOf(longer).fits[0].worse], [[['A', '2027-07-06', '2027-07-09']], []]);
});

test('what lies around a fit is the nearest on each side', () => {
  const lane = spans('cant', ['2027-07-25', undefined, 'four'], ['2027-07-01', '2027-07-02', 'one'], ['2027-07-20', undefined, 'three'], ['2027-07-05', '2027-07-06', 'two']);
  const fits = whenOf({ sketch: sketch(JULY, 5), lanes: [lane] }).fits;
  equal(fits.map(fit => [localDay(fit.from), fit.before, fit.after]), [['2027-07-07', 'two', 'three'], ['2027-07-26', 'four', null]]);
});

test("a work calendar's days off at the window's ends cost no leave", () => {
  vault(
    { id: 'maya', kind: 'contact', title: 'Maya' }, { id: 'work', kind: 'calendar', title: 'Work', calendar: { 'work-of': '@maya' } },
    event('before', 'work', '2027-07-01', '2027-07-06', { work: 'off' }), event('after', 'work', '2027-07-09', '2027-07-20', { work: 'off' }),
  );
  const project = { sketch: sketch({ from: '2027-07-05', to: '2027-07-09' }, 4), facts: { travelers: ['@maya'] } };
  equal(whenOf(project).fits[0].cost, [{ traveler: 'Maya', workdays: 3, off: 2, remote: 0 }]);
});
