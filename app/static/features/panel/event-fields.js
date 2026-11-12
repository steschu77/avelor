// An event's times and where on its card, a trip's event's
// (features/panel/trip-fields.js) and a plain one's alike: an end's day and
// time in its zone, with the zones' menu (`timeFields`), the change a new
// start, end or zone makes, either end as text in its zone (`whenIn`), and
// Where (`whereField`). Also New's zone menu (features/new.js).

import { accEl } from '../../core/dom.js';
import { DATE_FORMAT, durationText, isoInstant, localDay, parseDate } from '../../core/format.js';
import { addDays, daysBetween, timeFormatIn, zonedInstant, zonedTime } from '../../core/format.js';
import { t } from '../../core/text.js';
import { LOCAL_ZONE, endZone, startZone, tripPlaces, zoneName } from '../../model/trip.js';
import { editField, fieldInput } from '../../ui/field.js';
import { showMenu } from '../../ui/menu.js';

// ----------------------------------------------------------------------------
// "07/08/2027, 6:05 PM \u00b7 Chicago time": an end in its own zone.
export function whenIn(doc, end) {
  const date = end === 'start' ? doc.date : doc.end;
  if (doc.all_day) {
    const nights = doc.end ? daysBetween(doc.date, doc.end) : 0;
    return end === 'start' && doc.event.type === 'stay' && nights > 0
      ? `${DATE_FORMAT.format(doc.date)} \u2013 ${DATE_FORMAT.format(doc.end)} \u00b7 ${t('trip.nights_count', { n: nights })}`
      : DATE_FORMAT.format(end === 'end' ? addDays(date, -1) : date);
  }
  const zone = zoneOf(doc.event, end);
  return `${DATE_FORMAT.format(parseDate(zonedTime(date, zone).day))}, ${timeFormatIn(zone).format(date)} \u00b7 ${zoneLabel(zone)}`;
}

// ----------------------------------------------------------------------------
// The zone an end is shown and written in: its own, else this PC's.
export function zoneOf(event, end) {
  return (end === 'start' ? startZone(event) : endZone(event)) ?? LOCAL_ZONE;
}

// "Chicago time": the zone by its city. The place's name would hide a zone
// that isn't the place's.
function zoneLabel(zone) {
  return t('zone.time', { zone: zoneName(zone) });
}

// ----------------------------------------------------------------------------
// An end of the event `doc` (`start` or `end`, Leaves or Arrives): its day
// and time in its zone, the zone (a menu), and for the end how long it
// takes. A day-only event (a planned journey) has no zone; a time typed into its
// start makes it a timed one of an hour, clearing that time a day-only one
// again. A new start keeps how long the event takes.
export function timeFields(doc, end, edit) {
  const { event } = doc;
  const zone = zoneOf(event, end);
  const date = end === 'start' ? doc.date : doc.end;
  const at = doc.all_day ? { day: localDay(end === 'end' && date ? addDays(date, -1) : date ?? doc.date), time: '' }
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
    parts.push(zoneButton(`${end}:zone`, zone, next => {
      if (next !== zone) edit(zoneChange(doc, end, next));
    }));
    if (end === 'end' && doc.end) parts.push(accEl('span', 'meta', durationText(doc.end - doc.date)));
  }
  return parts;
}

// The change a new start makes: day-only while it has no time; else the
// instant in `zone`, the end as far after it as before.
function startChange(doc, day, time, zone) {
  if (!time) {
    const days = doc.all_day && doc.end ? Math.max(1, daysBetween(doc.date, doc.end)) : 1;
    return { start: day, end: localDay(addDays(parseDate(day), days)), tz: null, 'end-tz': null };
  }
  const start = zonedInstant(day, time, zone);
  const span = !doc.all_day && doc.end ? doc.end - doc.date : 3600e3;
  return { start: isoInstant(start), end: isoInstant(new Date(start.getTime() + span)), tz: zone };
}

// The change a new end makes: day-only, the day after (an all-day end is
// excluded); else the instant in `zone`, its own if it isn't the start's.
function endChange(doc, day, time, zone) {
  if (doc.all_day || !time) return { end: localDay(addDays(parseDate(day), 1)) };
  const own = zone === startZone(doc.event) ? null : zone;
  return { end: isoInstant(zonedInstant(day, time, zone)), 'end-tz': own };
}

// The change another zone makes for an end: the same wall time there. A
// journey's arrival keeps the zone it had, so its end has its own unless
// that is the start's; the other events' ends move with their start.
export function zoneChange(doc, end, zone) {
  const { event } = doc;
  const wall = (instant, from) => {
    const at = zonedTime(instant, from);
    return isoInstant(zonedInstant(at.day, at.time, zone));
  };
  if (end === 'end') {
    return { end: wall(doc.end, zoneOf(event, 'end')), 'end-tz': zone === startZone(event) ? null : zone };
  }
  const change = { start: wall(doc.date, zoneOf(event, 'start')), tz: zone };
  if (!doc.end) return change;
  if (event.type === 'journey') {
    const endsIn = zoneOf(event, 'end');
    change['end-tz'] = endsIn === zone ? null : endsIn;
  } else {
    change.end = wall(doc.end, zoneOf(event, 'end'));
    change['end-tz'] = null;
  }
  return change;
}

// ----------------------------------------------------------------------------
// Where the event is, its `location` typed in, `edit` writing it (null for
// none).
export function whereField(event, edit) {
  const where = fieldInput('text', 'where', event.location ?? '', 'form-input');
  where.placeholder = t('trip.address');
  editField(where, value => edit({ location: value.trim() || null }));
  return where;
}

// ----------------------------------------------------------------------------
// A button naming `zone` that opens the zones' menu (`zoneMenu`); `key`
// finds it again once the block is drawn anew.
export function zoneButton(key, zone, onPick) {
  const button = accEl('button', 'tool label props__zone', zoneLabel(zone));
  button.type = 'button';
  button.title = t('zone.another', { zone: zone.replace(/_/g, ' ') });
  if (key) button.dataset.focusKey = key;
  button.addEventListener('click', () => zoneMenu(button, zone, onPick));
  return button;
}

// The time zones at `anchor`, `current` ticked: this PC's and those the
// vault's trip events use, then every other; a filter finds one.
function zoneMenu(anchor, current, onPick) {
  const first = new Set([LOCAL_ZONE, ...tripPlaces().zones]);
  const all = Intl.supportedValuesOf?.('timeZone') ?? [];
  const item = zone => ({ label: zone.replace(/_/g, ' '), checked: zone === current, run: () => onPick(zone) });
  showMenu(anchor, [
    ...[...first].sort().map(item),
    { head: t('zone.all'), separator: true },
    ...all.filter(zone => !first.has(zone)).map(item),
  ], { head: t('zone.head'), filter: { placeholder: t('zone.filter') } });
}
