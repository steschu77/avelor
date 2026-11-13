// A trip's event on its card (panel.md#trip-fields): what part of the trip
// it is and held or booked, then its own fields by that -- a transport's
// way, where it leaves from and arrives, both ends' times each in its own
// place's time zone with how long it takes, and its booking; a stay's place
// and nights; an activity's place and times -- and where it is. While the
// host can write it, each field writes by itself through `edit_entities`
// (`editEntityLocal`, an `{event: {...}}` change); a time goes with its zone.
// The place inputs offer the places of the vault's trip events, and a
// place named turns to the zone it had last (`placeZone`). Also New's
// place inputs and zone menu (features/new.js). After its calendar, what it
// meets on its trip's route (`meetsField`).

import { accEl } from '../../core/dom.js';
import { DATE_FORMAT, durationText, isoInstant, localDay, parseDate } from '../../core/format.js';
import { daysBetween, timeFormatIn, zonedInstant, zonedTime } from '../../core/format.js';
import { t } from '../../core/text.js';
import { editEntityLocal } from '../../model/structure.js';
import {
  LOCAL_ZONE, STATES, TRIP_KINDS, WAYS, endZone, isHeld, issueText, nightsText, placeZone, routeSpot, startZone, tripPlaces, zoneIssuesOf,
  zoneName,
} from '../../model/trip.js';
import { editField, fieldInput } from '../../ui/field.js';
import { WAY_ICONS } from '../../ui/icons.js';
import { showMenu } from '../../ui/menu.js';
import { accSeg } from '../../ui/segmented.js';

const DAY = 864e5;

// ----------------------------------------------------------------------------
// The trip fields of the event `doc` into the card's grid (`add(label,
// ...content)`, features/panel/details.js); as text while the host can't
// write it.
export function tripFields(doc, add) {
  if (!doc.writable) return tripText(doc, add);
  const { doc_id: id, event } = doc;
  const edit = change => editEntityLocal(id, { event: change });
  add(t('props.kind'), keyed(accSeg(TRIP_KINDS, event.kind ?? null, kind => edit({ kind })), TRIP_KINDS, 'kind'));
  const state = isHeld(event) ? 'tentative' : 'confirmed';
  add(t('props.state'), keyed(accSeg(STATES, state, status => edit({ status })), STATES, 'state'));
  if (event.kind === 'transport') {
    const ways = WAYS.map(([value, label]) => [value, label, { icon: WAY_ICONS[value] }]);
    add(t('props.way'), keyed(accSeg(ways, event.way ?? null, way => edit({ way })), WAYS, 'way'));
    add(t('props.from'), placeField('from', event.from, from => edit({ from, ...placeMove(doc, 'start', from) })));
    add(t('props.leaves'), ...timeFields(doc, 'start', edit));
    add(t('props.to'), placeField('to', event.to, to => edit({ to, ...placeMove(doc, 'end', to) })));
    add(t('props.arrives'), ...timeFields(doc, 'end', edit));
    const booking = fieldInput('text', 'booking', event.booking ?? '', 'form-input mono props__booking');
    booking.placeholder = t('trip.booking_ref');
    editField(booking, value => edit({ booking: value.trim() || null }));
    add(t('props.booking'), booking);
  } else if (event.kind === 'stay') {
    add(t('props.place'), placeField('place', event.place, place => edit({ place })));
    add(t('props.nights'), ...nightFields(doc, edit));
  } else {
    if (event.kind) add(t('props.place'), placeField('place', event.place, place => edit({ place, ...placeMove(doc, 'start', place) })));
    add(t('props.starts'), ...timeFields(doc, 'start', edit));
    if (doc.end && !doc.all_day) add(t('props.ends'), ...timeFields(doc, 'end', edit));
  }
  const where = fieldInput('text', 'where', event.location ?? '', 'form-input');
  where.placeholder = t('trip.address');
  editField(where, value => edit({ location: value.trim() || null }));
  add(t('props.where'), where);
}

// The same, read-only.
function tripText(doc, add) {
  const { event } = doc;
  const words = (options, value) => options.find(([key]) => key === value)?.[1];
  if (event.kind) add(t('props.kind'), words(TRIP_KINDS, event.kind) ?? event.kind);
  add(t('props.state'), isHeld(event) ? t('trip.held') : t('trip.booked'));
  if (event.way) add(t('props.way'), words(WAYS, event.way) ?? event.way);
  if (event.kind === 'transport') {
    if (event.from) add(t('props.from'), event.from);
    add(t('props.leaves'), whenIn(doc, 'start'));
    if (event.to) add(t('props.to'), event.to);
    if (doc.end) add(t('props.arrives'), whenIn(doc, 'end'));
    if (event.booking) add(t('props.booking'), accEl('span', 'mono', event.booking));
  } else {
    if (event.place) add(t('props.place'), event.place);
    add(event.kind === 'stay' ? t('props.nights') : t('props.when'), whenIn(doc, 'start'));
  }
  if (event.location) add(t('props.where'), event.location);
}

// "07/08/2027, 6:05 PM \u00b7 Chicago time": an end in its own zone.
function whenIn(doc, end) {
  const date = end === 'start' ? doc.date : doc.end;
  if (doc.all_day) {
    const nights = doc.end ? daysBetween(doc.date, doc.end) : 0;
    return end === 'start' && doc.event.kind === 'stay' && nights > 0
      ? `${DATE_FORMAT.format(doc.date)} \u2013 ${DATE_FORMAT.format(doc.end)} \u00b7 ${t('trip.nights_count', { n: nights })}`
      : DATE_FORMAT.format(end === 'end' ? new Date(date - DAY) : date);
  }
  const zone = zoneOf(doc.event, end);
  return `${DATE_FORMAT.format(parseDate(zonedTime(date, zone).day))}, ${timeFormatIn(zone).format(date)} \u00b7 ${zoneLabel(zone)}`;
}

// ----------------------------------------------------------------------------
// The zone an end is shown and written in: its own, else this PC's.
function zoneOf(event, end) {
  return (end === 'start' ? startZone(event) : endZone(event)) ?? LOCAL_ZONE;
}

// "Chicago time": the zone by its city. The place's name would hide a zone
// that isn't the place's.
function zoneLabel(zone) {
  return t('zone.time', { zone: zoneName(zone) });
}

// An end of the event `doc` (`start` or `end`, Leaves or Arrives): its day
// and time in its zone, the zone (a menu), and for the end how long it
// takes. A day-only event (a held way) has no zone; a time typed into its
// start makes it a timed one of an hour, clearing that time a day-only one
// again. A new start keeps how long the event takes.
function timeFields(doc, end, edit) {
  const { event } = doc;
  const zone = zoneOf(event, end);
  const date = end === 'start' ? doc.date : doc.end;
  const at = doc.all_day ? { day: localDay(end === 'end' && date ? new Date(date - DAY) : date ?? doc.date), time: '' }
    : zonedTime(date ?? doc.date, zone);
  const day = fieldInput('date', `${end}:day`, at.day, 'form-input props__date');
  const time = fieldInput('time', `${end}:time`, at.time, 'form-input props__time');
  const write = () => {
    if (!day.value) return;
    edit(end === 'start' ? startChange(doc, day.value, time.value, zone) : endChange(doc, day.value, time.value, zone));
  };
  editField(day, write);
  editField(time, write);
  if (doc.all_day && end === 'end') time.disabled = true;
  const parts = [day, time];
  if (!doc.all_day) {
    const issue = zoneIssuesOf(doc)[end];
    parts.push(zoneButton(`${end}:zone`, zone, next => {
      if (next !== zone) edit(zoneChange(doc, end, next));
    }, issue ? issueText(issue) : null));
    if (end === 'end' && doc.end) parts.push(accEl('span', 'meta', durationText(doc.end - doc.date)));
  }
  return parts;
}

// The change a new start makes: day-only while it has no time; else the
// instant in `zone`, the end as far after it as before.
function startChange(doc, day, time, zone) {
  if (!time) {
    const days = doc.all_day && doc.end ? Math.max(1, daysBetween(doc.date, doc.end)) : 1;
    return { start: day, end: localDay(new Date(parseDate(day).getTime() + days * DAY)), timeZone: null, 'end-time-zone': null };
  }
  const start = zonedInstant(day, time, zone);
  const span = !doc.all_day && doc.end ? doc.end - doc.date : 3600e3;
  return { start: isoInstant(start), end: isoInstant(new Date(start.getTime() + span)), timeZone: zone };
}

// The change a new end makes: day-only, the day after (an all-day end is
// excluded); else the instant in `zone`, its own if it isn't the start's.
function endChange(doc, day, time, zone) {
  if (doc.all_day || !time) return { end: localDay(new Date(parseDate(day).getTime() + DAY)) };
  const own = zone === startZone(doc.event) ? null : zone;
  return { end: isoInstant(zonedInstant(day, time, zone)), 'end-time-zone': own };
}

// The change another zone makes for an end: the same wall time there. A
// transport's arrival keeps the zone it had, so its end has its own unless
// that is the start's; the other events' ends move with their start.
function zoneChange(doc, end, zone) {
  const { event } = doc;
  const wall = (instant, from) => {
    const at = zonedTime(instant, from);
    return isoInstant(zonedInstant(at.day, at.time, zone));
  };
  if (end === 'end') {
    return { end: wall(doc.end, zoneOf(event, 'end')), 'end-time-zone': zone === startZone(event) ? null : zone };
  }
  const change = { start: wall(doc.date, zoneOf(event, 'start')), timeZone: zone };
  if (!doc.end) return change;
  if (event.kind === 'transport') {
    const endsIn = zoneOf(event, 'end');
    change['end-time-zone'] = endsIn === zone ? null : endsIn;
  } else {
    change.end = wall(doc.end, zoneOf(event, 'end'));
    change['end-time-zone'] = null;
  }
  return change;
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
    const end = localDay(new Date(parseDate(value).getTime() + Math.max(1, nights) * DAY));
    edit({ start: value, end, timeZone: null, 'end-time-zone': null });
  });
  editField(until, value => { if (value) edit({ end: value }); });
  return [from, accEl('span', 'meta', t('trip.until')), until,
    accEl('span', 'meta', nights > 0 ? t('trip.nights_count', { n: nights }) : '')];
}

// What the trip event `doc` meets on its trip's route (`routeSpot`): a
// transport its leg, where from and to, which leg and part and the night
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

// ----------------------------------------------------------------------------
// The segmented control `seg` of `options`, its buttons found again once
// the block is drawn anew.
function keyed(seg, options, key) {
  [...seg.children].forEach((option, i) => { option.dataset.focusKey = `${key}:${options[i][0]}`; });
  return seg;
}

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

// A button naming `zone` that opens the zones' menu (`zoneMenu`); `key`
// finds it again once the block is drawn anew. With `warning`, the zone
// isn't the one the trip arrived in (`zoneIssuesOf`), marked and said.
export function zoneButton(key, zone, onPick, warning = null) {
  const button = accEl('button', 'tool label props__zone', zoneLabel(zone));
  button.type = 'button';
  button.title = [t('zone.another', { zone: zone.replace(/_/g, ' ') }), warning].filter(Boolean).join('\n');
  button.classList.toggle('tool--danger', Boolean(warning));
  if (key) button.dataset.focusKey = key;
  button.addEventListener('click', () => zoneMenu(button, zone, onPick));
  return button;
}

// The time zones at `anchor`, `current` ticked: this PC's and those the
// vault's trip events use, then every other; a filter finds one.
export function zoneMenu(anchor, current, onPick) {
  const first = new Set([LOCAL_ZONE, ...tripPlaces().zones]);
  const all = Intl.supportedValuesOf?.('timeZone') ?? [];
  const item = zone => ({ label: zone.replace(/_/g, ' '), checked: zone === current, run: () => onPick(zone) });
  showMenu(anchor, [
    ...[...first].sort().map(item),
    { head: t('zone.all'), separator: true },
    ...all.filter(zone => !first.has(zone)).map(item),
  ], { head: t('zone.head'), filter: { placeholder: t('zone.filter') } });
}
