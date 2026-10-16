// The Timeline lens (ui.md#timeline): the dated entities in the vault
// panel's filter, of the types the lens shows (its own selection,
// features/lenses/types.js: the `tl_types` and `tl_types_bar`
// preferences), as a feed of day blocks under month headings, the newest
// first, a Today line between what is coming and what was; above it a
// scrubber with a bar per week over last year and this one, across which
// a drag picks a range of weeks that the feed keeps to (while the app
// runs). A row selects its entity and drags like a row of the Documents
// lens; a card dragged out of its thread onto the lens leaves it.

import { $, accEl, placeChildren } from '../../core/dom.js';
import {
  DAY_MONTH_YEAR, LONG_DAY, MONTH_YEAR, SHORT_MONTH_YEAR, TIME_FORMAT, WEEKDAY, dayStart,
  everyMidnight, localDay, plural, shortDate,
} from '../../core/format.js';
import { store } from '../../model/state.js';
import { entities, matchesFilter, projectNames } from '../../model/entities.js';
import { leaveThreadLocal } from '../../model/structure.js';
import { addDays, weekStartOf } from '../../model/calendar.js';
import { getDocIcon, iconEl } from '../../ui/icons.js';
import { endDragPreview, startDragPreview, takeMemberDrops } from '../../ui/dropzone.js';
import { TASK_STATUSES, parseContact, repeatsText } from '../panel/details.js';
import { markSelection, selectDocument } from '../selection.js';
import { invalidateLenses, registerLens } from './registry.js';
import { hiddenText, typeBar, typeOf } from './types.js';

const lensEl = $('lens-timeline');
const countEl = $('tl-count');
const rangeEl = $('tl-range');
const clearBtn = $('tl-clear');
const trackEl = $('tl-track');
const yearsEl = $('tl-years');
const feedEl = $('tl-feed');
const hiddenEl = $('tl-hidden');
const hiddenTextEl = $('tl-hidden-text');

const DAY_MS = 864e5;

const inRange = (date, { from, to }) => date >= from && date < to;

// ----------------------------------------------------------------------------
// The type selection: every type shown at first.
const tlTypes = typeBar({ lens: 'timeline', el: $('tl-types'), pref: 'tl_types', barPref: 'tl_types_bar' });

$('tl-show-all').addEventListener('click', tlTypes.showAll);

// ----------------------------------------------------------------------------
// An entry: an entity with a date, but a calendar (dated when it was
// connected) and an event cancelled in Google. A series shows once, on its
// first start; a task on its own date.
function isEntry(doc) {
  return !isNaN(doc.date) && doc.kind !== 'calendar' && doc.event?.status !== 'cancelled';
}

// What a row tells of its entity after the title, by its kind: an event's
// place, time and rule; a mail's sender and time; a task's status and due
// day; a chat's model and messages, a list's items, a contact's phone or
// mail, a bank account's bank and number, transactions' bookings; a
// document's author or sender and its genre; then its projects while no
// project is selected.
const TASK_STATUS_NAMES = Object.fromEntries(TASK_STATUSES);

function summaryOf(doc) {
  const time = doc.all_day ? '' : TIME_FORMAT.format(doc.date);
  const parts = [];
  if (doc.event) {
    parts.push(doc.event.location);
    if (time) parts.push(doc.end ? `${time}\u2013${TIME_FORMAT.format(doc.end)}` : time);
    if (doc.event.recurrence?.length) parts.push(repeatsText(doc.event.recurrence));
  } else if (doc.email) {
    if (doc.email.from) parts.push(parseContact(doc.email.from).name);
    parts.push(time);
  } else if (doc.task) {
    parts.push(TASK_STATUS_NAMES[doc.task.status] ?? doc.task.status);
    if (!isNaN(doc.due)) parts.push(`due ${shortDate(doc.due)}`);
  } else if (doc.conversation) {
    const messages = doc.conversation.messages ?? [];
    parts.push(messages.findLast(message => message.model)?.model, plural(messages.length, 'message'));
  } else if (doc.list) {
    parts.push(plural(doc.list.items?.length ?? 0, 'item'));
  } else if (doc.contact) {
    parts.push(doc.contact.phone?.[0] ?? doc.contact.email?.[0]);
  } else if (doc['bank-account']) {
    parts.push(doc['bank-account'].institution, doc['bank-account'].number);
  } else if (doc.transactions) {
    parts.push(plural(doc.transactions.transactions?.length ?? 0, 'booking'));
  } else {
    const by = doc.document?.author ?? doc.message?.sender;
    if (by) parts.push(parseContact(by).name);
    parts.push(doc.genre);
  }
  parts.push(projectNames(doc));
  return parts.filter(Boolean).join(' \u00b7 ');
}

// ----------------------------------------------------------------------------
// The range picked on the scrubber: its first Monday and the Monday after
// its last week; null for none.
let range = null;
// Whether the next render scrolls the feed to the Today line: once the
// vault is open, and on Today.
let scrollToToday = false;

// ----------------------------------------------------------------------------
// The scrubber: a bar per week (their Mondays in `shownWeeks`), from the
// week of 1 January last year to the week of 31 December this year, as high
// as its entries of the types shown against the busiest week's; the weeks
// to come lighter, a line at today, the years under the bars.
let shownWeeks = [];
let barEls = new Map(); // a Monday's day -> its bar
const todayMarkEl = accEl('span', 'scrub__today');

function weeksAround(today) {
  const year = today.getFullYear();
  const last = weekStartOf(new Date(year, 11, 31));
  const weeks = [];
  for (let monday = weekStartOf(new Date(year - 1, 0, 1)); monday <= last; monday = addDays(monday, 7)) {
    weeks.push(monday);
  }
  return weeks;
}

// The days from the first bar's Monday to `date`'s day (whole days, also
// across a change to or from summer time).
const daysIn = date => Math.round((dayStart(date) - shownWeeks[0]) / DAY_MS);

// Where `date` falls along the track, in percent.
const trackPos = date => daysIn(date) / (shownWeeks.length * 7) * 100;

function renderTrack(entries, today) {
  shownWeeks = weeksAround(today);
  const counts = shownWeeks.map(() => 0);
  for (const doc of entries) {
    const week = Math.floor(daysIn(doc.date) / 7);
    if (week >= 0 && week < counts.length) counts[week]++;
  }
  const busiest = Math.max(1, ...counts);
  const bars = new Map();
  for (const [i, monday] of shownWeeks.entries()) {
    const key = localDay(monday);
    const bar = barEls.get(key) ?? accEl('i', 'scrub__bar');
    bar.style.height = counts[i] ? `max(2px, ${counts[i] / busiest * 100}%)` : '0';
    bar.classList.toggle('is-future', monday > today);
    bar.title = `Week of ${DAY_MONTH_YEAR.format(monday)} \u00b7 ${plural(counts[i], 'entry', 'entries')}`;
    bars.set(key, bar);
  }
  barEls = bars;
  todayMarkEl.style.left = `${trackPos(today)}%`;
  placeChildren(trackEl, [...bars.values(), todayMarkEl]);
  const year = today.getFullYear();
  yearsEl.replaceChildren(...[year - 1, year].map(y => {
    const label = accEl('span', 'label scrub__year', String(y));
    label.style.left = `${trackPos(new Date(y, 0, 1))}%`;
    return label;
  }));
}

// The weeks of `picked` marked on the track (none for null).
function markRange(picked) {
  trackEl.classList.toggle('has-range', !!picked);
  for (const [i, bar] of [...barEls.values()].entries()) {
    bar.classList.toggle('is-in', !!picked && inRange(shownWeeks[i], picked));
  }
}

// A drag across the track picks the weeks it passes as the range; a click
// alone, the week under it.
let brushFrom = null; // the week the drag started on, by its index

function weekAt(ev) {
  const rect = trackEl.getBoundingClientRect();
  const week = Math.floor((ev.clientX - rect.left) / rect.width * shownWeeks.length);
  return Math.min(Math.max(week, 0), shownWeeks.length - 1);
}

function brushRange(ev) {
  const [first, last] = [brushFrom, weekAt(ev)].sort((a, b) => a - b);
  return { from: shownWeeks[first], to: addDays(shownWeeks[last], 7) };
}

trackEl.addEventListener('pointerdown', ev => {
  if (ev.button !== 0 || !shownWeeks.length) return;
  trackEl.setPointerCapture(ev.pointerId);
  brushFrom = weekAt(ev);
  markRange(brushRange(ev));
});

trackEl.addEventListener('pointermove', ev => {
  if (brushFrom !== null) markRange(brushRange(ev));
});

trackEl.addEventListener('pointerup', ev => {
  if (brushFrom === null) return;
  range = brushRange(ev);
  brushFrom = null;
  invalidateLenses('timeline');
});

trackEl.addEventListener('pointercancel', () => {
  brushFrom = null;
  invalidateLenses('timeline');
});

clearBtn.addEventListener('click', () => {
  range = null;
  invalidateLenses('timeline');
});

// Today: the feed scrolled to today, a range without it dropped.
$('tl-today').addEventListener('click', () => {
  if (range && !inRange(dayStart(new Date()), range)) range = null;
  scrollToToday = true;
  invalidateLenses('timeline');
});

// ----------------------------------------------------------------------------
// The feed's parts, kept across renders so a scan only adds: a month's
// heading, a day's block (the date beside its entries), an entity's row.
const monthEls = new Map(); // year * 12 + month -> its heading
const dayEls = new Map(); // `YYYY-MM-DD` -> its block
const rowEls = new Map(); // doc id -> its row
const rowFills = new WeakMap(); // a row -> { doc, fill } it was filled from
const todayEl = accEl('div', 'label tl__today');
const emptyEl = accEl('p', 'note tl__empty');

const monthKey = day => day.getFullYear() * 12 + day.getMonth();

function monthEl(day) {
  if (!monthEls.has(monthKey(day))) monthEls.set(monthKey(day), accEl('div', 'tl__month', MONTH_YEAR.format(day)));
  return monthEls.get(monthKey(day));
}

function dayEl(day) {
  const key = localDay(day);
  let el = dayEls.get(key);
  if (!el) {
    el = accEl('div', 'tl__day');
    const date = accEl('div', 'tl__date');
    date.append(
      accEl('span', 'tl__num', String(day.getDate())),
      accEl('span', 'label', `${WEEKDAY.format(day)} \u00b7 ${SHORT_MONTH_YEAR.format(day)}`));
    el.append(date, accEl('div', 'tl__entries'));
    dayEls.set(key, el);
  }
  return el;
}

// An entity's row: refilled when `entities` holds a new object for it, or
// `fill` (what else its meta names: the projects, the filter) changed.
function rowEl(doc, fill) {
  let el = rowEls.get(doc.doc_id);
  if (!el) {
    el = accEl('div', 'tl__row');
    el.dataset.id = doc.doc_id;
    el.tabIndex = 0;
    el.setAttribute('role', 'button');
    el.draggable = true;
    const text = accEl('span', 'tl__text');
    text.append(accEl('span', 'tl__title'), accEl('span', 'meta tl__meta'));
    el.append(iconEl('', 'table__icon'), text, accEl('span', 'label tl__labels'));
    rowEls.set(doc.doc_id, el);
  }
  const filled = rowFills.get(el);
  if (filled?.doc !== doc || filled.fill !== fill) {
    const [icon, text, labels] = el.children;
    icon.firstChild.setAttribute('href', getDocIcon(doc.kind, doc.genre));
    text.firstChild.textContent = doc.title;
    text.lastChild.textContent = summaryOf(doc);
    labels.textContent = (doc.tags ?? []).join(' \u00b7 ');
    rowFills.set(el, { doc, fill });
  }
  return el;
}

// The entries, newest first, as day blocks under their months' headings;
// the Today line before the first day that isn't to come, while today is
// in the range.
function renderFeed(listed, today, fill) {
  const days = [];
  for (const doc of listed) {
    const day = dayStart(doc.date);
    const last = days.at(-1);
    if (last?.day.getTime() === day.getTime()) last.docs.push(doc);
    else days.push({ day, docs: [doc] });
  }
  const nodes = [];
  let todayPlaced = !listed.length || (range && !inRange(today, range));
  let month = null;
  for (const { day, docs } of days) {
    const future = day > today;
    if (!future && !todayPlaced) {
      nodes.push(todayEl);
      todayPlaced = true;
    }
    if (monthKey(day) !== month) {
      nodes.push(monthEl(day));
      month = monthKey(day);
    }
    const block = dayEl(day);
    block.classList.toggle('tl__day--future', future);
    placeChildren(block.lastChild, docs.map(doc => rowEl(doc, fill)));
    nodes.push(block);
  }
  if (!todayPlaced) nodes.push(todayEl);
  if (!listed.length) {
    emptyEl.textContent = range ? 'Nothing in these weeks.' : 'Nothing with a date here.';
    nodes.push(emptyEl);
  }
  todayEl.textContent = `Today \u00b7 ${LONG_DAY.format(today)}`;
  placeChildren(feedEl, [...nodes, hiddenEl]);
}

const markRows = markSelection(feedEl);

function renderTimeline() {
  const state = store.getState();
  const today = dayStart(new Date());
  const shown = tlTypes.shown();
  // the filter's entries: those of the types shown on the track; in the
  // range, counted by type, and those of a type shown listed, newest first
  const onTrack = [];
  const counts = new Map();
  const listed = [];
  for (const doc of entities.values()) {
    if (!isEntry(doc) || !matchesFilter(doc, state)) continue;
    const type = typeOf(doc);
    if (shown.has(type)) onTrack.push(doc);
    if (range && !inRange(doc.date, range)) continue;
    counts.set(type, (counts.get(type) ?? 0) + 1);
    if (shown.has(type)) listed.push(doc);
  }
  listed.sort((a, b) => b.date - a.date);

  for (const id of rowEls.keys()) if (!entities.has(id)) rowEls.delete(id);
  renderTrack(onTrack, today);
  markRange(range);
  renderFeed(listed, today, `${state.projects_rev}\n${state.selected_project}`);
  markRows();
  if (scrollToToday && listed.length) {
    feedEl.scrollTop = todayEl.isConnected ? todayEl.offsetTop : 0;
    scrollToToday = false;
  }

  tlTypes.render(shown, counts);
  countEl.textContent = `Time \u00b7 ${plural(listed.length, 'entry', 'entries')}`;
  rangeEl.hidden = !range;
  rangeEl.textContent = range ? DAY_MONTH_YEAR.formatRange(range.from, addDays(range.to, -1)) : '';
  clearBtn.hidden = !range;
  const hidden = [...counts].reduce((n, [type, k]) => n + (shown.has(type) ? 0 : k), 0);
  hiddenEl.hidden = !hidden;
  hiddenTextEl.textContent = hidden ? `${hiddenText(shown, counts)} hidden` : '';
}

// ----------------------------------------------------------------------------
// A row selects its entity, by a click or Enter or Space; it drags like a
// row of the Documents lens, onto a project pill, a card or the thread
// panel.
feedEl.addEventListener('click', ev => {
  const row = ev.target.closest('.tl__row');
  if (row) selectDocument(row.dataset.id);
});

feedEl.addEventListener('keydown', ev => {
  const row = ev.target.closest('.tl__row');
  if (!row || (ev.key !== 'Enter' && ev.key !== ' ')) return;
  ev.preventDefault();
  selectDocument(row.dataset.id);
});

feedEl.addEventListener('dragstart', ev => {
  const row = ev.target.closest('.tl__row');
  if (!row) return;
  const doc = rowFills.get(row).doc;
  ev.dataTransfer.setData('text/plain', row.dataset.id);
  startDragPreview(ev, getDocIcon(doc.kind, doc.genre), doc.title);
});

feedEl.addEventListener('dragend', endDragPreview);

// A card dragged out of its thread (features/panel/panel.js) and dropped
// on the lens leaves the thread (`leaveThreadLocal`).
takeMemberDrops(lensEl, leaveThreadLocal);

// ----------------------------------------------------------------------------
// Once a vault is open, the next render starts at today; at midnight, today
// moves on.
store.subscribe(state => state.vault_state, vault_state => {
  if (vault_state !== 'open') return;
  scrollToToday = true;
  invalidateLenses('timeline');
});
everyMidnight(() => invalidateLenses('timeline'));

registerLens('timeline', lensEl, renderTimeline);
