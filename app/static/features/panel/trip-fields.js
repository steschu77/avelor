// A trip's event on its card (panel.md#trip-fields): what part of the trip
// it is and planned or booked, then its own fields by that -- a journey's
// means, where it leaves from and arrives, both ends' times each in its own
// place's time zone with how long it takes, and its booking; a stay's place
// and nights; an activity's place and times -- and where it is. While the
// host can write it, each field writes by itself through `edit_entities`
// (`editEntityLocal`, an `{event: {...}}` change); a time goes with its zone.
// In a calendar Google keeps as it is (`googleLocked`), what it syncs -- planned
// or booked, the times, where -- is shown as text, the rest still edited.
// The place inputs offer the places of the vault's trip events, and a
// place named turns to the zone it had last (`placeZone`). Also New's
// place inputs (features/new.js). After its calendar, what it meets on its
// trip's route (`meetsField`). The times and Where are every event's
// (features/panel/event-fields.js).

import { accEl } from '../../core/dom.js';
import { DAY_MONTH, addDays, daysBetween, localDay, parseDate } from '../../core/format.js';
import { t } from '../../core/text.js';
import { googleLocked } from '../../model/calendar.js';
import { editEntityLocal } from '../../model/structure.js';
import { EVENT_TYPES, STATES, TRIP_TYPES, MEANS, isPlanned, placeZone, routeSpot, tripPlaces } from '../../model/trip.js';
import { editField, fieldInput } from '../../ui/field.js';
import { MEANS_ICONS } from '../../ui/icons.js';
import { accSeg, keyedSeg } from '../../ui/segmented.js';
import { timeFields, whenIn, whereField, zoneChange, zoneOf } from './event-fields.js';


// ----------------------------------------------------------------------------
// The trip fields of the event `doc` into the card's grid (`add(label,
// ...content)`, features/panel/details.js); as text while the host can't
// write it.
export function tripFields(doc, add) {
  if (!doc.writable) return tripText(doc, add);
  const { doc_id: id, event } = doc;
  const locked = googleLocked(doc);
  const edit = change => editEntityLocal(id, { event: change });
  // a place named moves its end into the place's zone, but where Google
  // keeps the times
  const move = (end, place) => (locked ? {} : placeMove(doc, end, place));
  const times = end => (locked ? [whenIn(doc, end)] : timeFields(doc, end, edit));
  add(t('props.type'), eventTypeField(event, edit));
  const state = isPlanned(event) ? 'tentative' : 'confirmed';
  if (locked) add(t('props.state'), isPlanned(event) ? t('trip.planned') : t('trip.booked'));
  else add(t('props.state'), keyedSeg(accSeg(STATES, state, status => edit({ status })), STATES, 'state'));
  if (event.type === 'journey') {
    const means = MEANS.map(([value, label]) => [value, label, { icon: MEANS_ICONS[value] }]);
    add(t('props.by'), keyedSeg(accSeg(means, event.by ?? null, by => edit({ by })), MEANS, 'by'));
    add(t('props.from'), placeField('from', event.from, from => edit({ from, ...move('start', from) })));
    add(t('props.leaves'), ...times('start'));
    add(t('props.to'), placeField('to', event.to, to => edit({ to, ...move('end', to) })));
    if (!locked || doc.end) add(t('props.arrives'), ...times('end'));
    const booking = fieldInput('text', 'booking', event.booking ?? '', 'form-input mono props__booking');
    booking.placeholder = t('trip.booking_ref');
    editField(booking, value => edit({ booking: value.trim() || null }));
    add(t('props.booking'), booking);
  } else if (event.type === 'stay') {
    add(t('props.place'), placeField('place', event.place, place => edit({ place })));
    add(t('props.nights'), ...(locked ? [whenIn(doc, 'start')] : nightFields(doc, edit)));
  } else {
    if (event.type) add(t('props.place'), placeField('place', event.place, place => edit({ place, ...move('start', place) })));
    add(t('props.starts'), ...times('start'));
    if (doc.end && !doc.all_day) add(t('props.ends'), ...times('end'));
  }
  if (locked) {
    if (event.location) add(t('props.where'), event.location);
    return;
  }
  add(t('props.where'), whereField(event, edit));
}

// The same, read-only.
function tripText(doc, add) {
  const { event } = doc;
  const words = (options, value) => options.find(([key]) => key === value)?.[1];
  if (event.type) add(t('props.type'), words(TRIP_TYPES, event.type) ?? event.type);
  add(t('props.state'), isPlanned(event) ? t('trip.planned') : t('trip.booked'));
  if (event.by) add(t('props.by'), words(MEANS, event.by) ?? event.by);
  if (event.type === 'journey') {
    if (event.from) add(t('props.from'), event.from);
    add(t('props.leaves'), whenIn(doc, 'start'));
    if (event.to) add(t('props.to'), event.to);
    if (doc.end) add(t('props.arrives'), whenIn(doc, 'end'));
    if (event.booking) add(t('props.booking'), accEl('span', 'mono', event.booking));
  } else {
    if (event.place) add(t('props.place'), event.place);
    add(event.type === 'stay' ? t('props.nights') : t('props.when'), whenIn(doc, 'start'));
  }
  if (event.location) add(t('props.where'), event.location);
}

// ----------------------------------------------------------------------------

// An event's Type (`EVENT_TYPES`): Event, a plain one without `type`, or a
// part of a trip; a trip's type turns a plain event's card to its trip
// fields. The vault's own field, so also in a calendar Google keeps as it
// is.
export function eventTypeField(event, edit) {
  const seg = accSeg(EVENT_TYPES, event.type ?? '', type => edit({ type: type || null }));
  return keyedSeg(seg, EVENT_TYPES, 'type');
}

// A place newly named for an end that had another zone: the end moves to
// the zone the place had last, at the same wall time; nothing if it has none.
function placeMove(doc, end, place) {
  const zone = placeZone(place);
  if (!zone || doc.all_day || zone === zoneOf(doc.event, end)) return {};
  return zoneChange(doc, end, zone);
}

// A stay's nights: the first night's day and the day it ends (the morning
// after the last night), and how many. A new first night keeps them.
function nightFields(doc, edit) {
  const from = fieldInput('date', 'nights:from', localDay(doc.date), 'form-input props__date');
  const until = fieldInput('date', 'nights:until', doc.end ? localDay(doc.end) : '', 'form-input props__date');
  const nights = doc.end ? daysBetween(doc.date, doc.end) : 0;
  editField(from, value => {
    if (!value) return;
    const end = localDay(addDays(parseDate(value), Math.max(1, nights)));
    edit({ start: value, end, tz: null, 'end-tz': null });
  });
  editField(until, value => { if (value) edit({ end: value }); });
  return [from, accEl('span', 'meta', t('trip.until')), until,
    accEl('span', 'meta', nights > 0 ? t('trip.nights_count', { n: nights }) : '')];
}

// What the trip event `doc` meets on its trip's route (`routeSpot`): a
// journey its leg, where from and to, which leg and part and the night
// it runs over itself; a stay its stop, which and its nights. None for one
// the route doesn't take.
export function meetsField(doc) {
  const spot = routeSpot(doc);
  if (!spot) return null;
  if (spot.leg) {
    const { leg, index, legs, part, nights } = spot;
    const which = [`leg ${index + 1} of ${legs}`, leg.parts.length > 1 ? `part ${part + 1} of ${leg.parts.length}` : null];
    return meets(`${leg.from ?? '?'} \u2192 ${leg.to ?? '?'}`, [which.filter(Boolean).join(', '), nightsText(nights)]);
  }
  const { stop, index, stops, nights } = spot;
  return meets(stop.name ?? '', [t('meets.stop', { at: index + 1, n: stops }), nightsText(nights)]);
}

function meets(text, notes) {
  return [accEl('span', '', text), accEl('span', 'meta props__meets', notes.filter(Boolean).join(' \u00b7 '))];
}

// "the night of 07/08", "the nights of 07/09 \u2013 07/14"; none without one.
function nightsText(days) {
  if (!days.length) return null;
  if (days.length === 1) return t('trip.night_of', { day: DAY_MONTH.format(days[0]) });
  return t('trip.nights_of', { from: DAY_MONTH.format(days[0]), to: DAY_MONTH.format(days.at(-1)) });
}

// ----------------------------------------------------------------------------

// A place's input of the card (`key` its field), writing what is typed
// (`write(place)`, null for none).
function placeField(key, value, write) {
  const input = placeInput(fieldInput('text', key, value ?? '', 'form-input props__place'));
  editField(input, typed => write(typed.trim() || null));
  return input;
}

// `input` offers the places of the vault's trip events (`tripPlaces`).
const placeList = accEl('datalist');
placeList.id = 'trip-places';
document.body.append(placeList);

export function placeInput(input) {
  input.setAttribute('list', placeList.id);
  input.placeholder ||= t('trip.place');
  input.addEventListener('focus', () => {
    placeList.replaceChildren(...tripPlaces().places.map(place => {
      const option = accEl('option');
      option.value = place;
      return option;
    }));
  });
  return input;
}
