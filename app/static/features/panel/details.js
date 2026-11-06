// What a card shows of its event, task, email, document, bank account or
// statement (panel.md#cards):
// the details grid (`renderCardDetails`) with the contacts, the calendar
// and the account as chips that select what they name, a task's fields and a document's
// type editable where the host can write them (`edit_entities`), and a
// task's tag (`taskTag`), which the day view's task rows and the calendar's
// grid show too.

import { accEl } from '../../core/dom.js';
import { DATE_FORMAT, MONTH, WEEKDAY, addDays, fileSize, money, sameDay } from '../../core/format.js';
import { TIME_FORMAT, isoInstant, localDay, relativeDay } from '../../core/format.js';
import { ordinal, t } from '../../core/text.js';
import { entities, parseContact } from '../../model/entities.js';
import { googleKept, googleType, isOverdue } from '../../model/calendar.js';
import { byDay, icalDate, parseSchedule, ruleSupported } from '../../model/schedule.js';
import { editEntityLocal } from '../../model/structure.js';
import { onEditEnd } from '../../ui/field.js';
import { fileIcon, iconEl } from '../../ui/icons.js';
import { accSeg, keyedSeg } from '../../ui/segmented.js';
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
// What a card shows of its entity (features/panel/panel.js): `row`, a `.props`
// grid, filled with its kinds' rows in this order: an event's (`eventFields`,
// `occurrence`'s times on a day card), a task's (`taskFields`, or `taskText`
// while the host can't write it), an email's (`emailFields`), its date (any
// entity's but an event's, whose start it is: `dateField`), a document's type
// and the file it stands for (`originalField`), a bank account's
// (`bankAccountFields`), and a statement's (`statementFields`). A task's
// fields, the date and a document's type are editable while the host can
// write the entity (`taskFields`, `docTypeField`). Rows without a value are left
// out; hidden for an entity with none of them.
//
// Drawn again on every entity change: while one of its fields has the focus,
// that waits until the block is left, so what is typed stays (as a title
// being renamed, ui/field.js); a button that had the focus gets it back.
export function renderCardDetails(row, doc, occurrence = null) {
    if (heldWhileTyping(row, () => renderCardDetails(row, entities.get(doc?.doc_id) ?? doc, occurrence))) return;
    const focusKey = row.contains(document.activeElement) ? document.activeElement.dataset.focusKey : undefined;
    const parts = [];
    const add = (label, ...content) => {
        const dt = accEl('dt', 'label', label);
        const dd = accEl('dd');
        dd.append(...content);
        parts.push(dt, dd);
        return dt;
    };
    if (doc?.event && doc.date) eventFields(doc, add, occurrence);
    if (doc?.task && doc.writable) taskFields(doc, add);
    else if (doc?.task) taskText(doc, add);
    if (doc?.email) emailFields(doc, add);
    if (doc && !doc.event && !isNaN(doc.date)) add(t('props.date'), doc.writable ? dateField(doc) : DATE_FORMAT.format(doc.date));
    if (doc?.kind === 'document' && doc.writable) add(t('props.type'), docTypeField(doc));
    else if (doc?.kind === 'document' && doc.type) add(t('props.type'), doc.type);
    if (doc?.document?.original) add(t('props.file'), ...originalField(doc));
    if (doc?.['bank-account']) bankAccountFields(doc, add);
    if (doc?.statement) statementFields(doc, add);
    row.replaceChildren(...parts);
    row.hidden = !parts.length;
    if (focusKey) row.querySelector(`[data-focus-key="${focusKey}"]`)?.focus({ preventScroll: true });
}

// ----------------------------------------------------------------------------
// An event's rows: when it is (`occurrence`'s times on a day card, and where
// an exception moved it from), how it repeats and where; then the guests, its
// calendar, and a status other than confirmed.
function eventFields(doc, add, occurrence) {
    const { event } = doc;
    const exception = occurrence?.exception;
    add(t('props.when'), whenText(occurrence ?? doc) + movedText(exception, doc.all_day));
    if (event.schedule?.length) add(t('props.schedule'), scheduleText(event.schedule));
    const location = exception?.location ?? event.location;
    if (location) add(t('props.where'), location);
    if (event.guests?.length) add(t('props.guests'), ...event.guests.map(contactChip));
    const calendar = entities.get(event.calendar);
    if (calendar) add(t('props.calendar'), entityChip(event.calendar, calendar.title || t('untitled')), ...accessText(calendar));
    const status = { tentative: t('event.tentative'), cancelled: t('event.cancelled') }[event.status];
    if (status) add(t('props.status'), status);
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
// What becomes of a change to a synced calendar's event in Google (its
// entity's `access`, api.md#events; read-only for a kind of event Google
// keeps as it is, `googleKept`), after the calendar's chip: a word, its
// tooltip saying what it means; and for one Gmail made, that a change
// replaces it.
const CALENDAR_ACCESS = {
    'two-way': [t('event.calendar_two_way'), t('event.calendar_two_way_title')],
    'read-only': [t('event.calendar_read_only'), t('event.calendar_read_only_title')],
    'not-synced': [t('event.calendar_not_synced'), t('event.calendar_not_synced_title')],
};

function accessText(doc, calendar) {
    const [word, means] = googleKept(doc) ? [t('event.calendar_read_only'), t('event.kept_title')]
        : CALENDAR_ACCESS[calendar.access] ?? [];
    const texts = word ? [[word, means]] : [];
    // one Gmail made, which a change replaces in Google
    if (googleType(doc) === 'fromGmail') texts.push([t('event.from_gmail'), t('event.from_gmail_title')]);
    return texts.map(([text, title]) => {
        const el = accEl('span', 'meta', text);
        el.title = title;
        return el;
    });
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
    keyedSeg(seg, TASK_STATUSES, 'status');
    const known = TASK_STATUSES.some(([status]) => status === task.status);
    add(t('props.status'), seg, ...(task.status && !known ? [accEl('span', 'meta', task.status)] : []));

    const hasDue = !isNaN(doc.due);
    const due = fieldInput('date', 'due', hasDue ? localDay(doc.due) : '', 'form-input props__date');
    editField(due, day => edit({ due: day ? dueOn(day, doc) : null }));
    const dueParts = [due];
    if (hasDue) {
        const at = task.due.length === 10 ? '' : `${TIME_FORMAT.format(doc.due)} \u00b7 `;
        dueParts.push(accEl('span', 'meta', at + relativeDay(doc.due)));
        if (isOverdue(doc)) dueParts.push(dueTag(t('task.overdue')));
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
    return sameTimeOn(day, doc.due);
}

// The instant on `day` (yyyy-mm-dd) at the time of day `date` has.
function sameTimeOn(day, date) {
    const [y, m, d] = day.split('-').map(Number);
    const at = new Date(date);
    at.setFullYear(y, m - 1, d);
    return isoInstant(at);
}

// ----------------------------------------------------------------------------
// An entity's date (vault.md#metadata), its day typed in: a scan's or a
// mail's wrong date corrected; its time of day, if it has one, kept.
function dateField(doc) {
    const input = fieldInput('date', 'date', doc.date ? localDay(doc.date) : '', 'form-input props__date');
    editField(input, day => {
        if (day) editEntityLocal(doc.doc_id, { date: doc.all_day || !doc.date ? day : sameTimeOn(day, doc.date) });
    });
    return input;
}

// ----------------------------------------------------------------------------
// A task's fields as text, while the host can't write it.
function taskText(doc, add) {
    const { task } = doc;
    if (task.status) add(t('props.status'), Object.fromEntries(TASK_STATUSES)[task.status] ?? task.status);
    if (!isNaN(doc.due)) {
        const date = task.due.length === 10 ? DATE_FORMAT.format(doc.due)
            : `${DATE_FORMAT.format(doc.due)}, ${TIME_FORMAT.format(doc.due)}`;
        add(t('props.due'), `${date} \u00b7 ${relativeDay(doc.due)}`, ...(isOverdue(doc) ? [dueTag(t('task.overdue'))] : []));
    }
    if (task.assignee) add(t('props.assignee'), contactChip(task.assignee));
    if (!isNaN(doc.done)) add(t('props.done'), DATE_FORMAT.format(doc.done));
}

// ----------------------------------------------------------------------------
// An email's sender and its type (the email analysis's), whom
// it went to and its subject (written only once the title differs from it,
// vault.md#email): read only, as the mail came, each such row's label saying
// so.
const EMAIL_PEOPLE = [['to', t('props.to')], ['cc', t('props.cc')], ['bcc', t('props.bcc')]];

function emailFields(doc, add) {
    const { email } = doc;
    const asItCame = dt => { dt.title = t('email.as_it_came'); };
    if (email.from) asItCame(add(t('props.from'), contactChip(email.from), ...(doc.type ? [` \u00b7 ${doc.type}`] : [])));
    else if (doc.type) add(t('props.type'), doc.type);
    for (const [key, label] of EMAIL_PEOPLE) {
        if (email[key]?.length) asItCame(add(label, ...email[key].map(contactChip)));
    }
    if (email.subject) asItCame(add(t('props.subject'), email.subject));
}

// ----------------------------------------------------------------------------
// A document's type, typed in while the host can write it; the types the
// vault's documents use are offered (`typeList`).
const typeList = accEl('datalist');
typeList.id = 'type-list';
document.body.append(typeList);

function docTypeField(doc) {
    const input = fieldInput('text', 'type', doc.type ?? '', 'form-input');
    input.placeholder = t('type.none');
    input.setAttribute('list', typeList.id);
    input.addEventListener('focus', () => {
        const used = new Set([...entities.values()].map(other => other.kind === 'document' && other.type));
        used.delete(false);
        typeList.replaceChildren(...[...used].sort().map(type => {
            const option = accEl('option');
            option.value = type;
            return option;
        }));
    });
    editField(input, value => editEntityLocal(doc.doc_id, { type: value.trim() || null }));
    return input;
}

// ----------------------------------------------------------------------------
// A bank account's bank, number, balance and holder.
function bankAccountFields(doc, add) {
    const account = doc['bank-account'];
    if (account.bank) add(t('props.bank'), account.bank);
    if (account.number) add(t('props.number'), account.number);
    if (account.balance != null) add(t('props.balance'), money(account.balance, account.currency));
    if (account.holder) add(t('props.holder'), contactChip(account.holder));
}

// ----------------------------------------------------------------------------
// A statement's account, its transactions and the balance it ends with
// (vault.md#statement).
function statementFields(doc, add) {
    const { statement } = doc;
    const holder = entities.get(statement.account);
    if (holder) add(t('props.account'), entityChip(statement.account, holder.title || t('untitled')));
    const transactions = statement.transactions ?? [];
    const pending = transactions.filter(transaction => transaction.status === 'pending').length;
    add(t('props.transactions'), String(transactions.length - pending) + (pending ? ` \u00b7 ${t('statement.pending', { n: pending })}` : ''));
    if (statement.balance != null) add(t('props.balance'), money(statement.balance, holder?.['bank-account']?.currency));
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
// email is the tooltip. The card's and a transaction's (features/panel/transaction.js).
export function contactChip(ref) {
    const { id, name, email } = parseContact(ref);
    if (id) return entityChip(id, name);
    const chip = accEl('span', 'chip', name);
    if (email) chip.title = email;
    return chip;
}

// ----------------------------------------------------------------------------
// The entity `id` as a chip titled `title`; a click selects it.
export function entityChip(id, title) {
    const chip = accEl('span', 'chip');
    const name = accEl('button', 'chip__name', title);
    name.type = 'button';
    name.addEventListener('click', () => selectById(id));
    chip.append(name);
    return chip;
}
