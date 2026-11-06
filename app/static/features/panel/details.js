// What a card shows of its event, task, email, document, bank account or
// statement (panel.md#cards):
// the details grid (`renderCardDetails`) with the contacts, the calendar
// and the account as chips that select what they name, a task's fields and a document's
// genre editable where the host can write them (`edit_entities`), and a
// task's tag (`taskTag`), which the day view's task rows and the calendar's
// grid show too.

import { accEl } from '../../core/dom.js';
import { DATE_FORMAT, MONTH, WEEKDAY, addDays, fileSize, money, sameDay } from '../../core/format.js';
import { TIME_FORMAT, isoInstant, localDay, relativeDay } from '../../core/format.js';
import { ordinal, t } from '../../core/text.js';
import { entities, parseContact } from '../../model/entities.js';
import { isOverdue } from '../../model/calendar.js';
import { byDay, icalDate, parseSchedule, ruleSupported } from '../../model/schedule.js';
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
// What a card shows of its event, task, email or document
// (features/panel/panel.js): `row`, a `.props` grid, filled with when an
// event is (`occurrence`'s times on a day card), how it repeats, where, the
// guests, its calendar, and a status other than confirmed; a task's status,
// when it is due (from today, marked when overdue), its assignee and when
// it was done; an email's sender and its kind (the email analysis's,
// `EMAIL_KINDS`); a document's genre, and the file it stands for
// (`originalField`); a bank account's bank, number, balance and owner; a
// statement's account, its bookings and the balance it ends with
// (vault.md#statement). A task's fields and a document's genre are editable
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
        add(t('props.when'), whenText(occurrence ?? doc) + movedText(exception, doc.all_day));
        if (event.schedule?.length) add(t('props.schedule'), scheduleText(event.schedule));
        const location = exception?.location ?? event.location;
        if (location) add(t('props.where'), location);
        if (event.attendees?.length) add(t('props.guests'), ...event.attendees.map(contactChip));
        const calendar = entities.get(event.calendar);
        if (calendar) add(t('props.calendar'), entityChip(event.calendar, calendar.title || t('untitled')));
        const status = { tentative: t('event.tentative'), cancelled: t('event.cancelled') }[event.status];
        if (status) add(t('props.status'), status);
    }
    if (task && doc.writable) {
        taskFields(doc, add);
    } else if (task) {
        if (task.status) add(t('props.status'), Object.fromEntries(TASK_STATUSES)[task.status] ?? task.status);
        if (!isNaN(doc.due)) {
            const date = task.due.length === 10 ? DATE_FORMAT.format(doc.due)
                : `${DATE_FORMAT.format(doc.due)}, ${TIME_FORMAT.format(doc.due)}`;
            add(t('props.due'), `${date} \u00b7 ${relativeDay(doc.due)}`, ...(isOverdue(doc) ? [dueTag(t('task.overdue'), true)] : []));
        }
        if (task.assignee) add(t('props.assignee'), contactChip(task.assignee));
        if (!isNaN(doc.done)) add(t('props.done'), DATE_FORMAT.format(doc.done));
    }
    if (email) {
        const kind = EMAIL_KINDS.find(([key]) => key === email.kind)?.[1];
        if (email.from) add(t('props.from'), contactChip(email.from), ...(kind ? [` \u00b7 ${kind}`] : []));
        else if (kind) add(t('props.kind'), kind);
    }
    if (doc?.kind === 'document' && doc.writable) add(t('props.genre'), genreField(doc));
    else if (doc?.kind === 'document' && doc.genre) add(t('props.genre'), doc.genre);
    if (doc?.document?.original) add(t('props.file'), ...originalField(doc));
    const account = doc?.['bank-account'];
    if (account) {
        if (account.institution) add(t('props.bank'), account.institution);
        if (account.number) add(t('props.number'), account.number);
        if (account.balance != null) add(t('props.balance'), money(account.balance, account.currency));
        if (account.owner) add(t('props.owner'), contactChip(account.owner));
    }
    const statement = doc?.statement;
    if (statement) {
        const holder = entities.get(statement.account);
        if (holder) add(t('props.account'), entityChip(statement.account, holder.title || t('untitled')));
        const bookings = statement.bookings ?? [];
        const pending = bookings.filter(booking => booking.status === 'pending').length;
        add(t('props.bookings'), String(bookings.length - pending) + (pending ? ` \u00b7 ${t('statement.pending', { n: pending })}` : ''));
        if (statement.balance != null) add(t('props.balance'), money(statement.balance, holder?.['bank-account']?.currency));
    }
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
    add(t('props.status'), seg, ...(task.status && !known ? [accEl('span', 'meta', task.status)] : []));

    const hasDue = !isNaN(doc.due);
    const due = fieldInput('date', 'due', hasDue ? localDay(doc.due) : '', 'form-input props__date');
    editField(due, day => edit({ due: day ? dueOn(day, doc) : null }));
    const dueParts = [due];
    if (hasDue) {
        const at = task.due.length === 10 ? '' : `${TIME_FORMAT.format(doc.due)} \u00b7 `;
        dueParts.push(accEl('span', 'meta', at + relativeDay(doc.due)));
        if (isOverdue(doc)) dueParts.push(dueTag(t('task.overdue'), true));
        const clear = accEl('button', 'tool');
        clear.type = 'button';
        clear.append(iconEl('#i-dismiss'));
        clear.title = t('task.no_due');
        clear.dataset.focusKey = 'due:clear';
        clear.addEventListener('click', () => edit({ due: null }));
        dueParts.push(clear);
    }
    add(t('props.due'), ...dueParts);

    const name = task.assignee ? parseContact(task.assignee).name : '';
    const assignee = fieldInput('text', 'assignee', name, 'form-input');
    assignee.placeholder = t('task.nobody');
    editField(assignee, value => edit({ assignee: value.trim() || null }));
    add(t('props.assignee'), assignee);

    if (!isNaN(doc.done)) add(t('props.done'), DATE_FORMAT.format(doc.done));
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
    input.placeholder = t('genre.none');
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
// The file a document stands for (vault.md#document; an email's attachment):
// its type's icon, the name it came with, its size and whether it is an
// encrypted PDF, and Open, which opens it in its app (`openOriginal`).
function originalField(doc) {
    const original = doc.document.original;
    const name = original.name ?? original.file ?? '';
    const size = original.size != null ? ` \u00b7 ${fileSize(original.size)}` : '';
    const encrypted = original.encrypted ? ` \u00b7 ${t('file.encrypted')}` : '';
    const open = accEl('button', 'icon-btn icon-btn--quiet');
    open.type = 'button';
    open.title = t('file.open');
    open.dataset.focusKey = 'open-file';
    open.append(iconEl('#i-open'));
    open.addEventListener('click', () => openOriginal(doc.doc_id));
    return [iconEl(fileIcon(name)), `${name}${size}${encrypted}`, open];
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
export const TASK_STATUSES = [
    ['todo', t('task.todo')], ['in-progress', t('task.in_progress')],
    ['waiting', t('task.waiting')], ['done', t('task.done')],
];

// ----------------------------------------------------------------------------
// The kinds of email the email analysis finds (vault.md#email), in words;
// `other` goes unnamed.
const EMAIL_KINDS = [
    ['request', t('email_kind.request')], ['reply', t('email_kind.reply')],
    ['notification', t('email_kind.notification')], ['newsletter', t('email_kind.newsletter')],
];

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
    return ` \u00b7 ${t('event.moved_from', { from })}`;
}

// ----------------------------------------------------------------------------
// A recurring payment's rule in words, e.g. "Every 2 weeks on Mon, Wed, 10 times"; one
// the page doesn't expand, or that mixes BYDAY and BYMONTHDAY, as written
// (a card, a row of the Timeline lens).
const EVERY = {
    DAILY: n => t('schedule.every_day', { n }),
    WEEKLY: n => t('schedule.every_week', { n }),
    MONTHLY: n => t('schedule.every_month', { n }),
    YEARLY: n => t('schedule.every_year', { n }),
};
// Sunday first, as RRULE's days are counted (4 Jan 2026 was a Sunday).
const WEEKDAY_NAMES = [...Array(7)].map((_, i) => WEEKDAY.format(new Date(2026, 0, 4 + i)));
const MONTH_NAMES = [...Array(12)].map((_, i) => MONTH.format(new Date(2026, i, 1)));

// ----------------------------------------------------------------------------
export function scheduleText(lines) {
    const { rule, written } = parseSchedule(lines);
    if (!ruleSupported(rule) || (rule.BYDAY && rule.BYMONTHDAY)) return written ?? lines.join(' ');
    const interval = Math.max(1, Number(rule.INTERVAL) || 1);
    let text = EVERY[rule.FREQ](interval);
    const list = (value, name) => value.split(',').map(name).join(', ');
    if (rule.BYMONTH) text += t('schedule.in_months', { months: list(rule.BYMONTH, m => MONTH_NAMES[Number(m) - 1] ?? m) });
    if (rule.BYDAY) text += t('schedule.on_days', { days: list(rule.BYDAY, byDayText) });
    if (rule.BYMONTHDAY) text += t('schedule.on_days', { days: list(rule.BYMONTHDAY, monthDayText) });
    if (rule.COUNT) text += t('schedule.times', { n: Number(rule.COUNT) });
    if (rule.UNTIL) text += t('schedule.until', { date: DATE_FORMAT.format(icalDate(rule.UNTIL, true)) });
    return text;
}

// ----------------------------------------------------------------------------
// A BYDAY entry in words: "Mon", "the 2nd Tue", "the last Fri", "the 2nd
// last Fri".
function byDayText(entry) {
    const { weekday, nth } = byDay(entry);
    const name = WEEKDAY_NAMES[weekday] ?? entry;
    if (!nth) return name;
    if (nth === -1) return t('schedule.last_weekday', { day: name });
    return nth > 0 ? t('schedule.nth_weekday', { nth: ordinal(nth), day: name })
        : t('schedule.nth_last_weekday', { nth: ordinal(-nth), day: name });
}

// ----------------------------------------------------------------------------
// A BYMONTHDAY entry in words: "day 15", "the last day", "the 2nd last day".
function monthDayText(entry) {
    const n = Number(entry);
    if (n === -1) return t('schedule.last_day');
    return n < 0 ? t('schedule.nth_last_day', { nth: ordinal(-n) }) : t('schedule.month_day', { n });
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
