// What a card shows of its event, task, email, document, bank account,
// statement or recurring payment (panel.md#cards):
// the details grid (`renderCardDetails`) with the contacts, the calendar
// and the account as chips that select what they name, a task's fields, a
// document's type, a trip event's fields (features/panel/trip-fields.js)
// and a work calendar's event's Work editable where the host can write them
// (`edit_entities`), a recurring payment's too (`set_recurring_payment`), and a
// task's tag (`taskTag`), which the day view's task rows and the calendar's
// grid show too.

import { accEl } from '../../core/dom.js';
import { DATE_FORMAT, amountText, fileSize, money, parseAmount, parseDate } from '../../core/format.js';
import { TIME_FORMAT, isoInstant, localDay, relativeDay } from '../../core/format.js';
import { entities, parseContact } from '../../model/entities.js';
import { googleKept, googleLocked, googleType, isOverdue } from '../../model/calendar.js';
import { addDays, sameDay } from '../../core/format.js';
import { t } from '../../core/text.js';
import { PERIODS, eventRuleOf, eventScheduleLines, periodOf, scheduleLine } from '../../model/schedule.js';
import {
    CATEGORIES, amountsOf, nextOccurrence, withAmount,
} from '../../model/bank.js';
import { editEntityLocal, setRecurringPaymentLocal } from '../../model/structure.js';
import { LOCAL_ZONE, isTripEvent } from '../../model/trip.js';
import { editField, fieldInput } from '../../ui/field.js';
import { fileIcon, iconEl } from '../../ui/icons.js';
import { accSeg, keyedSeg } from '../../ui/segmented.js';
import { openOriginal, selectById } from '../selection.js';
import { plainEventFields, statusField } from './event-fields.js';
import { eventScheduleField, scheduleField, scheduleText } from './schedule-field.js';
import { eventTypeField, tripFields } from './trip-fields.js';
import { showMenu } from '../../ui/menu.js';

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
// (`bankAccountFields`), a statement's (`statementFields`), and a recurring
// payment's (`recurringPaymentFields`). A task's fields, the date, a
// document's type and an event's Work are editable while the host can write
// the entity (`taskFields`, `docTypeField`, `workField`). Rows without a value
// are left out; hidden for an entity with none of them.
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
    if (doc?.['recurring-payment']) recurringPaymentFields(doc, add);
    row.replaceChildren(...parts);
    row.hidden = !parts.length;
    if (focusKey) row.querySelector(`[data-focus-key="${focusKey}"]`)?.focus({ preventScroll: true });
}

// ----------------------------------------------------------------------------
// An event's rows: its Type (`eventTypeField`), when it is (`occurrence`'s times on
// a day card, and where an exception moved it from), how it repeats and where,
// or a trip event's fields in their place (`tripFields`, but for a recurring
// event); then the guests, its calendar, in a work calendar whether it is a
// day off or a remote one (`workField`), and a status other than confirmed,
// but a trip's, whose fields hold it.
function eventFields(doc, add, occurrence) {
    const { event } = doc;
    const trip = isTripEvent(doc) && !event.schedule?.length;
    const editable = !trip && doc.writable && !googleLocked(doc);
    const edit = change => editEntityLocal(doc.doc_id, { event: change });
    if (!trip && doc.writable) add(t('props.type'), eventTypeField(event, edit));
    if (trip) {
        tripFields(doc, add);
    } else if (editable) {
        // a recurring event's occurrence on a day card, then the series
        if (occurrence?.occurrence) add(t('props.when'), whenText(occurrence) + movedText(occurrence.exception, doc.all_day));
        plainEventFields(doc, add, edit);
        add(t('props.schedule'), scheduleRow(doc, edit));
    } else {
        const exception = occurrence?.exception;
        add(t('props.when'), whenText(occurrence ?? doc) + movedText(exception, doc.all_day));
        if (event.schedule?.length) add(t('props.schedule'), scheduleText(event.schedule));
        const location = exception?.location ?? event.location;
        if (location) add(t('props.where'), location);
    }
    if (event.guests?.length) add(t('props.guests'), ...event.guests.map(contactChip));
    const calendar = entities.get(event.calendar);
    const move = doc.writable && !googleLocked(doc) ? moveButton(doc, edit) : null;
    if (calendar) add(t('props.calendar'), entityChip(event.calendar, calendar.title || t('untitled')), ...accessText(doc, calendar), ...(move ? [move] : []));
    else if (move) add(t('props.calendar'), move);
    const work = calendar?.calendar?.['work-of'] || event.work ? workField(doc) : null;
    if (work) add(t('props.work'), ...work);
    const status = { tentative: t('event.tentative'), cancelled: t('event.cancelled') }[event.status];
    if (editable && event.status !== 'cancelled') add(t('props.status'), statusField(event, edit));
    else if (status && !trip) add(t('props.status'), status);
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
// Move: a menu of the calendars the event may go to, as the host allows
// (`calendar::move_refused`): the other two-way ones of its calendar's
// Google account, or any two-way one for an event in none. None to go to,
// no button.
function moveButton(doc, edit) {
    const current = entities.get(doc.event.calendar);
    // Google moves a plain event only, not one Gmail made
    if ((current && current.access !== 'two-way') || googleType(doc)) return null;
    const account = current?.source?.remote?.account;
    const targets = [...entities.values()].filter(other => other.kind === 'calendar' && other.access === 'two-way'
        && other.doc_id !== current?.doc_id && (!current || other.source?.remote?.account === account));
    if (!targets.length) return null;
    const btn = accEl('button', 'tool label', t('event.move'));
    btn.type = 'button';
    btn.dataset.focusKey = 'calendar:move';
    btn.addEventListener('click', () => showMenu(btn, targets.map(other => ({
        label: other.title || t('untitled'), run: () => edit({ calendar: other.doc_id }),
    })), { head: t('event.move_head') }));
    return btn;
}

// ----------------------------------------------------------------------------
// A plain event's schedule (`eventScheduleField`), its new lines with the
// zone a timed event repeats in (`tz`, this PC's if it had none); one
// the field doesn't write, in words.
function scheduleRow(doc, edit) {
    const { event } = doc;
    const zone = doc.all_day ? null : event.tz ?? LOCAL_ZONE;
    const rule = eventRuleOf(event.schedule, doc.date, zone);
    if (!rule) return scheduleText(event.schedule);
    return eventScheduleField(rule, {
        from: doc.date, key: 'schedule', bind: editField,
        onChange: next => {
            const lines = eventScheduleLines(next, doc.date, zone, doc.all_day);
            edit({ schedule: lines.length ? lines : null, ...(zone && !event.tz ? { tz: zone } : {}) });
        },
    });
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
// An event's Work in a work calendar (`work`, vault.md#event), while the host
// can write it: Off or Remote, another value as text after them, and an x
// that makes it neither (a meeting, say), written by itself
// (`editEntityLocal`); else as text, none without a value.
const WORK_DAYS = [['off', t('work.off')], ['remote', t('work.remote')]];

function workField(doc) {
    const { doc_id: id, event } = doc;
    const words = Object.fromEntries(WORK_DAYS)[event.work];
    if (!doc.writable) return event.work ? [words ?? event.work] : null;
    const edit = work => editEntityLocal(id, { event: { work } });
    const seg = accSeg(WORK_DAYS, event.work ?? null, edit);
    keyedSeg(seg, WORK_DAYS, 'work');
    const parts = [seg];
    if (event.work && !words) parts.push(accEl('span', 'meta', event.work));
    if (event.work) {
        const clear = accEl('button', 'tool');
        clear.type = 'button';
        clear.append(iconEl('#i-dismiss'));
        clear.title = t('work.neither');
        clear.dataset.focusKey = 'work:clear';
        clear.addEventListener('click', () => edit(null));
        parts.push(clear);
    }
    return parts;
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
// A recurring payment's fields (vault.md#recurring): its account, then while
// the host can write it its party, schedule (a period and a week's days or
// its day, as `scheduleLine` writes them, `scheduleField`; another schedule
// in words), first and last day,
// current amount (since when, and the earlier ones), how far a transaction may
// vary and its category, each writing the whole role (`setRecurringPaymentLocal`);
// else as text. Then its next occurrence after the statements.
function recurringPaymentFields(doc, add) {
    const { doc_id: id, 'recurring-payment': recurring } = doc;
    const account = entities.get(recurring.account);
    if (account) {
        add(t('props.account'), entityChip(account.id, account.title || t('untitled')),
            accEl('span', 'meta', account['bank-account']?.number ?? ''));
    }
    const amounts = amountsOf(recurring);
    const current = amounts.at(-1);
    const day = text => DATE_FORMAT.format(parseDate(text));
    const earlier = amounts.slice(0, -1).reverse()
        .map(entry => t('recurring.amount_from', { amount: amountText(entry.amount), day: day(entry.from) })).join(', ');
    const sinceText = current && t('recurring.since', { day: day(current.from) })
        + (earlier ? ` \u00b7 ${t('recurring.before', { earlier })}` : '');
    const since = current ? [accEl('span', 'meta', sinceText)] : [];
    const schedule = periodOf(recurring.schedule);
    if (!doc.writable) {
        if (recurring.party) add(t('props.party'), contactChip(recurring.party));
        add(t('props.schedule'), scheduleText(recurring.schedule ?? []));
        if (recurring.from) add(t('props.from'), day(recurring.from));
        if (recurring.until) add(t('props.until'), day(recurring.until));
        if (current) add(t('props.amount'), amountText(current.amount), ' ', ...since);
        if (recurring.vary != null) add(t('props.varies'), amountText(recurring.vary));
        if (recurring.category) add(t('props.category'), recurring.category);
    } else {
        // the role with `change`, a key whose value is null left out
        const write = change => {
            const next = { ...recurring, ...change };
            for (const key of Object.keys(next)) if (next[key] == null) delete next[key];
            setRecurringPaymentLocal(id, next);
        };
        const party = fieldInput('text', 'party', recurring.party ? parseContact(recurring.party).name : '', 'form-input');
        party.placeholder = t('recurring.party_placeholder');
        editField(party, value => write({ party: value.trim() || null }));
        add(t('props.party'), party);

        if (schedule) {
            const from = parseDate(recurring.from) ?? new Date();
            add(t('props.schedule'), scheduleField(schedule, {
                periods: PERIODS, from, key: 'schedule', bind: editField,
                onChange: rule => write({ schedule: [scheduleLine(rule, from)] }),
            }));
        } else {
            add(t('props.schedule'), scheduleText(recurring.schedule ?? []));
        }

        const from = fieldInput('date', 'from', recurring.from ? localDay(parseDate(recurring.from)) : '', 'form-input props__date');
        editField(from, value => { if (value) write({ from: value }); });
        add(t('props.from'), from);
        const until = fieldInput('date', 'until', recurring.until ? localDay(parseDate(recurring.until)) : '', 'form-input props__date');
        editField(until, value => write({ until: value || null }));
        add(t('props.until'), until, ...(recurring.until ? [] : [accEl('span', 'meta', t('recurring.open_ended'))]));

        const amount = fieldInput('text', 'amount', current ? amountText(current.amount) : '', 'form-input recurring-payment-amount');
        editField(amount, value => {
            const n = parseAmount(value);
            const day = localDay(parseDate(current?.from ?? recurring.from));
            if (n != null) write({ amounts: withAmount(recurring, day, n).amounts });
        });
        add(t('props.amount'), amount, ...since);

        const vary = fieldInput('text', 'vary', recurring.vary != null ? amountText(recurring.vary) : '', 'form-input recurring-payment-amount');
        vary.placeholder = amountText(0);
        editField(vary, value => {
            const n = value.trim() ? parseAmount(value) : null;
            if (n == null && value.trim()) return;
            write({ vary: n == null ? null : Math.abs(n) });
        });
        add(t('props.varies'), vary, accEl('span', 'meta', t('recurring.vary_note')));

        const category = fieldInput('text', 'category', recurring.category ?? '', 'form-input');
        category.placeholder = t('category.none');
        offerCategories(category);
        editField(category, value => write({ category: value.trim() || null }));
        add(t('props.category'), category);
    }
    const next = nextOccurrence(doc);
    if (next) add(t('props.next'), DATE_FORMAT.format(next), ' ', accEl('span', 'meta', t('recurring.after_statements')));
}

// `input` offers the categories of a recurring payment: the app's (`CATEGORIES`) and
// those the vault's recurring payments use.
const categoryList = accEl('datalist');
categoryList.id = 'category-list';
document.body.append(categoryList);

export function offerCategories(input) {
    input.setAttribute('list', categoryList.id);
    input.addEventListener('focus', () => {
        const used = new Set(CATEGORIES);
        for (const other of entities.values()) if (other['recurring-payment']?.category) used.add(other['recurring-payment'].category);
        categoryList.replaceChildren(...[...used].sort().map(name => {
            const option = accEl('option');
            option.value = name;
            return option;
        }));
    });
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
