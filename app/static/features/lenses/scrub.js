// The scrubber of a lens (ui.md#timeline, ui.md#bank): a head -- what the
// lens counts, the range picked with Clear, Today -- over a track of a bar
// per week with a dashed line at today, and labels under it. A drag across
// the track picks the weeks it passes as the range, a click alone the week
// under it; the range lasts while the app runs. A lens makes its scrubber
// once with `scrubber`, draws each week's bar and keeps to the range
// (`holds`); a change of the range renders the lens again.

import { accEl, placeChildren } from '../../core/dom.js';
import { DAY_MONTH_YEAR, MONTH, SHORT_MONTH_YEAR, dayStart, daysBetween, localDay } from '../../core/format.js';
import { addDays, weekStartOf } from '../../model/calendar.js';
import { invalidateLenses } from './registry.js';

// The weeks' first days, from `first`'s week to `last`'s.
export function weeksBetween(first, last) {
  const weeks = [];
  for (let day = weekStartOf(first); day <= last; day = addDays(day, 7)) weeks.push(day);
  return weeks;
}

// The index of the week of `weeks` that holds `date`, or -1.
export function weekOf(weeks, date) {
  const week = Math.floor(daysBetween(weeks[0], date) / 7);
  return week >= 0 && week < weeks.length ? week : -1;
}

// The labels under the track, from the first week's first day to the last
// week's: every 1 January ('years'), or every month's first day, the first
// one and each January with the year ('months').
function labelsOf(weeks, by) {
  const step = by === 'years' ? 12 : 1;
  const labels = [];
  for (let day = new Date(weeks[0].getFullYear(), by === 'years' ? 0 : weeks[0].getMonth(), 1);
    day <= weeks.at(-1); day = new Date(day.getFullYear(), day.getMonth() + step, 1)) {
    if (day < weeks[0]) continue;
    const text = by === 'years' ? String(day.getFullYear())
      : !labels.length || !day.getMonth() ? SHORT_MONTH_YEAR.format(day) : MONTH.format(day);
    labels.push({ day, text });
  }
  return labels;
}

// The scrubber of the lens `lens`, its element `el` placed by the lens;
// `meta` says how to read its bars, `track` is a class of its track's
// besides `scrub__track`, `onToday` what Today does besides dropping a
// range without today. `render` draws it: its `title`, the
// `weeks` (their first days), `today`, each week's bar from `bar(i, kept)`
// (the element it had, kept by its day, or none yet) and the `labels`,
// 'years' or 'months'. `range()` is the range picked, its first week's
// first day and the day after its last week, or null; `holds(date)` whether
// `date` is in it, or there is none.
export function scrubber({ lens, meta = null, track = null, onToday = null }) {
  const titleEl = accEl('span', 'label');
  const rangeEl = accEl('span', 'label scrub__range');
  const clearBtn = accEl('button', 'tool label', 'Clear');
  const todayBtn = accEl('button', 'tool label push', 'Today');
  clearBtn.type = todayBtn.type = 'button';
  rangeEl.hidden = clearBtn.hidden = true;
  const head = accEl('div', 'scrub__head');
  head.append(titleEl, ...(meta ? [accEl('span', 'meta', meta)] : []), rangeEl, clearBtn, todayBtn);
  const trackEl = accEl('div', track ? `scrub__track ${track}` : 'scrub__track');
  trackEl.title = 'Drag to pick a range of weeks';
  const labelsEl = accEl('div', 'scrub__years');
  const todayEl = accEl('span', 'scrub__today');
  const el = accEl('div', 'scrub');
  el.append(head, trackEl, labelsEl);

  let range = null;
  let weeks = [];
  let barEls = new Map(); // a week's first day -> its bar
  let brushFrom = null; // the week a drag started on, by its index

  const holds = date => !range || (date >= range.from && date < range.to);

  // Where `date` falls along the track, in percent.
  const trackPos = date => daysBetween(weeks[0], date) / (weeks.length * 7) * 100;

  // The weeks of `picked` marked on the track (none for null).
  function mark(picked) {
    trackEl.classList.toggle('has-range', !!picked);
    for (const [i, bar] of [...barEls.values()].entries()) {
      bar.classList.toggle('is-in', !!picked && weeks[i] >= picked.from && weeks[i] < picked.to);
    }
  }

  function render({ title, weeks: shown, today, bar, labels }) {
    weeks = shown;
    titleEl.textContent = title;
    const bars = new Map();
    for (const [i, day] of weeks.entries()) {
      const key = localDay(day);
      const el = bar(i, barEls.get(key));
      el.classList.toggle('is-future', day > today);
      bars.set(key, el);
    }
    barEls = bars;
    todayEl.style.left = `${trackPos(today)}%`;
    placeChildren(trackEl, [...bars.values(), todayEl]);
    labelsEl.replaceChildren(...labelsOf(weeks, labels).map(({ day, text }) => {
      const label = accEl('span', 'label scrub__year', text);
      label.style.left = `${trackPos(day)}%`;
      return label;
    }));
    mark(range);
    rangeEl.hidden = clearBtn.hidden = !range;
    rangeEl.textContent = range ? DAY_MONTH_YEAR.formatRange(range.from, addDays(range.to, -1)) : '';
  }

  function weekAt(ev) {
    const rect = trackEl.getBoundingClientRect();
    const week = Math.floor((ev.clientX - rect.left) / rect.width * weeks.length);
    return Math.min(Math.max(week, 0), weeks.length - 1);
  }

  function brushRange(ev) {
    const [first, last] = [brushFrom, weekAt(ev)].sort((a, b) => a - b);
    return { from: weeks[first], to: addDays(weeks[last], 7) };
  }

  trackEl.addEventListener('pointerdown', ev => {
    if (ev.button !== 0 || !weeks.length) return;
    trackEl.setPointerCapture(ev.pointerId);
    brushFrom = weekAt(ev);
    mark(brushRange(ev));
  });

  trackEl.addEventListener('pointermove', ev => {
    if (brushFrom !== null) mark(brushRange(ev));
  });

  trackEl.addEventListener('pointerup', ev => {
    if (brushFrom === null) return;
    range = brushRange(ev);
    brushFrom = null;
    invalidateLenses(lens);
  });

  trackEl.addEventListener('pointercancel', () => {
    brushFrom = null;
    invalidateLenses(lens);
  });

  clearBtn.addEventListener('click', () => {
    range = null;
    invalidateLenses(lens);
  });

  todayBtn.addEventListener('click', () => {
    if (!holds(dayStart(new Date()))) range = null;
    onToday?.();
    invalidateLenses(lens);
  });

  return { el, range: () => range, holds, render };
}
