// A gap of a trip's route filled (panel.md#route): one menu at the gap --
// on the route, a Missing row of the blueprint, a gap's row on the
// Timeline -- with New first, what is missing filled in (`openNew`), then
// the events outside the trip at that time, and a filter finding any
// other. A picked event joins the trip, takes the gap's kind if it has
// none and the places it lacks, and its card shows, to finish there.

import { DATE_FORMAT, TIME_FORMAT, addDays, dayStart, localDay } from '../core/format.js';
import { t } from '../core/text.js';
import { entities } from '../model/entities.js';
import { nightsText } from '../model/trip.js';
import { editEntityLocal, joinProjectLocal } from '../model/structure.js';
import { docIcon } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import { openNew } from './new.js';
import { selectDocument } from './selection.js';

// How many other events the filter adds at most.
const MORE = 20;

// The menu at `anchor` for the gap `gap` of the trip `trip`
// (`tripGaps`, model/trip.js): nights without a stay (`night`: `place`,
// `from`, the morning after `to`) or a change of place without a leg
// (`missing`: its `leg`, `from`, `to` and `day`).
export function fillGap(anchor, trip, gap) {
  const need = gap.kind === 'night'
    ? { kind: 'stay', place: gap.place, from: gap.from, to: gap.to }
    : { kind: 'transport', from: gap.leg.from, to: gap.leg.to, day: gap.leg.day };
  const [from, to] = need.kind === 'stay' ? [need.from, need.to] : [addDays(need.day, -1), addDays(need.day, 2)];
  const others = [...entities.values()].filter(doc => doc.kind === 'event' && !isNaN(doc.date)
    && doc.event?.status !== 'cancelled' && !(doc.projects ?? []).includes(trip));
  const near = others.filter(doc => overlaps(doc, from, to)).sort((a, b) => a.date - b.date);
  const item = doc => ({ label: doc.title || t('untitled'), icon: docIcon(doc), desc: whenText(doc), run: () => pickFor(doc, trip, need) });
  const more = text => {
    const needle = text.toLowerCase();
    const found = others.filter(doc => !near.includes(doc) && (doc.title ?? '').toLowerCase().includes(needle))
      .sort((a, b) => Math.abs(a.date - from) - Math.abs(b.date - from)).slice(0, MORE);
    return found.length ? [{ head: t('gaps.other_events'), separator: true }, ...found.map(item)] : [];
  };
  showMenu(anchor, [
    newItem(need),
    { head: t('gaps.at_that_time'), separator: true },
    ...(near.length ? near.map(item) : [{ label: t('gaps.no_other_event'), dim: true }]),
  ], { head: gapText(gap).title, filter: { placeholder: t('gaps.filter_events'), extra: more } });
}

// A gap in words, its title and when: "No stay in Berlin", "the night of
// 07/17"; "No way from Hamburg to Berlin", its day.
export function gapText(gap) {
  if (gap.kind === 'night') {
    const last = addDays(gap.to, -1);
    const title = gap.place ? t('gaps.no_stay_in', { place: gap.place }) : t('gaps.no_stay');
    return { title, when: nightsText(last > gap.from ? [gap.from, last] : [gap.from]) };
  }
  const { from, to, day } = gap.leg;
  const places = { from: from ?? t('trip.somewhere'), to: to ?? t('trip.somewhere') };
  return { title: t('gaps.no_way', places), when: day ? DATE_FORMAT.format(day) : '' };
}

// New, with what the gap lacks: a stay at its place for its nights; a way
// between its places on its day, a day only.
function newItem(need) {
  if (need.kind === 'stay') {
    return {
      label: t('gaps.new_stay'), icon: '#i-add',
      run: () => openNew('event', { eventKind: 'stay', place: need.place ?? '', start: localDay(need.from), end: localDay(need.to) }),
    };
  }
  const day = localDay(need.day);
  return {
    label: t('gaps.new_way'), icon: '#i-add',
    run: () => openNew('event', { eventKind: 'transport', from: need.from ?? '', to: need.to ?? '', start: day, end: day, startTime: '', endTime: '' }),
  };
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
// gap's kind if it has none, and of that kind the places it lacks.
function pickFor(doc, trip, need) {
  joinProjectLocal(doc.doc_id, trip);
  const event = doc.event ?? {};
  const change = event.kind ? {} : { kind: need.kind };
  if ((event.kind ?? need.kind) === need.kind) {
    for (const key of need.kind === 'stay' ? ['place'] : ['from', 'to']) {
      if (!event[key] && need[key]) change[key] = need[key];
    }
  }
  if (Object.keys(change).length) editEntityLocal(doc.doc_id, { event: change });
  selectDocument(doc.doc_id);
}
