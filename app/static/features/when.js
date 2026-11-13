// When (panel.md#when), the first list of a trip's tab line while its days
// are open (features/blueprint.js): the sketch's window as days, a lane per
// thing that limits them (`whenOf`, model/when.js), its name opening what
// it comes from in the panel; under them the windows that fit, one chosen,
// its band shading the lanes; then each window's days, why and cost, and
// Set the dates from the one chosen (`openSetDates`, features/set-dates.js).

import { accEl } from '../core/dom.js';
import { DAY_MONTH, DAY_SHORT_MONTH, WEEKDAY, addDays, daysBetween, localDay } from '../core/format.js';
import { t } from '../core/text.js';
import { projectCalendar } from '../model/blueprint.js';
import { writableCalendar } from '../model/calendar.js';
import { setDatesProposal } from '../model/trip.js';
import { iconEl } from '../ui/icons.js';
import { openSetDates } from './set-dates.js';
import { selectDocument } from './selection.js';

// The window chosen: `{ id, letter }`, the first one's while none is.
let chosen = null;

const SPAN_CLASS = { cant: 'is-block', worse: 'is-soft', reason: 'is-mark' };
const dayText = day => `${WEEKDAY.format(day)} ${DAY_MONTH.format(day)}`;
const rangeText = (from, to) => (daysBetween(from, to) ? `${dayText(from)} \u2013 ${dayText(to)}` : dayText(from));

// When of the trip `ctx` (`{ id, project, writable }`), `when` its
// `whenOf`; `render()` draws the blueprint again.
export function whenEls(ctx, when, render) {
  const head = accEl('p', 'meta when__head');
  if (!when.from) {
    head.textContent = t('when.no_window');
    return [head];
  }
  head.textContent = [rangeText(when.from, when.to), when.nights ? nightsRange(when.nights) : t('when.no_length_yet')]
    .join(' \u00b7 ');
  const fit = when.fits.find(f => chosen?.id === ctx.id && f.letter === chosen.letter) ?? when.fits[0] ?? null;
  const choose = f => {
    chosen = { id: ctx.id, letter: f.letter };
    render();
  };
  const parts = [head, gridEl(when, fit, choose)];
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
function gridEl(when, fit, choose) {
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
    const name = lane.source ? accEl('button', 'when__name') : accEl('span', 'when__name');
    name.append(iconEl(lane.icon), lane.name);
    name.title = laneTitle(lane);
    if (lane.source) {
      name.type = 'button';
      name.addEventListener('click', () => selectDocument(lane.source.doc_id));
    }
    const line = accEl('span', 'when__lane');
    for (const el of [name, line]) el.style.gridRow = row;
    grid.append(name, line, ...lane.spans.map(span => spanEl(span, row, col)));
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
// its title on it or after it, and with its days in its tooltip.
function spanEl(span, row, col) {
  const el = accEl('span', `when__span ${SPAN_CLASS[span.counts]}`);
  el.style.gridRow = row;
  el.style.gridColumn = `${col(span.from)} / ${col(span.to) + 1}`;
  el.title = [span.title, span.detail, rangeText(span.from, span.to)].filter(Boolean).join(' \u00b7 ');
  if (span.counts === 'reason') el.append(iconEl('#i-star-filled'));
  el.append(accEl('span', '', span.title));
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
