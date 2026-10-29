// What a card shows of its event, task, mail or document (panel.md#cards):
// the details grid (`renderCardDetails`) with the contacts and the calendar
// as chips that select what they name, a task's fields and a document's
// genre editable where the host can write them (`edit_entities`), and a
// task's tag (`taskTag`), which the day view's task rows and the calendar's
// grid show too.

import { accEl } from '../../core/dom.js';
import { DATE_FORMAT, fileSize } from '../../core/format.js';
import { TIME_FORMAT, isoInstant, localDay, relativeDay } from '../../core/format.js';
import { entities } from '../../model/entities.js';
import {
    byDay, icalDate, parseRecurrence, ruleSupported,
} from '../../model/calendar.js';
import { isOverdue } from '../../model/calendar.js';
import { addDays, sameDay } from '../../model/calendar.js';
import { editEntityLocal } from '../../model/structure.js';
import { onEditEnd } from '../../ui/field.js';
import { fileIcon, iconEl } from '../../ui/icons.js';
import { accSeg } from '../../ui/segmented.js';
import { openOriginal, selectById } from '../selection.js';

// ----------------------------------------------------------------------------
// A task's tag: "Due", or "Overdue" and filled -- the only emphasis the grid
// uses.
export function taskTag(doc) {
    const overdue = isOverdue(doc);
    return iconEl(overdue ? '#i-warning-filled' : '#i-warning', 'cal__event-icon');
}

// ----------------------------------------------------------------------------
export function dueTag(text) {
    const tag = accEl('span', 'cal__event-icon', text);
    return tag;
}

// ----------------------------------------------------------------------------
// What a card shows of its event, task, mail or document
// (features/panel/panel.js): `row`, a `.props` grid, filled with when an
// event is (`occurrence`'s times on a day card), how it repeats, where, the
// guests, its calendar, and a status other than confirmed; a task's status,
// when it is due (from today, marked when overdue), its assignee and when
// it was done; a mail's sender and its kind (the mail analysis's,
// `MAIL_KINDS`); a document's genre, and the file it stands for
// (`originalField`). A task's fields and a document's genre are editable
// while the host can write the entity (`taskFields`, `genreField`). Rows
// without a value are left out; hidden for an entity with none of them.
//
// Drawn again on every entity change: while one of its fields has the focus,
// that waits until the block is left, so what is typed stays (as a title
// being renamed, ui/field.js); a button that had the focus gets it back.
export function renderCardDetails(row, doc, occurrence = null) {
    if (heldWhileTyping(row, () => renderCardDetails(row, entities.get(doc?.doc_id) ?? doc, occurrence))) return;
    const focusKey = row.contains(document.activeElement) ? document.activeElement.dataset.focusKey : undefined;
    const event = doc?.event;
    const task = doc?.task;
    const email = doc?.email;
    const parts = [];
    const add = (label, ...content) => {
        const dt = accEl('dt', 'label', label);
        const dd = accEl('dd');
        dd.append(...content);
        parts.push(dt, dd);
    };
    if (event && doc.date) {
        const exception = occurrence?.exception;
        add('When', whenText(occurrence ?? doc) + movedText(exception, doc.all_day));
        if (event.recurrence?.length) add('Repeats', repeatsText(event.recurrence));
        const location = exception?.location ?? event.location;
        if (location) add('Where', location);
        if (event.attendees?.length) add('Guests', ...event.attendees.map(contactChip));
        const calendar = entities.get(event.calendar);
        if (calendar) add('Calendar', entityChip(event.calendar, calendar.title || 'Untitled'));
        const status = { tentative: 'Tentative', cancelled: 'Cancelled' }[event.status];
        if (status) add('Status', status);
    }
    if (task && doc.writable) {
        taskFields(doc, add);
    } else if (task) {
        if (task.status) add('Status', Object.fromEntries(TASK_STATUSES)[task.status] ?? task.status);
        if (!isNaN(doc.due)) {
            const date = task.due.length === 10 ? DATE_FORMAT.format(doc.due)
                : `${DATE_FORMAT.format(doc.due)}, ${TIME_FORMAT.format(doc.due)}`;
            add('Due', `${date} \u00b7 ${relativeDay(doc.due)}`, ...(isOverdue(doc) ? [dueTag('Overdue', true)] : []));
        }
        if (task.assignee) add('Assignee', contactChip(task.assignee));
        if (!isNaN(doc.done)) add('Done', DATE_FORMAT.format(doc.done));
    }
    if (email) {
        const kind = MAIL_KINDS.find(([key]) => key === email.kind)?.[1];
        if (email.from) add('From', contactChip(email.from), ...(kind ? [` \u00b7 ${kind}`] : []));
        else if (kind) add('Kind', kind);
    }
    if (doc?.kind === 'document' && doc.writable) add('Genre', genreField(doc));
    else if (doc?.kind === 'document' && doc.genre) add('Genre', doc.genre);
    if (doc?.document?.original) add('File', ...originalField(doc));
    row.replaceChildren(...parts);
    row.hidden = !parts.length;
    if (focusKey) row.querySelector(`[data-focus-key="${focusKey}"]`)?.focus({ preventScroll: true });
}

// ----------------------------------------------------------------------------
// The details blocks waiting to be drawn again (`renderCardDetails`): one is
// held while an input of it has the focus, and drawn once the block is left.
const heldRenders = new WeakMap(); // block -> its latest render
const watchedBlocks = new WeakSet(); // the blocks listening for that

function typingIn(row) {
    return row.contains(document.activeElement) && document.activeElement.tagName === 'INPUT';
}

// Hold `render` of `row` if an input of it has the focus; whether it did.
function heldWhileTyping(row, render) {
    if (!typingIn(row)) return false;
    heldRenders.set(row, render);
    if (!watchedBlocks.has(row)) {
        watchedBlocks.add(row);
        // where the focus went (perhaps another field of the block) is known
        // only after the event
        row.addEventListener('focusout', () => setTimeout(() => {
            const held = heldRenders.get(row);
            if (!held || typingIn(row)) return;
            heldRenders.delete(row);
            held();
        }));
    }
    return true;
}

// ----------------------------------------------------------------------------
// A task's fields while the host can write it, each written by itself
// (`editEntityLocal`): the status buttons (an unknown status as text after
// them), the due date with when that is, the Overdue tag and an x to remove
// it, the assignee typed in (a contact of the vault shown by its name), and
// when it was done.
function taskFields(doc, add) {
    const { doc_id: id, task } = doc;
    const edit = change => editEntityLocal(id, { task: change });

    const seg = accSeg(TASK_STATUSES, task.status, status => edit({ status }));
    [...seg.children].forEach((option, i) => { option.dataset.focusKey = `status:${TASK_STATUSES[i][0]}`; });
    const known = TASK_STATUSES.some(([status]) => status === task.status);
    add('Status', seg, ...(task.status && !known ? [accEl('span', 'meta', task.status)] : []));

    const hasDue = !isNaN(doc.due);
    const due = fieldInput('date', 'due', hasDue ? localDay(doc.due) : '', 'form-input props__date');
    editField(due, day => edit({ due: day ? dueOn(day, doc) : null }));
    const dueParts = [due];
    if (hasDue) {
        const at = task.due.length === 10 ? '' : `${TIME_FORMAT.format(doc.due)} \u00b7 `;
        dueParts.push(accEl('span', 'meta', at + relativeDay(doc.due)));
        if (isOverdue(doc)) dueParts.push(dueTag('Overdue', true));
        const clear = accEl('button', 'tool');
        clear.type = 'button';
        clear.append(iconEl('#i-dismiss'));
        clear.title = 'No due date';
        clear.dataset.focusKey = 'due:clear';
        clear.addEventListener('click', () => edit({ due: null }));
        dueParts.push(clear);
    }
    add('Due', ...dueParts);

    const name = task.assignee ? parseContact(task.assignee).name : '';
    const assignee = fieldInput('text', 'assignee', name, 'form-input');
    assignee.placeholder = 'Nobody';
    editField(assignee, value => edit({ assignee: value.trim() || null }));
    add('Assignee', assignee);

    if (!isNaN(doc.done)) add('Done', DATE_FORMAT.format(doc.done));
}

// The due date `day` (yyyy-mm-dd) of the task `doc`, at the time of day its
// due date has, if it has one.
function dueOn(day, doc) {
    if (doc.task.due?.length === 10 || isNaN(doc.due)) return day;
    const [y, m, d] = day.split('-').map(Number);
    const at = new Date(doc.due);
    at.setFullYear(y, m - 1, d);
    return isoInstant(at);
}

// ----------------------------------------------------------------------------
// A document's genre, typed in while the host can write it; the genres the
// vault's documents use are offered (`genreList`).
const genreList = accEl('datalist');
genreList.id = 'genre-list';
document.body.append(genreList);

function genreField(doc) {
    const input = fieldInput('text', 'genre', doc.genre ?? '', 'form-input');
    input.placeholder = 'None';
    input.setAttribute('list', genreList.id);
    input.addEventListener('focus', () => {
        const used = new Set([...entities.values()].map(other => other.kind === 'document' && other.genre));
        used.delete(false);
        genreList.replaceChildren(...[...used].sort().map(genre => {
            const option = accEl('option');
            option.value = genre;
            return option;
        }));
    });
    editField(input, value => editEntityLocal(doc.doc_id, { genre: value.trim() || null }));
    return input;
}

// ----------------------------------------------------------------------------
// The file a document stands for (vault.md#document; a mail's attachment):
// its type's icon, the name it came with and its size, and Open, which
// opens it in its app (`openOriginal`).
function originalField(doc) {
    const original = doc.document.original;
    const name = original.name ?? original.file ?? '';
    const size = original.size != null ? ` \u00b7 ${fileSize(original.size)}` : '';
    const open = accEl('button', 'icon-btn icon-btn--quiet');
    open.type = 'button';
    open.title = 'Open in its app';
    open.dataset.focusKey = 'open-file';
    open.append(iconEl('#i-open'));
    open.addEventListener('click', () => openOriginal(doc.doc_id));
    return [iconEl(fileIcon(name)), `${name}${size}`, open];
}

// ----------------------------------------------------------------------------
// An input of the block, of `type`, holding `value`; `key` finds it again
// once the block is drawn anew.
function fieldInput(type, key, value, className) {
    const input = accEl('input', className);
    input.type = type;
    input.value = value;
    input.dataset.focusKey = key;
    return input;
}

// A field of the block: Enter or leaving it writes what it holds, if that
// changed (`write(value)`); Esc puts back what it held (ui/field.js's
// `onEditEnd`). A date input's change fires for every digit typed, so it
// isn't what writes.
function editField(input, write) {
    const arm = () => {
        const before = input.value;
        onEditEnd(input, save => {
            if (!save) input.value = before;
            else if (input.value !== before) write(input.value);
            if (document.activeElement === input) arm();
        });
    };
    input.addEventListener('focus', arm);
}

// ----------------------------------------------------------------------------
// A task's statuses, as the vault keeps them and in words.
export const TASK_STATUSES = [['todo', 'To do'], ['in-progress', 'In progress'], ['waiting', 'Waiting'], ['done', 'Done']];

// ----------------------------------------------------------------------------
// The kinds of mail the mail analysis finds (vault.md#email), in words;
// `other` goes unnamed.
const MAIL_KINDS = [['request', 'Request'], ['reply', 'Reply'], ['notification', 'Notification'], ['newsletter', 'Newsletter']];

// ----------------------------------------------------------------------------
// When `ev` is: all day, its date or "date - date" (`end` is the day after
// the last); else "date, 10:00 - 11:00", or with both dates across days.
function whenText(ev) {
    const date = d => DATE_FORMAT.format(d);
    const time = d => TIME_FORMAT.format(d);
    const start = ev.date;
    if (ev.all_day) {
        const days = ev.end ? Math.round((ev.end - start) / 864e5) : 1;
        return days > 1 ? `${date(start)} \u2013 ${date(addDays(start, days - 1))}` : date(start);
    }
    if (!ev.end) return `${date(start)}, ${time(start)}`;
    if (sameDay(start, ev.end)) return `${date(start)}, ${time(start)} \u2013 ${time(ev.end)}`;
    return `${date(start)} ${time(start)} \u2013 ${date(ev.end)} ${time(ev.end)}`;
}

// ----------------------------------------------------------------------------
// " - moved from" where an occurrence's `exception` moved it from: the
// time on the same day, else the day; empty if it wasn't moved.
function movedText(exception, allDay) {
    const { start, original } = exception ?? {};
    if (!start || start.getTime() === original.getTime()) return '';
    const from = !allDay && sameDay(start, original) ? TIME_FORMAT.format(original) : DATE_FORMAT.format(original);
    return ` \u00b7 moved from ${from}`;
}

// ----------------------------------------------------------------------------
// A series' rule in words, e.g. "Every 2 weeks on Mon, Wed, 10 times"; one
// the page doesn't expand, or that mixes BYDAY and BYMONTHDAY, as written
// (a card, a row of the Timeline lens).
const RRULE_UNITS = { DAILY: 'day', WEEKLY: 'week', MONTHLY: 'month', YEARLY: 'year' };
const WEEKDAY_NAMES = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTH_NAMES = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

// ----------------------------------------------------------------------------
export function repeatsText(lines) {
    const { rule, written } = parseRecurrence(lines);
    if (!ruleSupported(rule) || (rule.BYDAY && rule.BYMONTHDAY)) return written ?? lines.join(' ');
    const unit = RRULE_UNITS[rule.FREQ];
    const interval = Math.max(1, Number(rule.INTERVAL) || 1);
    let text = interval === 1 ? `Every ${unit}` : `Every ${interval} ${unit}s`;
    const list = (value, name) => value.split(',').map(name).join(', ');
    if (rule.BYMONTH) text += ` in ${list(rule.BYMONTH, m => MONTH_NAMES[Number(m) - 1] ?? m)}`;
    if (rule.BYDAY) text += ` on ${list(rule.BYDAY, byDayText)}`;
    if (rule.BYMONTHDAY) text += ` on ${list(rule.BYMONTHDAY, monthDayText)}`;
    if (rule.COUNT) text += `, ${rule.COUNT} times`;
    if (rule.UNTIL) text += `, until ${DATE_FORMAT.format(icalDate(rule.UNTIL, true))}`;
    return text;
}

// ----------------------------------------------------------------------------
// A BYDAY entry in words: "Mon", "the 2nd Tue", "the last Fri", "the 2nd
// last Fri".
function byDayText(entry) {
    const { weekday, nth } = byDay(entry);
    const name = WEEKDAY_NAMES[weekday] ?? entry;
    if (!nth) return name;
    if (nth === -1) return `the last ${name}`;
    return nth > 0 ? `the ${ordinal(nth)} ${name}` : `the ${ordinal(-nth)} last ${name}`;
}

// ----------------------------------------------------------------------------
// A BYMONTHDAY entry in words: "day 15", "the last day", "the 2nd last day".
function monthDayText(entry) {
    const n = Number(entry);
    if (n === -1) return 'the last day';
    return n < 0 ? `the ${ordinal(-n)} last day` : `day ${n}`;
}

// ----------------------------------------------------------------------------
function ordinal(n) {
    const suffix = n % 100 >= 11 && n % 100 <= 13 ? 'th' : { 1: 'st', 2: 'nd', 3: 'rd' }[n % 10] ?? 'th';
    return `${n}${suffix}`;
}

// ----------------------------------------------------------------------------
// A contact as the vault keeps it (a guest, an assignee): a contact of the
// vault (`@id`) as its id and title, "Name <email>" as the name and the
// email, else as written.
export function parseContact(ref) {
    const id = /^@(.+)$/.exec(ref)?.[1];
    if (id && entities.has(id)) return { id, name: entities.get(id).title || 'Untitled' };
    const named = /^(.+?)\s*<([^<>]+)>$/.exec(ref);
    return named ? { name: named[1], email: named[2] } : { name: ref };
}

// ----------------------------------------------------------------------------
// A contact as a chip by its name: the vault's selects it on a click, an
// email is the tooltip.
function contactChip(ref) {
    const { id, name, email } = parseContact(ref);
    if (id) return entityChip(id, name);
    const chip = accEl('span', 'chip', name);
    if (email) chip.title = email;
    return chip;
}

// ----------------------------------------------------------------------------
// The entity `id` as a chip titled `title`; a click selects it.
function entityChip(id, title) {
    const chip = accEl('span', 'chip');
    const name = accEl('button', 'chip__name', title);
    name.type = 'button';
    name.addEventListener('click', () => selectById(id));
    chip.append(name);
    return chip;
}
