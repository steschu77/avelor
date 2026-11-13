// When (panel.md#when), the first list of a trip's tab line while its days
// are open (features/blueprint.js): the sketch's window as days, a lane per
// thing that limits them (`whenOf`, model/when.js); under them the windows
// that fit, one chosen, its band shading the lanes; then each window's
// days, why and cost, and Set the dates from the one chosen (`openSetDates`,
// features/set-dates.js). Edited in place, each through `set_project`: the
// window and the length in an edit line under the head; + Lane adds a
// calendar's, an event's or days typed in; a lane's name opens its menu
// (what it comes from, how its days count, Rename, Add days, Leave out),
// a day typed in its own.

import { accEl } from '../core/dom.js';
import { pressable } from '../core/dom.js';
import { DAY_MONTH, DAY_SHORT_MONTH, WEEKDAY, addDays, daysBetween, localDay } from '../core/format.js';
import { t } from '../core/text.js';
import { projectCalendar } from '../model/blueprint.js';
import { writableCalendar } from '../model/calendar.js';
import { projects } from '../model/entities.js';
import { setProjectLocal } from '../model/structure.js';
import { setDatesProposal, nightsOf } from '../model/trip.js';
import { COUNTS } from '../model/when.js';
import { editField } from '../ui/field.js';
import { iconEl } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import { toolButton } from '../ui/toolbar.js';
import { calendarMenu } from './facts.js';
import { openSetDates } from './set-dates.js';
import { eventMenu } from './gaps.js';
import { editInput, withKey, writeSketch } from './route.js';
import { selectDocument } from './selection.js';

// The window chosen: `{ id, letter }`, the first one's while none is.
let chosen = null;
// What the edit line edits: `{ id, what }`, `what` the `window`, a new
// lane typed in (`new`), a lane's `name` or its days (`span`, a lane's
// `key` and the span's `index`, null for one to add); null for none.
// `draft`: what is typed into a lane or days to add, kept while the line
// is drawn anew; `focused` once its first input had the focus.
let editing = null;

const SPAN_CLASS = { cant: 'is-block', worse: 'is-soft', reason: 'is-mark' };
const dayText = day => `${WEEKDAY.format(day)} ${DAY_MONTH.format(day)}`;
const rangeText = (from, to) => (daysBetween(from, to) ? `${dayText(from)} \u2013 ${dayText(to)}` : dayText(from));

// When of the trip `ctx` (`{ id, project, writable }`), `when` its
// `whenOf`; `render()` draws the blueprint again.
export function whenEls(ctx, when, render) {
  if (editing?.id !== ctx.id || !ctx.writable) editing = null;
  const edit = next => {
    editing = next && { id: ctx.id, ...next };
    render();
  };
  const head = accEl('div', 'when__head');
  head.append(accEl('span', 'meta', !when.from ? t('when.no_window')
    : [rangeText(when.from, when.to), when.nights ? nightsRange(when.nights) : t('when.no_length_yet')].join(' \u00b7 ')));
  if (ctx.writable) {
    const change = toolButton(when.from ? t('when.edit') : t('when.add_window'), t('when.window_title'),
      () => edit(editing?.what === 'window' ? null : { what: 'window' }));
    change.prepend(iconEl('#i-edit'));
    head.append(change);
    if (when.from) {
      const add = toolButton(t('when.lane'), t('when.lane_title'),
        btn => addLaneMenu(btn, ctx, when, edit));
      add.prepend(iconEl('#i-add'));
      head.append(add);
    }
  }
  const parts = [head];
  if (editing) parts.push(editLine(ctx, when, edit));
  if (!when.from) return parts;
  const fit = when.fits.find(f => chosen?.id === ctx.id && f.letter === chosen.letter) ?? when.fits[0] ?? null;
  const choose = f => {
    chosen = { id: ctx.id, letter: f.letter };
    render();
  };
  parts.push(gridEl(ctx, when, fit, choose, edit));
  if (!when.nights) {
    parts.push(accEl('p', 'note bp__none', t('when.no_length')));
  } else if (!fit) {
    parts.push(accEl('p', 'note bp__none', t('when.no_fit', { nights: nightsRange(when.nights) })));
  } else {
    parts.push(choicesEl(when, fit, choose), footEl(ctx, fit));
  }
  return parts;
}

function nightsRange({ min, max, about }) {
  if (min !== max) return t('when.nights_range', { min, max });
  return about ? t('when.about_nights', { n: min }) : t('trip.nights', { n: min });
}

// ----------------------------------------------------------------------------
// The window's days in columns, a Monday's date over its week; a row per
// lane, its spans where they lie; the windows that fit in the last row.
function gridEl(ctx, when, fit, choose, edit) {
  const grid = accEl('div', 'when');
  grid.style.setProperty('--days', String(when.days));
  // repeat() takes no 0
  grid.style.setProperty('--lanes', String(Math.max(1, when.lanes.length)));
  const col = day => 2 + daysBetween(when.from, day);
  let month = null;
  for (let i = 0; i < when.days; i++) {
    const day = addDays(when.from, i);
    if (day.getDay() !== 1) continue;
    const label = accEl('span', 'meta when__mon', day.getMonth() !== month ? DAY_SHORT_MONTH.format(day) : String(day.getDate()));
    label.style.gridColumn = `${col(day)} / span 7`;
    const week = accEl('span', 'when__week');
    week.style.gridColumn = String(col(day));
    month = day.getMonth();
    grid.append(week, label);
  }
  when.lanes.forEach((lane, i) => {
    const row = String(i + 2);
    const opens = ctx.writable || lane.source;
    const name = accEl(opens ? 'button' : 'span', 'when__name');
    name.append(iconEl(lane.icon), lane.name);
    name.title = laneTitle(lane);
    if (opens) {
      name.type = 'button';
      name.dataset.lane = lane.key;
      name.addEventListener('click', () => (ctx.writable ? laneMenu(name, ctx, lane, edit) : selectDocument(lane.source.doc_id)));
    }
    const line = accEl('span', 'when__lane');
    for (const el of [name, line]) el.style.gridRow = row;
    grid.append(name, line, ...lane.spans.map(span => spanEl(ctx, lane, span, row, col, edit)));
  });
  const label = accEl('span', 'when__name label', t('when.fits'));
  grid.append(label);
  if (fit) {
    const band = accEl('span', 'when__band');
    band.style.gridColumn = `${col(fit.from)} / ${col(fit.to) + 1}`;
    grid.append(band);
  }
  for (const f of when.fits) {
    const btn = accEl('button', 'when__fit', f.letter);
    btn.type = 'button';
    btn.style.gridColumn = `${col(f.from)} / ${col(f.to) + 1}`;
    btn.title = `${f.letter} \u00b7 ${rangeText(f.from, f.to)}, ${t('trip.nights', { n: f.nights })}`;
    btn.setAttribute('aria-selected', String(f === fit));
    btn.append(accEl('span', 'meta', t('trip.nights', { n: f.nights })));
    btn.addEventListener('click', () => choose(f));
    grid.append(btn);
  }
  return grid;
}

function laneTitle(lane) {
  const title = lane.source?.title || t('untitled');
  if (lane.work) return t('when.lane.work', { traveler: lane.work, title });
  if (lane.source?.kind === 'calendar') return t('when.lane.calendar', { title });
  if (lane.source) return t('when.lane.event', { title });
  return t('when.lane.typed');
}

// A span of a lane: hatched (can't), tinted (worse), a star (a reason),
// its title on it or after it, and with its days in its tooltip. An
// event's opens it; one typed in opens its menu.
function spanEl(ctx, lane, span, row, col, edit) {
  const el = accEl('span', `when__span ${SPAN_CLASS[span.counts]}`);
  el.style.gridRow = row;
  el.style.gridColumn = `${col(span.from)} / ${col(span.to) + 1}`;
  el.title = [span.title, span.detail, rangeText(span.from, span.to)].filter(Boolean).join(' \u00b7 ');
  if (span.counts === 'reason') el.append(iconEl('#i-star-filled'));
  el.append(accEl('span', '', span.title));
  if (span.doc) {
    el.classList.add('is-open');
    pressable(el).addEventListener('click', () => selectDocument(span.doc.doc_id));
  } else if (span.index !== undefined && ctx.writable) {
    el.classList.add('is-open');
    pressable(el).addEventListener('click', () => spanMenu(el, ctx, lane, span, edit));
  }
  return el;
}

// ----------------------------------------------------------------------------
// The windows that fit, one to choose: its days and nights, why (the
// reasons in it, the worse days, between what), and what it costs each
// traveler with a work calendar.
function choicesEl(when, fit, choose) {
  const list = accEl('ul', 'proposal when__choices');
  for (const f of when.fits) {
    const item = accEl('li');
    const label = accEl('label', 'proposal__row');
    const radio = accEl('input');
    radio.type = 'radio';
    radio.name = 'when';
    radio.checked = f === fit;
    radio.dataset.focusKey = `when:${f.letter}`;
    radio.addEventListener('change', () => choose(f));
    const text = accEl('span', 'proposal__text');
    const line = accEl('span');
    line.append(accEl('b', '', f.letter), ` \u00b7 ${rangeText(f.from, f.to)} \u00b7 ${t('trip.nights', { n: f.nights })}`);
    text.append(line, accEl('span', 'proposal__why', whyText(f)));
    const end = accEl('span', 'proposal__end');
    end.append(...f.cost.map(cost => accEl('span', 'meta', costText(cost))));
    label.append(radio, text, end);
    item.append(label);
    list.append(item);
  }
  return list;
}

function whyText({ reasons, worse, before, after }) {
  const around = before && after ? t('when.after_before', { before, after })
    : before ? t('when.after', { name: before }) : after ? t('when.before', { name: after }) : null;
  return [...reasons, ...worse.map(title => t('when.worse', { title })), around].filter(Boolean).join(' \u00b7 ')
    || t('when.nothing_in_the_way');
}

// "Alex: 3 workdays \u00b7 1 off, 1 remote"
function costText({ traveler, workdays, off, remote }) {
  const already = [off && t('when.days_off', { n: off }), remote && t('when.days_remote', { n: remote })].filter(Boolean).join(', ');
  const cost = t('when.cost', { traveler, workdays: t('when.workdays', { n: workdays }) });
  return already ? `${cost} \u00b7 ${already}` : cost;
}

// Set the dates from the window chosen, and what it then proposes.
function footEl(ctx, fit) {
  const days = { from: localDay(fit.from), to: localDay(fit.to) };
  const foot = accEl('div', 'when__foot');
  const btn = accEl('button', 'btn btn--primary', t('when.set_dates', { letter: fit.letter }));
  btn.type = 'button';
  btn.disabled = !ctx.writable;
  btn.title = t('when.set_dates_title', { days: rangeText(fit.from, fit.to) });
  btn.addEventListener('click', () => openSetDates(ctx.id, days));
  const { journeys, stays, off } = setDatesProposal(ctx.id, ctx.project, days, writableCalendar(projectCalendar(ctx.id)));
  const what = [stays.length && t('when.planned_stays', { n: stays.length }), journeys.length && t('when.planned_journeys', { n: journeys.length }),
    off.length && t('when.the_days_off')].filter(Boolean).join(', ');
  foot.append(btn, accEl('span', 'meta', what ? t('when.then_proposes', { what }) : t('when.nothing_before_apply')));
  return foot;
}

// ----------------------------------------------------------------------------
// + Lane: a calendar's all-day events, an event in the window or any
// other, or days typed in, a lane named first.
function addLaneMenu(anchor, { id }, when, edit) {
  showMenu(anchor, [
    { label: t('when.a_calendar'), icon: '#i-calendar', run: () => calendarMenu(anchor, null,
      calendar => calendar && addLane(id, { calendar, counts: 'cant' }), t('when.lane_from_calendar')) },
    { label: t('when.an_event'), icon: '#i-calendar', run: () => eventMenu(anchor, {
      head: t('when.lane_from_event'), near: t('when.in_window'), none: t('when.no_event_in_window'),
      from: when.from, to: addDays(when.to, 1),
      skip: doc => (doc.projects ?? []).includes(id),
      pick: doc => addLane(id, { event: doc.doc_id, counts: 'cant' }),
    }) },
    { label: t('when.days_typed_in'), icon: '#i-edit', run: () => edit({ what: 'new' }) },
  ], { head: t('when.add_lane') });
}

// A lane's menu: what it comes from, opened; how its days count (a work
// calendar's other events; a lane typed in all its days); Rename; Add days
// to one typed in; Leave out.
function laneMenu(anchor, { id }, lane, edit) {
  const items = [];
  if (lane.source) {
    items.push({ label: lane.source.kind === 'calendar' ? t('when.open_calendar') : t('when.open_event'), icon: '#i-open',
      run: () => selectDocument(lane.source.doc_id) });
  }
  items.push({ head: lane.work ? t('when.its_other_days') : t('when.its_days'), separator: items.length > 0 });
  for (const [value, label] of COUNTS) {
    if (value === 'only' && !lane.source) continue;
    items.push({
      label, checked: lane.counts === value,
      run: () => setLane(id, lane, l => ({
        ...withKey(l, 'counts', value),
        ...(l.spans && { spans: l.spans.map(span => withKey(span, 'counts', null)) }),
      })),
    });
  }
  items.push({ label: t('panel.rename'), separator: true, run: () => edit({ what: 'name', key: lane.key }) });
  if (!lane.source) items.push({ label: t('when.add_days'), run: () => edit({ what: 'span', key: lane.key, index: null }) });
  items.push({ label: t('when.leave_out'), run: () => leaveOut(id, lane) });
  showMenu(anchor, items, { head: lane.name });
}

// The menu of a span typed in: how it counts, Edit, Remove.
function spanMenu(anchor, { id }, lane, span, edit) {
  const setSpans = change => setLane(id, lane, l => ({ ...l, spans: change(l.spans ?? []) }));
  showMenu(anchor, [
    ...COUNTS.filter(([value]) => value !== 'only').map(([value, label]) => ({
      label, checked: span.counts === value,
      run: () => setSpans(spans => spans.map((s, i) => (i === span.index ? { ...s, counts: value } : s))),
    })),
    { label: t('when.edit'), separator: true, run: () => edit({ what: 'span', key: lane.key, index: span.index }) },
    { label: t('id_documents.remove'), run: () => setSpans(spans => spans.filter((_, i) => i !== span.index)) },
  ], { head: span.title });
}

// ----------------------------------------------------------------------------
// The edit line: the window's days and the length; a new lane's name; a
// lane's name; a span's days and note. Enter or leaving an input writes it.
function editLine(ctx, when, edit) {
  const line = accEl('div', 'route__edit when__edit');
  const { id } = ctx;
  const lane = editing.key ? when.lanes.find(l => l.key === editing.key) : null;
  const done = toolButton(t('id_documents.done'), t('when.close_edit'), () => edit(null));
  if (editing.what === 'window') {
    const window = ctx.project.sketch?.window ?? {};
    const setWindow = (key, value) => writeSketch(id, sketch => {
      const next = withKey(sketch.window ?? {}, key, value || null);
      return withKey(sketch, 'window', Object.keys(next).length ? next : null);
    });
    const from = dateInput(window.from, t('props.from'), 'when:from');
    editField(from, value => setWindow('from', value));
    const to = dateInput(window.to, t('props.to'), 'when:to');
    editField(to, value => setWindow('to', value));
    const nights = editInput(ctx.project.sketch?.nights == null ? '' : String(ctx.project.sketch.nights), t('props.nights'), 'when:nights');
    nights.title = t('when.nights_title');
    nights.classList.add('route__nights-input');
    editField(nights, typed => {
      const text = typed.trim();
      if (!text) writeSketch(id, sketch => withKey(sketch, 'nights', null));
      else if (nightsOf(text)) writeSketch(id, sketch => withKey(sketch, 'nights', /^\d+$/.test(text) ? Number(text) : text));
    });
    line.append(accEl('span', 'label', t('when.window')), from, to, nights, done);
    return line;
  }
  if (editing.what === 'new' || editing.what === 'name') {
    if (editing.what === 'name' && !lane) return line;
    const name = editInput(editing.draft?.name ?? lane?.name ?? '', t('docs.name'), 'when:name');
    name.addEventListener('input', () => { editing.draft = { name: name.value }; });
    editField(name, typed => {
      const text = typed.trim();
      if (editing?.what === 'new') {
        if (!text) return;
        const index = projects.get(id)?.project?.lanes?.length ?? 0;
        writeLanes(id, lanes => [...lanes, { name: text }]);
        editing = { id, what: 'span', key: `lane:${index}`, index: null };
      } else {
        setLane(id, lane, l => withKey(l, 'name', text || null));
        editing = null;
      }
      name.blur();
    });
    focusOnce(name);
    line.append(accEl('span', 'label', editing.what === 'new' ? t('when.new_lane') : t('when.lane')), name, done);
    return line;
  }
  if (!lane) return line;
  const stored = editing.index === null ? null : projects.get(id)?.project?.lanes?.[lane.index]?.spans?.[editing.index];
  const draft = editing.draft ?? {};
  const from = dateInput(stored?.from ?? draft.from, t('props.from'), 'when:span-from');
  const to = dateInput(stored?.to ?? draft.to, t('props.to'), 'when:span-to');
  const note = editInput(stored?.note ?? draft.note ?? '', t('when.note'), 'when:span-note');
  if (stored) {
    const setSpan = (key, value) => setLane(id, lane, l => ({
      ...l, spans: (l.spans ?? []).map((s, i) => (i === editing.index ? withKey(s, key, value || null) : s)),
    }));
    editField(from, value => setSpan('from', value));
    editField(to, value => setSpan('to', value));
    editField(note, typed => setSpan('note', typed.trim()));
    line.append(accEl('span', 'label', lane.name), from, to, note, done);
    return line;
  }
  // the blueprint isn't drawn again while an input has the focus
  const close = () => {
    if (line.contains(document.activeElement)) document.activeElement.blur();
    edit(null);
  };
  const add = toolButton(t('timeline.add'), t('when.add_these_days'), () => {
    if (!from.value) return;
    const span = { from: from.value, to: to.value && to.value > from.value ? to.value : from.value };
    if (note.value.trim()) span.note = note.value.trim();
    setLane(id, lane, l => ({ ...l, spans: [...(l.spans ?? []), span] }));
    close();
  });
  for (const input of [from, to, note]) {
    input.addEventListener('input', () => { editing.draft = { from: from.value, to: to.value, note: note.value }; });
    input.addEventListener('keydown', e => {
      if (e.key === 'Enter') add.click();
      else if (e.key === 'Escape') close();
    });
  }
  focusOnce(from);
  line.append(accEl('span', 'label', lane.name), from, to, note, add, done);
  return line;
}

// Focus `input` as the edit line opens, not each time it is drawn anew: it
// would take the focus back from the page.
function focusOnce(input) {
  if (editing.focused) return;
  editing.focused = true;
  queueMicrotask(() => input.focus());
}

function dateInput(value, label, key) {
  const input = editInput(value ? String(value).slice(0, 10) : '', label, key);
  input.type = 'date';
  return input;
}

// ----------------------------------------------------------------------------
// The trip `id`'s lanes as `change(lanes)` makes them, from its block as
// the host last pushed it; none left, the key goes.
function writeLanes(id, change) {
  const project = projects.get(id)?.project;
  if (!project) return;
  const lanes = change(project.lanes ?? []);
  setProjectLocal(id, withKey(project, 'lanes', lanes.length ? lanes : null));
}

// A lane shown changed: a work calendar not listed is listed with it.
function setLane(id, lane, change) {
  writeLanes(id, lanes => (lane.index === null
    ? [...lanes, change({ calendar: lane.source.doc_id })]
    : lanes.map((l, i) => (i === lane.index ? change(l) : l))));
}

// A calendar listed again takes the place of its lane left out.
function addLane(id, lane) {
  writeLanes(id, lanes => [...lanes.filter(l => !(lane.calendar && l.calendar === lane.calendar)), lane]);
}

// A work calendar's lane stays listed, `none`, so it isn't shown by
// itself again; any other goes.
function leaveOut(id, lane) {
  if (lane.work) return setLane(id, lane, l => withKey(l, 'counts', 'none'));
  writeLanes(id, lanes => lanes.filter((_, i) => i !== lane.index));
}
