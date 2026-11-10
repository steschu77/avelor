// The dashboard's cards (ui.md#dashboard): Week, Tasks, Upcoming, Payments,
// Recent files and Recent conversations, each drawn from the page's data in the
// vault panel's filter (`matchesFilter`). A card is `{ id, title, size, render }`:
// `size` is `full`, `wide` or none (a third of the grid), and
// `render(ctx)` gives the nodes of its head (after its title) and of its
// body. `ctx`: `today`, `shown(id)` (whether a card is shown) and
// `refresh()` (draw the dashboard again). The cards, their order and the
// layout are features/dashboard/dashboard.js's.
//
// A row, an event or a task selects its entity (`selectDocument`), by click
// or Enter; it carries `data-id`, so the dashboard marks the selected one (a
// booking's row selects the booking, and also carries `data-booking`). A
// task dragged to another column of the Tasks card takes its status
// (`editEntityLocal`).

import { accEl } from '../../core/dom.js';
import { pressable } from '../../core/dom.js';
import { t } from '../../core/text.js';
import {
  DAY_LONG_MONTH, DAY_MONTH, DAY_MONTH_YEAR, DAY_SHORT_MONTH, MONTH, TIME_FORMAT, WEEKDAY, addDays, dayStart,
  parseDate, sameDay, shortDate, signedAmount, weekStartOf,
} from '../../core/format.js';
import { savePrefs, store } from '../../model/state.js';
import { editEntityLocal } from '../../model/structure.js';
import {
  entities, isAiConversation, matchesFilter, projectNames,
} from '../../model/entities.js';
import { eventsBetween, eventsOnDay, getDueTasks, getEvents, isOverdue } from '../../model/calendar.js';
import {
  amountOn, bankScope, bookingInProject, bookingName, expectedIn, horizonOf, occurrences, pairOf, recurringPaymentEnded,
  recurringPaymentsIn, recurringPaymentInProject, recurringPaymentOf,
} from '../../model/bank.js';
import { KIND_ICONS, bookingIcon, getDocIcon, iconEl } from '../../ui/icons.js';
import { clearDropMark, dropTarget } from '../../ui/dropzone.js';
import { showMenu } from '../../ui/menu.js';
import { selectBooking, selectDocument } from '../selection.js';
import { TASK_STATUSES } from '../panel/details.js';

// ----------------------------------------------------------------------------
// An element that selects the entity `id` on a click, or Enter or Space, or
// does `open` instead.
function entityEl(tag, className, id, open = () => selectDocument(id)) {
  const el = accEl(tag, className);
  el.dataset.id = id;
  if (tag === 'button') el.type = 'button';
  else pressable(el);
  el.addEventListener('click', open);
  return el;
}

// A tool button of a card's head.
function toolButton(className, text, title, onClick) {
  const btn = accEl('button', className, text);
  btn.type = 'button';
  if (title) btn.title = title;
  btn.addEventListener('click', onClick);
  return btn;
}

// A tool button of a card's head that shows the icon `href` (ui/icons.js).
function toolIcon(href, title, onClick) {
  const btn = toolButton('tool', '', title, onClick);
  btn.append(iconEl(href));
  return btn;
}

// ----------------------------------------------------------------------------
// Week: seven days from today, or the calendar week from Monday (the
// `week_start` preference); previous and next move by a week.
let weekOffset = 0; // weeks from this one

// The first day of this week's strip.
function weekBase(today) {
  return store.getState().prefs.week_start === 'monday' ? weekStartOf(today) : today;
}

let weekRefresh = () => {};
const weekRangeEl = accEl('span', 'meta');
const weekTodayBtn = toolButton('tool', t('cal.today'), '', () => moveWeek(-weekOffset));
const weekGearBtn = toolIcon('#i-settings', t('dash.week_settings'), showWeekMenu);
weekGearBtn.setAttribute('aria-haspopup', 'menu');
const weekTools = accEl('span', 'toolbar');
weekTools.append(
  toolIcon('#i-chevron-left', t('dash.previous_week'), () => moveWeek(-1)),
  toolIcon('#i-chevron-right', t('dash.next_week'), () => moveWeek(1)),
  weekTodayBtn, weekGearBtn);

function moveWeek(by) {
  weekOffset += by;
  weekRefresh();
}

function showWeekMenu() {
  const monday = store.getState().prefs.week_start === 'monday';
  const pick = week_start => () => {
    weekOffset = 0;
    savePrefs({ week_start });
  };
  showMenu(weekGearBtn, [
    { label: t('cal.today'), hint: t('dash.week_today_hint'), checked: !monday, run: pick('today') },
    { label: t('dash.week_monday'), hint: t('dash.week_monday_hint'), checked: monday, run: pick('monday') },
  ], { head: t('dash.week_starts') });
}

function renderWeek({ today, refresh }) {
  weekRefresh = refresh;
  const start = addDays(weekBase(today), 7 * weekOffset);
  const end = addDays(start, 7);
  const events = eventsBetween(start, end, getEvents());
  const tasks = getDueTasks().filter(task => task.due >= start && task.due < end);

  const week = accEl('div', 'week');
  for (let i = 0; i < 7; i++) {
    const day = addDays(start, i);
    let cls = 'week__day';
    if (day < today) cls += ' week__day--past';
    if (sameDay(day, today)) cls += ' week__day--today';
    if (day.getDay() % 6 === 0) cls += ' week__day--weekend';
    const dayEl = accEl('div', cls);
    const head = accEl('div', 'week__head');
    head.append(
      accEl('b', 'week__num', String(day.getDate())),
      accEl('span', 'label week__dow', WEEKDAY.format(day)),
      accEl('span', 'meta week__month', i === 0 || day.getDate() === 1 ? MONTH.format(day) : ''));
    dayEl.append(head);
    for (const task of tasks.filter(t => sameDay(t.due, day))) dayEl.append(weekItem(task, 'due'));
    for (const ev of eventsOnDay(day, events)) dayEl.append(weekItem(ev, ev.all_day ? '' : TIME_FORMAT.format(ev.date)));
    week.append(dayEl);
  }

  weekRangeEl.textContent = DAY_LONG_MONTH.formatRange(start, addDays(start, 6));
  weekTodayBtn.setAttribute('aria-disabled', String(weekOffset === 0));
  return { head: [weekRangeEl, weekTools], body: [week] };
}

// An event, or a task due, on its day of the strip.
function weekItem(doc, time) {
  const el = entityEl('button', 'week__event', doc.doc_id);
  el.title = doc.title;
  el.append(accEl('span', 'week__time', time), accEl('span', 'week__title', doc.title));
  return el;
}

// ----------------------------------------------------------------------------
// Tasks: the entities with a task block in the columns of their status (an
// unknown one under To do); the open ones by due date, Done the last done
// first, three of them unless unfolded. A task the host can write drags to
// another column, which sets its status.
const DONE_SHOWN = 3;
let doneUnfolded = false;

// A task dragged by its card carries its id as this type, which only the
// board's columns take; as `text/plain` too, so a project pill in the vault
// panel takes it like a table row.
const TASK_DRAG = 'application/x-avelor-task';
let draggedTask = null; // { id, status } of the task being dragged

function renderTasks({ today, refresh }) {
  const statuses = new Set(TASK_STATUSES.map(([status]) => status));
  const statusOf = doc => (statuses.has(doc.task.status) ? doc.task.status : 'todo');
  const tasks = [...entities.values()].filter(doc => doc.task && matchesFilter(doc));

  const board = accEl('div', 'board');
  for (const [status, name] of TASK_STATUSES) {
    const done = status === 'done';
    const list = tasks.filter(task => statusOf(task) === status).sort(done ? byDoneDesc : byDue);
    const col = accEl('div', 'board__col');
    dropColumn(col, status);
    const head = accEl('div', 'board__head label');
    head.append(accEl('span', 'board__name', name), accEl('span', '', String(list.length)));
    col.append(head);
    const clipped = done && list.length > DONE_SHOWN;
    for (const task of clipped && !doneUnfolded ? list.slice(0, DONE_SHOWN) : list) {
      col.append(taskCard(task, status, today));
    }
    if (!list.length) col.append(accEl('p', 'note board__empty', t('dash.no_tasks')));
    if (clipped) {
      const more = doneUnfolded ? t('dash.show_fewer') : t('dash.n_more', { n: list.length - DONE_SHOWN });
      col.append(toolButton('btn btn--ghost board__more', more, '', () => {
        doneUnfolded = !doneUnfolded;
        refresh();
      }));
    }
    board.append(col);
  }

  const open = tasks.filter(task => statusOf(task) !== 'done');
  const overdue = open.filter(task => isOverdue(task, today)).length;
  const count = [t('dash.open_n', { n: open.length }), overdue ? t('cal.overdue_n', { n: overdue }) : null].filter(Boolean).join(' \u00b7 ');
  return { head: [accEl('span', 'meta', count)], body: [board] };
}

// Whether `date` is one (a host date that didn't parse isn't).
function isDate(date) {
  return !!date && !isNaN(date);
}

// Open tasks by due date (none last), then title.
function byDue(a, b) {
  const da = isDate(a.due) ? a.due.getTime() : Infinity;
  const db = isDate(b.due) ? b.due.getTime() : Infinity;
  return (da === db ? 0 : da < db ? -1 : 1) || (a.title ?? '').localeCompare(b.title ?? '');
}

// The later of two dates first, a missing one last.
function newestFirst(a, b) {
  const ta = isDate(a) ? a.getTime() : -Infinity;
  const tb = isDate(b) ? b.getTime() : -Infinity;
  return ta === tb ? 0 : ta > tb ? -1 : 1;
}

// The newest first, those without a date last.
function byDateDesc(a, b) {
  return newestFirst(a.date, b.date);
}

// Done tasks: the last done first, then those without a done date (a file
// edited by hand), the newest first.
function byDoneDesc(a, b) {
  return newestFirst(a.done, b.done) || byDateDesc(a, b);
}

// A task on the board in its column's `status`: its title; when it is due,
// as a chip once overdue, or when it was done; its projects.
function taskCard(task, status, today) {
  const done = status === 'done';
  const el = entityEl('div', done ? 'task task--done' : 'task', task.doc_id);
  const meta = accEl('span', 'task__meta');
  if (done && isDate(task.done)) {
    meta.append(accEl('span', 'meta', t('dash.done_on', { day: shortDate(task.done) })));
  } else if (!done && isDate(task.due)) {
    meta.append(isOverdue(task, today)
      ? accEl('span', 'chip chip--danger', t('dash.overdue_on', { day: shortDate(task.due) }))
      : accEl('span', 'meta', t('dash.due_on', { day: shortDate(task.due) })));
  }
  const names = projectNames(task);
  if (names) meta.append(accEl('span', 'meta', names));
  el.append(accEl('span', 'task__title', task.title), meta);
  if (task.writable) dragTask(el, task.doc_id, status);
  return el;
}

// The task `id`'s card `el` drags it out of the column of `status`; the
// card itself is the drag image, dimmed once that is taken.
function dragTask(el, id, status) {
  el.draggable = true;
  el.addEventListener('dragstart', e => {
    e.dataTransfer.effectAllowed = 'copyMove';
    e.dataTransfer.setData(TASK_DRAG, id);
    e.dataTransfer.setData('text/plain', id);
    draggedTask = { id, status };
    setTimeout(() => el.classList.add('is-dragging'), 0);
  });
  el.addEventListener('dragend', () => {
    el.classList.remove('is-dragging');
    draggedTask = null;
    clearDropMark();
  });
}

// The column of `status` takes a task dragged from another one: marked
// while it is over it, the task dropped there gets its status.
function dropColumn(col, status) {
  dropTarget(col, {
    over: e => (e.dataTransfer.types.includes(TASK_DRAG) && draggedTask?.status !== status ? { el: col } : null),
    drop: (_at, e) => {
      const id = e.dataTransfer.getData(TASK_DRAG);
      draggedTask = null;
      if (id) editEntityLocal(id, { task: { status } });
    },
    effect: 'move',
  });
}

// ----------------------------------------------------------------------------
// Upcoming: the next events and open tasks due after the Week's strip (from
// today while the Week card is closed), within a year.
const UPCOMING_SHOWN = 6;

function renderUpcoming({ today, shown }) {
  const from = shown('week') ? addDays(weekBase(today), 7) : today;
  const until = addDays(from, 366);
  const items = [
    ...getDueTasks()
      .filter(task => task.due >= from && task.due < until)
      .map(task => ({ doc: task, day: dayStart(task.due), time: 'due', task: true })),
    ...eventsBetween(from, until, getEvents())
      .map(ev => ({ doc: ev, day: ev.date, time: ev.all_day ? '' : TIME_FORMAT.format(ev.date), task: false })),
  ]
    .sort((a, b) => a.day - b.day || Number(b.task) - Number(a.task))
    .slice(0, UPCOMING_SHOWN);

  const rows = items.map(({ doc, day, time }) => {
    const row = entityEl('div', 'list-row feed__row feed__row--event', doc.doc_id);
    row.append(
      accEl('span', 'meta', `${WEEKDAY.format(day)} ${DAY_MONTH.format(day)}`),
      iconEl(getDocIcon(doc.kind, doc.genre), 'table__icon'),
      accEl('span', 'row-title', doc.title),
      accEl('span', 'meta', time));
    return row;
  });
  const note = sameDay(from, today) ? [] : [accEl('span', 'meta', t('recurring.from', { day: DAY_SHORT_MONTH.format(from) }))];
  return { head: note, body: [feed(rows, t('dash.nothing_planned'))] };
}

// ----------------------------------------------------------------------------
// Payments: what the recurring payments (vault.md#recurring) of the bank
// accounts in the filter (`bankScope`; under a project, the recurring payments in it,
// `recurringPaymentInProject`) are due to pay or bring -- the running ones' missed
// occurrences first, Overdue, then those no booking pays yet to the end of
// next month (`expectedOf`, `horizonOf`), soonest first -- over the newest
// bookings in the filter, transfers between the accounts left out. A due
// payment selects its recurring payment, a booking itself.
const DUE_SHOWN = 5;
const BOOKED_SHOWN = 4;

function renderPayments({ today }) {
  const state = store.getState();
  const scope = bankScope(state);
  const horizon = horizonOf(scope.accounts, today);
  const recurringPayments = recurringPaymentsIn(new Set(scope.accounts.map(doc => doc.id)))
    .filter(doc => recurringPaymentInProject(doc, state.selected_project));
  const missed = recurringPayments.filter(doc => !recurringPaymentEnded(doc, today))
    .flatMap(doc => occurrences(doc).filter(entry => entry.state === 'missed')
      .map(entry => ({ recurringPayment: doc, day: entry.day, amount: amountOn(doc, entry.day) ?? 0 })))
    .sort((a, b) => a.day - b.day);
  const due = expectedIn(recurringPayments, horizon);
  const booked = [];
  for (const statement of scope.statements) {
    for (const booking of statement.statement.bookings ?? []) {
      const date = parseDate(booking.date);
      if (date && !pairOf(booking) && bookingInProject(booking, state.selected_project)) {
        booked.push({ statement, booking, date });
      }
    }
  }
  booked.sort((a, b) => b.date - a.date);

  const dueRows = [...missed.map(entry => ({ ...entry, missed: true })), ...due].slice(0, DUE_SHOWN).map(dueRow);
  const bookedRows = booked.slice(0, BOOKED_SHOWN).map(bookedRow);
  const rows = [];
  if (dueRows.length) rows.push(accEl('span', 'label feed__head', t('props.due')), ...dueRows);
  if (bookedRows.length) rows.push(accEl('span', 'label feed__head', t('dash.booked')), ...bookedRows);
  const counts = [missed.length ? t('occurrence.count_missed', { n: missed.length }) : null,
    due.length ? t('dash.due_to', { n: due.length, day: DAY_SHORT_MONTH.format(horizon) }) : null].filter(Boolean);
  return {
    head: counts.length ? [accEl('span', 'meta', counts.join(' \u00b7 '))] : [],
    body: [feed(rows, t('dash.no_payments'))],
  };
}

// A payment due: its day, the recurring payment's icon and title (a missed one tagged
// Overdue), its amount; not there yet, as a pending booking.
function dueRow({ recurringPayment, day, amount, missed = false }) {
  const row = entityEl('div', 'list-row feed__row feed__row--event feed__row--due', recurringPayment.doc_id);
  const title = accEl('span', 'row-title', recurringPayment.title || t('untitled'));
  if (missed) {
    const tag = accEl('span', 'chip chip--danger', t('occurrence.missed'));
    tag.title = t('dash.no_booking_for', { day: DAY_MONTH_YEAR.format(day) });
    title.append(tag);
  }
  row.append(accEl('span', 'meta', `${WEEKDAY.format(day)} ${DAY_MONTH.format(day)}`),
    iconEl(KIND_ICONS['recurring-payment'].icon, 'table__icon'), title, amountEl(amount));
  return row;
}

// A booking: its day, its icon, who paid or was paid ("Pending" while it
// is), its amount.
function bookedRow({ statement, booking, date }) {
  const open = booking.id ? () => selectBooking(statement.doc_id, booking.id) : () => selectDocument(statement.doc_id);
  const row = entityEl('div', 'list-row feed__row feed__row--event', statement.doc_id, open);
  if (booking.id) row.dataset.booking = booking.id;
  const title = accEl('span', 'row-title', bookingName(booking, null));
  if (booking.status === 'pending') title.append(accEl('span', 'chip chip--pending', t('bank.pending_chip')));
  row.append(accEl('span', 'meta', `${WEEKDAY.format(date)} ${DAY_MONTH.format(date)}`),
    iconEl(bookingIcon(null, recurringPaymentOf(booking)), 'table__icon'), title, amountEl(booking.amount ?? 0));
  return row;
}

// An amount, signed; money in bold, as in the Bank lens.
function amountEl(amount) {
  return accEl('span', `feed__amount${amount > 0 ? ' feed__amount--in' : ''}`, signedAmount(amount));
}

// ----------------------------------------------------------------------------
// Recent files: the newest dated entities that aren't events, tasks or
// conversations with an AI, nor a calendar (whose date is when it was connected).
const FILES_SHOWN = 6;
const NOT_FILES = new Set(['event', 'calendar']);

function renderFiles() {
  const files = [...entities.values()]
    .filter(doc => isDate(doc.date) && !NOT_FILES.has(doc.kind) && !doc.task
      && !isAiConversation(doc) && matchesFilter(doc))
    .sort(byDateDesc)
    .slice(0, FILES_SHOWN);
  const rows = files.map(doc => {
    const row = entityEl('div', 'list-row feed__row', doc.doc_id);
    row.append(
      iconEl(getDocIcon(doc.kind, doc.genre), 'table__icon'),
      accEl('span', 'row-title', doc.title),
      accEl('span', 'meta', shortDate(doc.date)));
    return row;
  });
  return { head: [], body: [feed(rows, t('dash.no_files'))] };
}

// ----------------------------------------------------------------------------
// Recent conversations: the newest conversations with an AI, with their projects.
const CHATS_SHOWN = 4;

function renderConversations() {
  const conversations = [...entities.values()]
    .filter(doc => isAiConversation(doc) && matchesFilter(doc))
    .sort(byDateDesc)
    .slice(0, CHATS_SHOWN);
  const rows = conversations.map(doc => {
    const row = entityEl('div', 'list-row feed__row', doc.doc_id);
    const when = isDate(doc.date) ? shortDate(doc.date) : '';
    row.append(
      iconEl(getDocIcon(doc.kind, doc.genre), 'table__icon'),
      accEl('span', 'row-title', doc.title),
      accEl('span', 'meta', [projectNames(doc), when].filter(Boolean).join(' \u00b7 ')));
    return row;
  });
  return { head: [], body: [feed(rows, t('dash.no_conversations'))] };
}

// ----------------------------------------------------------------------------
// A list of one-line rows, or `empty` without any.
function feed(rows, empty) {
  const list = accEl('div', 'feed');
  list.append(...(rows.length ? rows : [accEl('p', 'note feed__empty', empty)]));
  return list;
}

// ----------------------------------------------------------------------------
// The cards in their default order.
export const CARDS = [
  { id: 'week', title: t('dash.card.week'), size: 'full', render: renderWeek },
  { id: 'tasks', title: t('dash.card.tasks'), size: 'wide', render: renderTasks },
  { id: 'upcoming', title: t('dash.card.upcoming'), render: renderUpcoming },
  { id: 'payments', title: t('dash.card.payments'), render: renderPayments },
  { id: 'files', title: t('dash.card.files'), render: renderFiles },
  { id: 'conversations', title: t('dash.card.conversations'), render: renderConversations },
];
