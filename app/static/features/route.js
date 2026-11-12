// A trip's route in its blueprint (panel.md#route), between its facts and
// its tab line (features/blueprint.js): the trip as its nights, home at both
// ends (`routeOf`, model/trip.js). A stop is as wide as its nights and the
// legs share at most a quarter of the strip, so it keeps to one line at any
// width, each part saying less as it narrows (header.css). A cell per night
// along a stop's foot, and a leg's line, say booked, held, missing or
// sketched. A click on a stop selects its first stay and keeps the
// Timeline to its weeks; on a leg, its parts, their cards in the panel; on
// a gap, New with what is missing filled in. While the days are open the route is the sketch,
// its parts picked to edit in a line under the strip: a stop's place,
// nights and note, a leg's ways, a stop added or removed, each written
// through `set_project`.

import { accEl } from '../core/dom.js';
import { DAY_MONTH, WEEKDAY, durationText, localDay } from '../core/format.js';
import { t } from '../core/text.js';
import { projects } from '../model/entities.js';
import { store } from '../model/state.js';
import { setProjectLocal } from '../model/structure.js';
import { WAYS, nightsOf, routeOf, tripSummary } from '../model/trip.js';
import { WAY_ICONS, iconEl } from '../ui/icons.js';
import { editField } from '../ui/field.js';
import { showMenu } from '../ui/menu.js';
import { toolButton } from '../ui/toolbar.js';
import { showWeeks } from './lenses/timeline.js';
import { openNew } from './new.js';
import { selectDocument, selectLeg } from './selection.js';

// A way within a sentence: "flight, train?".
const WAY_WORDS = {
  flight: t('way.word.flight'), train: t('way.word.train'), bus: t('way.word.bus'), ferry: t('way.word.ferry'),
  car: t('way.word.car'), way: t('way.word.way'),
};
const wayWord = way => WAY_WORDS[way] ?? String(way);

// The part of a sketch picked to edit: `{ id, type, index }`, a stop or a
// leg by its index (`routeOf`); `{ id, adding }` a stop to add there; null.
let picked = null;

// The route of the trip `ctx` (`{ id, project, writable }`, a project's
// blueprint): the strip and, while a part of its sketch is picked, its edit
// line; none for a project that is no trip. `render()` draws the blueprint
// again.
export function routeEls(ctx, render) {
  const route = routeOf(ctx.id, ctx.project);
  if (!route) return [];
  if (picked?.id !== ctx.id || !route.sketched || !ctx.writable) picked = null;
  const pick = next => {
    picked = next && !samePick(picked, next) ? { id: ctx.id, ...next } : null;
    render();
  };
  if (!route.parts.length) {
    if (!ctx.writable) return [];
    if (picked?.adding === undefined) {
      const line = accEl('div', 'route__edit');
      line.append(toolButton(t('route.add_stop'), t('route.add_stop_title'), () => pick({ adding: 0 })));
      return [line];
    }
    return [editLine(ctx, route, pick)];
  }

  const strip = accEl('div', 'route');
  strip.style.setProperty('--legs', String(route.parts.filter(part => part.type === 'leg').length));
  const selected = store.getState().selected_entity;
  strip.append(homeEl(route.home));
  for (const part of route.parts) {
    const el = part.type === 'stop' ? stopEl(part, route.sketched) : legEl(part);
    const on = route.sketched
      ? picked && picked.type === part.type && picked.index === part.index
      : [...part.stays ?? [], ...part.parts ?? []].some(doc => doc.doc_id === selected);
    if (on) el.setAttribute('aria-selected', 'true');
    el.addEventListener('click', () => {
      if (route.sketched) {
        if (ctx.writable) pick({ type: part.type, index: part.index });
      } else {
        openPart(part);
      }
    });
    strip.append(el);
  }
  strip.append(homeEl(route.home));
  return picked ? [strip, editLine(ctx, route, pick)] : [strip];
}

const samePick = (a, b) => a && a.type === b.type && a.index === b.index && a.adding === b.adding;

// ----------------------------------------------------------------------------
function homeEl(home) {
  const el = accEl('span', 'route__home');
  el.title = home ? t('route.home_named', { home }) : t('route.home');
  const box = accEl('span', 'route__box');
  box.append(iconEl('#i-home'));
  el.append(box);
  return el;
}

function partEl(className, part, weight, title) {
  const el = accEl('button', className);
  el.type = 'button';
  el.title = title;
  if (weight) el.style.setProperty('--n', String(weight));
  if (part.state !== 'booked') el.dataset.state = part.state;
  return el;
}

// A cell per night: booked, held, missing, sketched (a maybe faded).
function nightsEl(nights) {
  const el = accEl('span', 'route__nights');
  el.append(...nights.map(night => accEl('i', {
    booked: '', held: 'is-held', missing: 'is-gap', idea: night.maybe ? 'is-idea is-maybe' : 'is-idea',
  }[night.state])));
  return el;
}

const dayText = day => `${WEEKDAY.format(day)} ${DAY_MONTH.format(day)}`;

// A stop: its place, its nights and their cells; under it its days, or
// while sketched its note.
function stopEl(stop, sketched) {
  const n = stop.nights.length;
  const name = stop.name || (sketched ? t('route.no_place') : t('route.no_stay'));
  let count;
  let when;
  let title;
  if (sketched) {
    const c = stop.count;
    count = !c ? t('route.nights_unknown') : c.min !== c.max ? `${c.min}\u2013${c.max}`
      : c.about ? `~${t('trip.nights', { n: c.min })}` : t('trip.nights', { n: c.min });
    when = [stop.note ?? ''];
    title = [name, stop.note, c ? count : null].filter(Boolean).join(', ');
  } else {
    const covered = stop.nights.filter(night => night.state !== 'missing').length;
    count = stop.state === 'partial' ? t('count.of_total', { at: covered, n }) : t('trip.nights', { n });
    const last = stop.to;
    const long = n === 1 ? dayText(stop.from) : `${dayText(stop.from)} \u2013 ${dayText(last)}`;
    const short = n === 1 ? String(stop.from.getDate()) : `${stop.from.getDate()}\u2013${last.getDate()}`;
    when = [accEl('span', 'route__long', long), accEl('span', 'route__short', short)];
    const state = {
      booked: t('route.nights_booked', { n }), held: t('route.nights_held', { n }),
      partial: t('route.nights_partial', { at: covered, n }), missing: t('route.nights_missing', { n }),
    }[stop.state];
    title = [name, stop.stays.map(doc => doc.title).join(', '), long, state].filter(Boolean).join(' \u00b7 ');
  }
  const weight = sketched ? (stop.count ? (stop.count.min + stop.count.max) / 2 : 1) : n;
  const el = partEl('route__stop', stop, weight, title);
  const box = accEl('span', 'route__box');
  box.append(accEl('span', 'route__name', name), accEl('span', 'meta route__count', count), nightsEl(stop.nights));
  const whenEl = accEl('span', 'meta route__when');
  whenEl.append(...when);
  el.append(box, whenEl);
  return el;
}

// A leg: an icon per part, a dot where they meet, a cell per night it runs
// over; under it what it is: held, not booked; booked, its vias, how long
// it takes or its way; sketched, its ways guessed; missing, none.
function legEl(leg) {
  const line = accEl('span', 'route__line');
  let text;
  let title;
  if (leg.state === 'missing') {
    line.append(iconEl('#i-warning'));
    text = t('route.no_way');
    const places = { from: leg.from ?? t('trip.somewhere'), to: leg.to ?? t('trip.somewhere') };
    title = leg.day ? t('route.no_way_on', { ...places, day: dayText(leg.day) }) : t('route.no_way_title', places);
  } else {
    leg.ways.forEach((way, i) => {
      if (i) line.append(accEl('i', 'route__via'));
      line.append(iconEl(WAY_ICONS[way] ?? '#i-calendar'));
    });
    if (leg.state === 'idea') {
      text = leg.ways.length ? `${leg.ways.map(wayWord).join(', ')}?` : `${wayWord('way')}?`;
      title = [`${leg.from ?? t('trip.somewhere_first')} \u2192 ${leg.to ?? t('trip.somewhere')}`,
        leg.ways.length ? leg.ways.map(wayWord).join(', ') : t('route.no_way_yet'),
        leg.nights.length ? t('trip.over_night') : null].filter(Boolean).join(' \u00b7 ');
    } else {
      const vias = leg.parts.slice(0, -1).map(doc => doc.event.to).filter(Boolean);
      const [first, last] = [leg.parts[0], leg.parts.at(-1)];
      text = leg.state === 'held' ? t('route.not_booked')
        : vias.length ? t('route.via', { places: vias.join(', ') })
          : !first.all_day && last.end ? durationText(last.end - first.date) : wayWord(first.event.way ?? 'way');
      title = leg.parts.map(doc => `${doc.title} \u00b7 ${tripSummary(doc)}`).join('\n');
    }
  }
  if (leg.nights.length) line.append(nightsEl(leg.nights));
  const el = partEl('route__leg', leg, leg.nights.length, title);
  el.append(line, accEl('span', 'meta route__when', text));
  return el;
}

// A click on a part of a dated route: a gap opens New with what it lacks,
// a stop selects its first stay and keeps the Timeline to its days, a leg
// of parts selects them all (`selectLeg`), one of one part that one.
function openPart(part) {
  if (part.type === 'leg' && part.state === 'missing') {
    const day = localDay(part.day);
    openNew('event', { eventKind: 'transport', from: part.from ?? '', to: part.to ?? '', start: day, end: day, startTime: '', endTime: '' });
  } else if (part.type === 'leg' && part.parts.length > 1) {
    selectLeg(part.parts.map(doc => doc.doc_id));
  } else if (part.type === 'leg') {
    selectDocument(part.parts[0].doc_id);
  } else if (!part.stays.length) {
    openNew('event', { eventKind: 'stay', place: part.place ?? '', start: localDay(part.from), end: localDay(part.to) });
  } else {
    selectDocument(part.stays[0].doc_id);
    showWeeks(part.from, part.to);
  }
}

// ----------------------------------------------------------------------------
// The edit line of the part of the sketch picked: a stop's place, nights
// and note, each written by itself, with Add a stop after it and Remove; a
// leg's ways, each a menu, one more for a via, and over night for the way
// to a stop; the place of a stop to add, which Enter adds.
function editLine({ id }, route, pick) {
  const line = accEl('div', 'route__edit');
  const stops = route.parts.filter(part => part.type === 'stop');
  if (picked.adding !== undefined) {
    const place = editInput('', t('props.place'), 'route:new');
    editField(place, typed => {
      if (!typed.trim()) return;
      const at = picked.adding;
      writeSketch(id, sketch => {
        const all = sketch.stops ?? [];
        return { ...sketch, stops: [...all.slice(0, at), { place: typed.trim() }, ...all.slice(at)] };
      });
      picked = { id, type: 'stop', index: at };
      place.blur();
    });
    line.append(accEl('span', 'label', t('route.new_stop')), place, toolButton(t('field.cancel'), t('route.add_no_stop'), () => pick(null)));
    // once, as the line opens: drawn anew it would take the focus back
    if (!picked.focused) {
      picked.focused = true;
      queueMicrotask(() => place.focus());
    }
    return line;
  }
  const { type, index } = picked;
  const part = route.parts.find(p => p.type === type && p.index === index);
  if (!part) return line;
  if (type === 'stop') {
    const stop = part;
    const setStop = (key, value) => writeSketch(id, sketch => ({
      ...sketch, stops: (sketch.stops ?? []).map((s, i) => (i === index ? withKey(s, key, value) : s)),
    }));
    const place = editInput(stop.place ?? '', t('props.place'), 'route:place');
    editField(place, typed => setStop('place', typed.trim() || null));
    const nights = editInput(stop.count ? countValue(stop) : '', t('props.nights'), 'route:nights');
    nights.title = t('when.nights_title');
    nights.classList.add('route__nights-input');
    editField(nights, typed => {
      const text = typed.trim();
      if (!text) setStop('nights', null);
      else if (nightsOf(text)) setStop('nights', /^\d+$/.test(text) ? Number(text) : text);
    });
    const note = editInput(stop.note ?? '', t('when.note'), 'route:note');
    editField(note, typed => setStop('note', typed.trim() || null));
    line.append(accEl('span', 'label', t('route.stop')), place, nights, note,
      toolButton(t('route.add_stop'), t('route.add_stop_after'), () => pick({ adding: index + 1 })),
      toolButton(t('id_documents.remove'), t('route.remove_stop'), () => {
        writeSketch(id, sketch => ({ ...sketch, stops: (sketch.stops ?? []).filter((_, i) => i !== index) }));
        pick(null);
      }));
    return line;
  }
  const leg = part;
  const back = index === stops.length;
  const setWays = ways => writeSketch(id, sketch => {
    const by = ways.length > 1 ? ways : ways[0] ?? null;
    if (back) return withKey(sketch, 'back', withKey(sketch.back ?? {}, 'by', by));
    return { ...sketch, stops: (sketch.stops ?? []).map((s, i) => (i === index ? withKey(s, 'by', by) : s)) };
  });
  const wayItems = (current, onPick) => WAYS.map(([value, label]) => ({
    label, icon: WAY_ICONS[value], checked: value === current, run: () => onPick(value),
  }));
  line.append(accEl('span', 'label', back ? t('route.way_home') : t('props.way')));
  leg.ways.forEach((way, i) => {
    const btn = toolButton(WAYS.find(([value]) => value === way)?.[1] ?? String(way), t('route.another_way'), () => showMenu(btn, [
      ...wayItems(way, value => setWays(leg.ways.map((w, j) => (j === i ? value : w)))),
      { label: t('id_documents.remove'), separator: true, run: () => setWays(leg.ways.filter((_, j) => j !== i)) },
    ], { head: t('props.way') }));
    btn.prepend(iconEl(WAY_ICONS[way] ?? '#i-calendar'));
    line.append(btn);
  });
  const add = toolButton(leg.ways.length ? t('route.via_button') : t('route.add_way'),
    leg.ways.length ? t('route.via_title') : t('route.add_way_title'),
    () => showMenu(add, wayItems(null, value => setWays([...leg.ways, value])), { head: t('props.way') }));
  add.prepend(iconEl('#i-add'));
  line.append(add);
  if (!back) {
    const overnight = toolButton(t('route.over_night'), t('route.over_night_title'), () => {
      writeSketch(id, sketch => ({
        ...sketch, stops: (sketch.stops ?? []).map((s, i) => (i === index ? withKey(s, 'overnight', s.overnight ? null : true) : s)),
      }));
    });
    overnight.setAttribute('aria-pressed', String(leg.nights.length > 0));
    line.append(overnight);
  }
  return line;
}

// A sketched stop's nights as typed: `6`, `~6`, `2-3`.
function countValue({ count }) {
  if (count.min !== count.max) return `${count.min}-${count.max}`;
  return `${count.about ? '~' : ''}${count.min}`;
}

// `object` with `key` set to `value`; null removes it.
function withKey(object, key, value) {
  const next = { ...object };
  if (value === null || value === undefined) delete next[key];
  else next[key] = value;
  return next;
}

// Write the sketch of the trip `id` as `change(sketch)` makes it, from its
// block as the host last pushed it (an edit line outlives one write); a
// sketch left empty goes, a way home without a way too.
function writeSketch(id, change) {
  const project = projects.get(id)?.project;
  if (!project) return;
  let sketch = change(project.sketch ?? {});
  if (!sketch.stops?.length) sketch = withKey(sketch, 'stops', null);
  if (sketch.back && !Object.keys(sketch.back).length) sketch = withKey(sketch, 'back', null);
  setProjectLocal(id, withKey(project, 'sketch', Object.keys(sketch).length ? sketch : null));
}

function editInput(value, label, key) {
  const input = accEl('input', 'form-input bp__input');
  input.type = 'text';
  input.value = value;
  input.placeholder = label;
  input.dataset.focusKey = key;
  input.setAttribute('aria-label', label);
  return input;
}
