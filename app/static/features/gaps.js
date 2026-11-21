// A gap of a trip's route filled (panel.md#route): one menu at the gap --
// on the route, a Missing row of the blueprint, a gap's row on the
// Timeline -- with a draft first, what is missing filled in (`openDraft`), a
// single night marked a layover, then the events outside the trip at that
// time, and a filter finding any other. A picked event joins the trip, takes the gap's kind if it has
// none and the places it lacks, and its card shows, to finish there.

import { DATE_FORMAT, TIME_FORMAT, addDays, dayStart, localDay } from '../core/format.js';
import { newGroup, rpcCall } from '../core/rpc.js';
import { t } from '../core/text.js';
import { projectCalendar } from '../model/blueprint.js';
import { writableCalendar } from '../model/calendar.js';
import { entities } from '../model/entities.js';
import { showError } from '../model/state.js';
import { nightsText } from '../model/trip.js';
import { editEntityLocal, joinProjectLocal } from '../model/structure.js';
import { docIcon } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import { openDraft } from '../model/drafts.js';
import { selectDocument } from './selection.js';

// How many other events the filter adds at most.
const MORE = 20;

// The menu at `anchor` for the gap `gap` of the trip `trip`
// (`tripGaps`, model/trip.js): nights without a stay (`night`: `place`,
// `from`, the morning after `to`) or a change of place without a leg
// (`missing`: its `leg`, `from`, `to` and `day`).
export function fillGap(anchor, trip, gap) {
  const need = gap.kind === 'night'
    ? { type: 'stay', place: gap.place, from: gap.from, to: gap.to }
    : { type: 'journey', from: gap.leg.from, to: gap.leg.to, day: gap.leg.day };
  const [from, to] = need.type === 'stay' ? [need.from, need.to] : [addDays(need.day, -1), addDays(need.day, 2)];
  const single = need.type === 'stay' && addDays(need.from, 1).getTime() === need.to.getTime();
  eventMenu(anchor, {
    items: [newItem(need), ...(single ? [{ label: t('gaps.mark_layover'), icon: '#i-hourglass-half', run: () => markLayover(trip, need) }] : [])],
    head: gapText(gap).title, near: t('gaps.at_that_time'), none: t('gaps.no_other_event'), from, to,
    skip: doc => (doc.projects ?? []).includes(trip),
    pick: doc => pickFor(doc, trip, need),
  });
}

// A menu at `anchor` (`head` over it): `items`, then under the head `near`
// the events on the days `from` to before `to` by their starts (`none`
// without one), and a filter that also finds any other by its title;
// `skip(doc)` leaves an event out, `pick(doc)` takes the one picked. Also
// When's An event\u2026 (features/when.js).
export function eventMenu(anchor, { items = [], head, near: nearHead, none, from, to, skip, pick }) {
  const others = [...entities.values()].filter(doc => doc.kind === 'event' && !isNaN(doc.date)
    && doc.event?.status !== 'cancelled' && !skip(doc));
  const near = others.filter(doc => overlaps(doc, from, to)).sort((a, b) => a.date - b.date);
  const item = doc => ({ label: doc.title || t('untitled'), icon: docIcon(doc), desc: whenText(doc), run: () => pick(doc) });
  const more = text => {
    const needle = text.toLowerCase();
    const found = others.filter(doc => !near.includes(doc) && (doc.title ?? '').toLowerCase().includes(needle))
      .sort((a, b) => Math.abs(a.date - from) - Math.abs(b.date - from)).slice(0, MORE);
    return found.length ? [{ head: t('gaps.other_events'), separator: true }, ...found.map(item)] : [];
  };
  showMenu(anchor, [
    ...items,
    { head: nearHead, separator: items.length > 0 },
    ...(near.length ? near.map(item) : [{ label: none, dim: true }]),
  ], { head, filter: { placeholder: t('gaps.filter_events'), extra: more } });
}

// A gap in words, its title and when: "No stay in Berlin", "the night of
// 07/17"; "No journey from Hamburg to Berlin", its day.
export function gapText(gap) {
  if (gap.kind === 'night') {
    const last = addDays(gap.to, -1);
    const title = gap.place ? t('gaps.no_stay_in', { place: gap.place }) : t('gaps.no_stay');
    return { title, when: nightsText(last > gap.from ? [gap.from, last] : [gap.from]) };
  }
  const { from, to, day } = gap.leg;
  const places = { from: from ?? t('trip.somewhere'), to: to ?? t('trip.somewhere') };
  return { title: t('gaps.no_journey', places), when: day ? DATE_FORMAT.format(day) : '' };
}

// New, with what the gap lacks: a stay at its place for its nights; a journey
// between its places on its day, a day only.
function newItem(need) {
  if (need.type === 'stay') {
    return {
      label: t('gaps.new_stay'), icon: '#i-add',
      run: () => openDraft('event', { event: placed({ type: 'stay', status: 'confirmed', start: localDay(need.from), end: localDay(need.to) }, need.place) }),
    };
  }
  const day = localDay(need.day);
  return {
    label: t('gaps.new_journey'), icon: '#i-add',
    run: () => openDraft('event', {
      event: { type: 'journey', by: 'flight', status: 'confirmed', start: day, end: localDay(addDays(need.day, 1)), ...(need.from && { from: need.from }), ...(need.to && { to: need.to }) },
    }),
  };
}

// `event` at `place`, if the gap names one.
function placed(event, place) {
  return place ? { ...event, place } : event;
}

// The night of `need` a layover, at once: a night on the way between two
// journeys, which the route takes into their leg; in the trip and its
// calendar, booked, and shown.
function markLayover(trip, need) {
  const calendar = writableCalendar(projectCalendar(trip));
  const event = { ...(calendar && { calendar }), status: 'confirmed', start: localDay(need.from), end: localDay(need.to), type: 'layover' };
  if (need.place) event.place = need.place;
  const title = need.place ? t('gaps.layover_in', { place: need.place }) : t('event_type.layover');
  rpcCall('create_entity', { roles: { event }, title, projects: [trip], labels: [] })
    .then(({ id }) => selectDocument(id), e => showError(e.message));
}

// Whether the event `doc` lies on a day from `from` to before `to`.
function overlaps(doc, from, to) {
  const first = dayStart(doc.date);
  const end = !doc.end ? addDays(first, 1) : doc.all_day ? dayStart(doc.end) : addDays(dayStart(doc.end), 1);
  return first < to && end > from;
}

function whenText(doc) {
  return doc.all_day ? DATE_FORMAT.format(doc.date) : `${DATE_FORMAT.format(doc.date)}, ${TIME_FORMAT.format(doc.date)}`;
}

// The event `doc` picked for the gap `need`: it joins the trip, takes the
// gap's kind if it has none, and of that kind the places it lacks; one
// step to undo.
function pickFor(doc, trip, need) {
  const step = newGroup('gap');
  joinProjectLocal(doc.doc_id, trip, step);
  const event = doc.event ?? {};
  const change = event.type ? {} : { type: need.type };
  if ((event.type ?? need.type) === need.type) {
    for (const key of need.type === 'stay' ? ['place'] : ['from', 'to']) {
      if (!event[key] && need[key]) change[key] = need[key];
    }
  }
  if (Object.keys(change).length) editEntityLocal(doc.doc_id, { event: change }, step);
  selectDocument(doc.doc_id);
}
