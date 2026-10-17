// Calendar lens: the month grid in #lens-calendar, with the events and the
// open tasks due as chips (today also counts the overdue ones); a chip's
// click selects its day and its card in the panel's day stack
// (features/panel/panel.js's `renderDay`, scrolled to with its
// `scrollIntoView`). The events, their series expanded, and the tasks due
// are model/calendar.js's, a task's tag features/panel/details.js's, the
// selection features/selection.js's; the lens registers itself
// (features/lenses/registry.js).

import { $, accEl } from '../../core/dom.js';
import { MONTH_YEAR, TIME_FORMAT, dayStart, everyMidnight, plural } from '../../core/format.js';
import { store } from '../../model/state.js';
import {
    addDays, eventsBetween, eventsOnDay, getDueTasks, getEvents, monthEndOf,
    monthStartOf, sameDay, weekStartOf,
} from '../../model/calendar.js';
import { KIND_ICONS, iconEl } from '../../ui/icons.js';
import { markSelection, selectDay } from '../selection.js';
import { invalidateLenses, registerLens } from './registry.js';
import { dueTag, taskTag } from '../panel/details.js';
import { refreshDay, scrollIntoView, scrollToOverdue } from '../panel/panel.js';

const calContainerEl = $('lens-calendar');
const calHeadRowEl = $('calendar-headrow');
const calWeeksContainerEl = $('calendar-weeks');
const calTitleEl = $('cal-title');

let calFocus = new Date(); // the day the current view centers on
let calSelectedDay = null; // the day clicked in the grid, if any
// the chips of the selected entity, each occurrence of a series, are marked
const markSelected = markSelection(calWeeksContainerEl);

// ----------------------------------------------------------------------------
export function initCalendarLens() {
    for (const day of ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']) {
        const dayEl = accEl('div', '', day);
        if (day === 'Sat' || day === 'Sun') {
            dayEl.classList.add('is-weekend');
        }
        calHeadRowEl.appendChild(dayEl);
    }

    // The grid shows the selected day: one outside its weeks moves it to
    // that day's month.
    store.subscribe(state => state.selected_day,
        selected_day => {
            calSelectedDay = selected_day;
            const { start, end } = gridRange(calFocus);
            if (selected_day && (selected_day < start || selected_day >= end)) calFocus = selected_day;
            renderCalendar();
        });

    renderCalendar();
    everyMidnight(onMidnight);
}

// ----------------------------------------------------------------------------
// At midnight the grid's today moves on, and so does a selected day that
// was today (yesterday now).
function onMidnight() {
    const { selected_day } = store.getState();
    const today = dayStart(new Date());
    if (selected_day && sameDay(selected_day, addDays(today, -1))) store.setState({ selected_day: today });
    invalidateLenses('calendar');
}

// ----------------------------------------------------------------------------
// The calendar lens as `invalidateLenses` renders it: the grid, and the
// day stack in the panel, which follows the same entities and filter.
function renderCalendarLens() {
    renderCalendar();
    refreshDay();
}

// ----------------------------------------------------------------------------
// The grid's weeks for the month of `focus`, Monday first: its first day,
// and the day after its last.
function gridRange(focus) {
    const start = weekStartOf(monthStartOf(focus));
    return { start, end: addDays(weekStartOf(monthEndOf(focus)), 7) };
}

// ----------------------------------------------------------------------------
function renderCalendar() {
    calWeeksContainerEl.replaceChildren();

    calTitleEl.textContent = calTitleText();

    // only the events and tasks in the grid's weeks, so each cell looks at
    // those alone
    const { start, end } = gridRange(calFocus);
    const events = eventsBetween(start, end, getEvents());
    const allTasks = getDueTasks();
    const tasks = allTasks.filter(task => task.due >= start && task.due < end);

    const today = dayStart(new Date());
    const overdue = allTasks.filter(task => task.due < today).length;
    for (let weekStart = start; weekStart < end; weekStart = addDays(weekStart, 7)) {
        const row = accEl('div', 'cal__week');
        for (let i = 0; i < 7; i++) {
            const d = addDays(weekStart, i);
            const isToday = sameDay(d, today);
            const items = {
                events: eventsOnDay(d, events),
                tasks: tasks.filter(task => sameDay(task.due, d)),
                overdue: isToday ? overdue : 0,
            };
            row.appendChild(
                monthCell(d, items, d.getMonth() === calFocus.getMonth(), isToday, calSelectedDay !== null && sameDay(d, calSelectedDay))
            );
        }
        calWeeksContainerEl.appendChild(row);
    }
    markSelected();
}

// ----------------------------------------------------------------------------
// A day of the grid: a click selects it; a click on one of its events or
// tasks selects that too, its card focused in the day stack. Today's cell
// first counts the overdue tasks; a click on the count shows them, at the
// end of today's stack.
function monthCell(d, { events, tasks, overdue }, inMonth, isToday, isSelected) {
    const cell = accEl('div', 'cal__day' + (inMonth ? '' : ' cal__day--out') + (isToday ? ' cal__day--today' : ''));
    cell.setAttribute('aria-selected', String(isSelected));
    cell.addEventListener('click', () => {
        selectDay(d);
    });
    const num = accEl('div', 'cal__num');
    num.innerHTML = `<b>${d.getDate()}</b>`;
    cell.appendChild(num);

    if (overdue) {
        const countEl = calChip(plural(overdue, 'overdue task'), null, () => {
            selectDay(d);
            scrollToOverdue();
        });
        countEl.appendChild(dueTag(`${overdue} overdue`, true));
        cell.appendChild(countEl);
    }

    for (const ev of events) {
        const evEl = calChip(ev.title, ev.doc_id, () => {
            selectDay(d, ev.doc_id);
            scrollIntoView(ev.doc_id);
        });

        evEl.appendChild(iconEl(KIND_ICONS[ev.kind]?.icon || '', 'cal__event-icon'));

        evEl.appendChild(calChipTitle(ev.title));

        if (!ev.all_day) {
            const timeEl = accEl('span', 'cal__event-time', TIME_FORMAT.format(ev.date));
            evEl.appendChild(timeEl);
        }

        cell.appendChild(evEl);
    }

    for (const task of tasks) {
        const taskEl = calChip(task.title, task.doc_id, () => {
            selectDay(d, task.doc_id);
            scrollIntoView(task.doc_id);
        });
        taskEl.append(taskTag(task), calChipTitle(task.title));
        cell.appendChild(taskEl);
    }
    return cell;
}

// ----------------------------------------------------------------------------
// A chip of the grid titled `title` (its tooltip), showing the entity
// `id` (none: the overdue count); a click runs `onClick` instead of
// selecting the cell's day.
function calChip(title, id, onClick) {
    const chip = accEl('div', 'cal__event');
    chip.title = title;
    if (id) chip.dataset.id = id;
    chip.addEventListener('click', e => {
        e.stopPropagation();
        onClick();
    });
    return chip;
}

// ----------------------------------------------------------------------------
function calChipTitle(title) {
    const titleEl = accEl('span', 'cal__event-title', title);
    return titleEl;
}

// ----------------------------------------------------------------------------
// Calendar helpers
function calNav(delta) {
    calFocus = new Date(calFocus.getFullYear(), calFocus.getMonth() + delta, 1);
    renderCalendar();
}

// ----------------------------------------------------------------------------
function calToday() {
    calFocus = new Date();
    renderCalendar();
}

// ----------------------------------------------------------------------------
function calTitleText() {
    return MONTH_YEAR.format(calFocus);
}

// ----------------------------------------------------------------------------
$('cal-next').addEventListener('click', ev => {
    ev.stopPropagation();
    calNav(1);
});

// ----------------------------------------------------------------------------
$('cal-prev').addEventListener('click', ev => {
    ev.stopPropagation();
    calNav(-1);
});

// ----------------------------------------------------------------------------
$('cal-today').addEventListener('click', ev => {
    ev.stopPropagation();
    calToday();
});

registerLens('calendar', calContainerEl, renderCalendarLens);
