// What a card shows of its event, task, email, document, bank account,
// statement or recurring payment (panel.md#cards):
// the details grid (`renderCardDetails`) with the contacts, the calendar
// and the account as chips that select what they name, a task's fields, a
// document's genre, a trip event's fields (features/panel/trip-fields.js),
// a work calendar's event's Work, a record's own fields and every entity's
// classification editable where the host can write them (`edit_entities`),
// a recurring payment's too (`set_recurring_payment`), and a task's tag
// (`taskTag`), which the day view's task rows and the calendar's grid show
// too.

import { accEl } from '../../core/dom.js';
import { DATE_FORMAT, amountText, fileSize, money, parseAmount, parseDate } from '../../core/format.js';
import { TIME_FORMAT, isoInstant, localDay, relativeDay } from '../../core/format.js';
import { entities, parseContact, vaultSettings } from '../../model/entities.js';
import {
    byDay, icalDate, parseSchedule, ruleSupported,
} from '../../model/schedule.js';
import { isOverdue } from '../../model/calendar.js';
import { MONTH, WEEKDAY, addDays, sameDay } from '../../core/format.js';
import { ordinal, t } from '../../core/text.js';
import { PERIODS, scheduleLine, periodOf } from '../../model/schedule.js';
import {
    CATEGORIES, amountsOf, nextOccurrence, withAmount,
} from '../../model/bank.js';
import { editEntityLocal, setFieldsLocal, setRecurringPaymentLocal } from '../../model/structure.js';
import { isTripEvent } from '../../model/trip.js';
import { editField, fieldInput } from '../../ui/field.js';
import { fileIcon, iconEl } from '../../ui/icons.js';
import { showMenu } from '../../ui/menu.js';
import { accSeg } from '../../ui/segmented.js';
import { CLASSIFICATIONS } from '../workspace/models.js';
import { openOriginal, selectById } from '../selection.js';
import { meetsField, tripFields } from './trip-fields.js';

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
// guests, its calendar, in a work calendar whether it is a day off or a
// remote one (`workField`), and a status other than confirmed -- a trip's
// event its trip fields instead of when, where and the status
// (`tripFields`, but for a recurring event); a task's status,
// when it is due (from today, marked when overdue), its assignee and when
// it was done; an email's sender and its kind (the email analysis's,
// `EMAIL_KINDS`); a document's genre, and the file it stands for
// (`originalField`); a bank account's bank, number, balance and owner; a
// statement's account, its bookings and the balance it ends with
// (vault.md#statement); a record's own fields (`recordFields`); and
// which models may read it (`classificationField`). A task's fields, a
// document's genre, an event's Work, a record's fields and the
// classification are editable while the host can write the entity
// (`taskFields`, `genreField`, `workField`). Rows without a value are left
// out; hidden for an entity with none of them.
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
        const trip = isTripEvent(doc) && !event.schedule?.length;
        if (trip) {
            tripFields(doc, add);
        } else {
            const exception = occurrence?.exception;
            add(t('props.when'), whenText(occurrence ?? doc) + movedText(exception, doc.all_day));
            if (event.schedule?.length) add(t('props.schedule'), scheduleText(event.schedule));
            const location = exception?.location ?? event.location;
            if (location) add(t('props.where'), location);
        }
        if (event.attendees?.length) add(t('props.guests'), ...event.attendees.map(contactChip));
        const calendar = entities.get(event.calendar);
        if (calendar) add(t('props.calendar'), entityChip(event.calendar, calendar.title || t('untitled')));
        const work = calendar?.calendar?.['work-of'] || event.work ? workField(doc) : null;
        if (work) add(t('props.work'), ...work);
        const meets = trip ? meetsField(doc) : null;
        if (meets) add(t('props.meets'), ...meets);
        const status = { tentative: t('event.tentative'), cancelled: t('event.cancelled') }[event.status];
        if (status && !trip) add(t('props.status'), status);
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
    if (doc?.['recurring-payment']) recurringPaymentFields(doc, add);
    if (doc?.kind === 'record') recordFields(doc, add);
    if (doc?.doc_id) add(t('props.classification'), classificationField(doc));
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
    [...seg.children].forEach((option, i) => { option.dataset.focusKey = `work:${WORK_DAYS[i][0]}`; });
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
// A record's own fields (vault.md#record), a row each by its key: a value
// typed in (text, a number, yes or no) edited in place, empty removing it;
// a list or a map shown as it is; x removes it. + Field adds one, its name
// and its value typed in.
function recordFields(doc, add) {
    for (const [key, value] of Object.entries(doc.fields ?? {})) add(key, ...recordValue(doc, key, value));
    if (doc.writable) add('', newRecordField(doc));
}

function recordValue(doc, key, value) {
    const scalar = value === null || ['string', 'number', 'boolean'].includes(typeof value);
    if (!scalar) return [JSON.stringify(value)];
    if (!doc.writable) return [String(value ?? '')];
    const input = fieldInput('text', `field:${key}`, value == null ? '' : String(value), 'form-input');
    editField(input, typed => setFieldsLocal(doc.doc_id, { [key]: typedAs(typed.trim(), value) }));
    const remove = accEl('button', 'icon-btn icon-btn--quiet');
    remove.type = 'button';
    remove.title = t('record.remove_field', { key });
    remove.dataset.focusKey = `field-remove:${key}`;
    remove.append(iconEl('#i-dismiss'));
    remove.addEventListener('click', () => setFieldsLocal(doc.doc_id, { [key]: null }));
    return [input, remove];
}

// `text` as a field's value: none if empty, a number or yes/no as the value
// `before` was, else text.
function typedAs(text, before) {
    if (!text) return null;
    if (typeof before === 'number' && !isNaN(Number(text))) return Number(text);
    if (typeof before === 'boolean' && ['true', 'false'].includes(text)) return text === 'true';
    return text;
}

function newRecordField(doc) {
    const add = accEl('button', 'tool label', t('record.add_field'));
    add.type = 'button';
    add.dataset.focusKey = 'field-new';
    add.addEventListener('click', () => {
        const name = fieldInput('text', 'field-new:name', '', 'form-input');
        name.placeholder = t('record.field_name');
        const value = fieldInput('text', 'field-new:value', '', 'form-input');
        value.placeholder = t('record.field_value');
        const line = accEl('span', 'props__new-field');
        line.append(name, value);
        add.replaceWith(line);
        name.focus();
        const done = write => {
            const key = name.value.trim();
            line.replaceWith(add);
            if (write && key && !/[.\s]/.test(key) && value.value.trim()) {
                setFieldsLocal(doc.doc_id, { [key]: value.value.trim() });
            }
        };
        for (const input of [name, value]) {
            input.addEventListener('keydown', e => {
                e.stopPropagation();
                if (e.key === 'Enter') done(true);
                else if (e.key === 'Escape') done(false);
            });
        }
    });
    return add;
}

// ----------------------------------------------------------------------------
// Which models may read the entity (vault.md#metadata): its own
// classification, else its kind's (a bank account, a statement: PII), else
// the vault's; a menu sets its own, or none.
function classificationField(doc) {
    const own = doc.classification ?? null;
    const kind = doc['bank-account'] || doc.statement ? 'pii' : null;
    const fallback = kind ?? vaultSettings().classification ?? 'public';
    const name = value => CLASSIFICATIONS.find(([v]) => v === value)?.[1] ?? value;
    const means = value => CLASSIFICATIONS.find(([v]) => v === value)?.[2] ?? t('class.no_model');
    const shown = own ? name(own)
        : kind ? t('class.default_kind', { name: name(fallback) }) : t('class.default_vault', { name: name(fallback) });
    if (!doc.writable) return shown;
    const btn = accEl('button', 'tool label', shown);
    btn.type = 'button';
    btn.title = means(own ?? fallback);
    btn.dataset.focusKey = 'classification';
    const set = value => {
        if (value !== own) setFieldsLocal(doc.doc_id, { classification: value });
    };
    btn.addEventListener('click', () => showMenu(btn, [
        { label: kind ? t('class.kind_s', { name: name(fallback) }) : t('class.vault_s', { name: name(fallback) }), checked: !own, run: () => set(null) },
        ...CLASSIFICATIONS.map(([value, label, what]) => ({ label, desc: what, checked: own === value, run: () => set(value) })),
    ], { head: t('class.which_models') }));
    return btn;
}

// ----------------------------------------------------------------------------
// A recurring payment's fields (vault.md#recurring): its account, then while
// the host can write it its party, schedule (a period and its day, as
// `scheduleLine` writes them; another schedule in words), first and last day,
// current amount (since when, and the earlier ones), how far a booking may
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
            const seg = accSeg(PERIODS, schedule.period, period => {
                write({ schedule: [scheduleLine(period, schedule.day ?? from.getDate(), from)] });
            });
            [...seg.children].forEach((option, i) => { option.dataset.focusKey = `schedule:${PERIODS[i][0]}`; });
            const dayInput = scheduleDayInput(schedule.day ?? from.getDate(), 'schedule:day');
            editField(dayInput, value => {
                const n = Number(value);
                if (n >= 1 && n <= 31) write({ schedule: [scheduleLine(schedule.period, n, from)] });
            });
            add(t('props.schedule'), scheduleField(seg, schedule.period, dayInput));
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
        vary.placeholder = '0.00';
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

// A recurring payment's schedule as a field (its card's, and New recurring
// payment's, features/recurring-payment-new.js): `seg`, the period's segmented
// control, then but for a week the day of the month, `day`
// (`scheduleDayInput`).
export function scheduleField(seg, period, day) {
    const field = accEl('span', 'schedule-field');
    field.append(seg, ...(period === 'week' ? [] : [accEl('span', 'meta', 'on day'), day]));
    return field;
}

// The input of a schedule's day of the month, holding `value`; `key` finds
// it again once its block is drawn anew.
export function scheduleDayInput(value, key) {
    const input = fieldInput('number', key, String(value), 'form-input schedule-field__day');
    input.min = '1';
    input.max = '31';
    return input;
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
// email is the tooltip. The card's and a booking's (features/panel/booking.js).
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
