// A project's facts in its header (features/blueprint.js, panel.md#blueprint):
// each its label, its value and a pencil, shown on hover, that edits it in
// place by its type (blueprints.rs): typed in (text, an amount), a date, a
// trip's days (from and to), or picked from a menu (contacts, a calendar, a
// choice). An edit writes the whole block through `set_project`
// (`setProjectLocal`), if the value changed; empty removes the fact. While
// a trip's dates aren't set, they show what its sketch says of them, and
// setting them opens Set the dates (features/set-dates.js) instead. The
// menus of calendars and contacts serve New and the Accounts page too.

import { accEl } from '../core/dom.js';
import { DATE_FORMAT, DAY_MONTH, SHORT_MONTH_YEAR, WEEKDAY } from '../core/format.js';
import { amountText, compareText, dayStart, daysBetween, parseAmount, parseDate } from '../core/format.js';
import { t } from '../core/text.js';
import { entities, parseContact } from '../model/entities.js';
import { factsOf, withFact } from '../model/blueprint.js';
import { setProjectLocal } from '../model/structure.js';
import { iconEl } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import { openSetDates } from './set-dates.js';
import { contactChip, entityChip } from './panel/details.js';
import { fieldInput } from '../ui/field.js';

// The fact `fact` (`factsOf`) of the project `id`, its block `project`, as
// the header shows it; `meta` a note after its value (what the budget has
// booked). Read-only while the host can't write the project.
export function factEl(fact, { id, project, writable }, meta = null) {
  const el = accEl('div', 'bp__fact');
  const value = accEl('span', 'bp__value');
  el.append(accEl('span', 'label', fact.label), value);
  const save = next => {
    if (JSON.stringify(next ?? null) === JSON.stringify(fact.value ?? null)) return;
    if (fact.type === 'days' && project.sketch && parseDate(next?.to) > parseDate(next?.from)) return openSetDates(id, next);
    setProjectLocal(id, withFact(project, fact.name, next));
  };
  const show = () => {
    value.replaceChildren(...valueParts(fact, project), ...(meta ? [accEl('span', 'meta', meta)] : []));
    if (!writable) return;
    const pencil = accEl('button', 'icon-btn icon-btn--quiet');
    pencil.type = 'button';
    pencil.title = t('facts.change', { label: fact.label });
    pencil.append(iconEl('#i-edit'));
    pencil.addEventListener('click', () => edit(fact, value, pencil, save, show));
    value.append(pencil);
  };
  show();
  return el;
}

// "+ Fact" after the project `id`'s facts: a fact the blueprint doesn't
// name, its name and its value typed in, as a record's + Field
// (features/panel/details.js); Enter writes it as text (`withFact`), Esc
// drops it. A name the project has a fact of already isn't written: the
// fact is edited by its own pencil.
export function newFactEl({ id, project }) {
  const add = accEl('button', 'tool label', t('facts.add'));
  add.type = 'button';
  add.dataset.focusKey = 'fact-new';
  add.addEventListener('click', () => {
    const name = fieldInput('text', 'fact-new:name', '', 'form-input');
    name.placeholder = t('facts.name');
    const value = fieldInput('text', 'fact-new:value', '', 'form-input');
    value.placeholder = t('facts.value');
    const line = accEl('span', 'bp__new-fact');
    line.append(name, value);
    add.replaceWith(line);
    name.focus();
    const known = new Set(factsOf(project).map(fact => fact.name));
    const done = write => {
      const key = name.value.trim();
      if (write && known.has(key)) {
        name.setAttribute('aria-invalid', 'true');
        name.title = t('facts.exists', { name: key });
        return;
      }
      line.replaceWith(add);
      if (write && key && value.value.trim()) setProjectLocal(id, withFact(project, key, value.value.trim()));
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
// What a fact's value shows: a dash while it has none, but for open days
// what the project's sketch says of them.
function valueParts(fact, project) {
  const { type, value } = fact;
  if (value == null || value === '' || (Array.isArray(value) && !value.length)) {
    return (type === 'days' && openParts(project.sketch)) || [accEl('span', 'meta', '\u2014')];
  }
  if (type === 'date') return [DATE_FORMAT.format(parseDate(String(value)))];
  if (type === 'amount') return [amountText(Number(value))];
  if (type === 'days') return daysParts(value);
  if (type === 'contacts') return [].concat(value).map(ref => contactChip(String(ref)));
  if (type === 'calendar') {
    const calendar = entities.get(String(value));
    return [calendar ? entityChip(calendar.doc_id, calendar.title || t('untitled')) : String(value)];
  }
  return [typeof value === 'object' ? JSON.stringify(value) : String(value)];
}

// A trip's days: from and to with their weekdays, the nights and how far
// off they are.
function daysParts(days) {
  const from = parseDate(days?.from);
  const to = parseDate(days?.to);
  if (!from || !to) return [String(days?.from ?? days)];
  const ahead = daysBetween(dayStart(new Date()), from);
  const meta = [t('trip.nights', { n: daysBetween(from, to) }), ahead > 0 ? t('facts.in_days', { n: ahead }) : null];
  return [`${WEEKDAY.format(from)} ${DAY_MONTH.format(from)} \u2013 ${WEEKDAY.format(to)} ${DATE_FORMAT.format(to)}`,
    accEl('span', 'meta', meta.filter(Boolean).join(' \u00b7 '))];
}

// A trip's days while they are open (vault.md#project): the window its
// sketch falls within ("Aug 2028" for a whole month) and how long it takes;
// none if the sketch says neither.
function openParts(sketch) {
  const from = parseDate(sketch?.window?.from);
  const to = parseDate(sketch?.window?.to);
  const n = sketch?.nights;
  const nights = n == null ? null
    : typeof n === 'number' ? t('trip.nights', { n }) : t('facts.nights_text', { nights: String(n).replace('-', '\u2013') });
  const window = from && to ? windowText(from, to) : null;
  if (!window && !nights) return null;
  const meta = [window && nights, t('facts.not_set')].filter(Boolean).join(' \u00b7 ');
  return [window ?? nights, accEl('span', 'meta', meta)];
}

function windowText(from, to) {
  const last = new Date(from.getFullYear(), from.getMonth() + 1, 0);
  const wholeMonth = from.getDate() === 1 && daysBetween(last, to) === 0;
  return wholeMonth ? SHORT_MONTH_YEAR.format(from) : `${DAY_MONTH.format(from)} \u2013 ${DATE_FORMAT.format(to)}`;
}

// ----------------------------------------------------------------------------
// Edit `fact` in `value` (its pencil `anchor`): a menu for what is picked,
// inputs in place for what is typed. `save(next)` writes it, `show()` puts
// the value back.
function edit(fact, value, anchor, save, show) {
  const { type } = fact;
  if (type === 'contacts') return pickContacts(fact, anchor, save);
  if (type === 'calendar') return pickCalendar(fact, anchor, save);
  if (type === 'choice') return pickChoice(fact, anchor, save);
  if (type === 'days') {
    const days = fact.value ?? {};
    const from = input('date', days.from ?? '', t('props.from'));
    const to = input('date', days.to ?? '', t('props.to'));
    return editInPlace(value, [from, to],
      () => (from.value || to.value ? { from: from.value, to: to.value } : null), save, show);
  }
  if (type === 'date') {
    const day = input('date', fact.value ? String(fact.value).slice(0, 10) : '', fact.label);
    return editInPlace(value, [day], () => day.value || null, save, show);
  }
  const text = input('text', fact.value == null ? '' : type === 'amount' ? amountText(Number(fact.value)) : String(fact.value), fact.label);
  return editInPlace(value, [text], () => {
    const typed = text.value.trim();
    if (type !== 'amount' || !typed) return typed || null;
    return parseAmount(typed) ?? fact.value;
  }, save, show);
}

function input(type, value, label) {
  const el = accEl('input', 'form-input bp__input');
  el.type = type;
  el.value = value;
  el.setAttribute('aria-label', label);
  if (type === 'text') el.placeholder = label;
  return el;
}

// Inputs in place of the value: Enter, or the focus leaving them, writes
// what `read()` gives; Esc puts the value back.
function editInPlace(value, inputs, read, save, show) {
  const group = accEl('span', 'bp__edit');
  group.append(...inputs);
  value.replaceChildren(group);
  inputs[0].focus();
  let done = false;
  const end = write => {
    if (done) return;
    done = true;
    if (write) save(read());
    show();
  };
  group.addEventListener('click', e => e.stopPropagation());
  group.addEventListener('keydown', e => {
    e.stopPropagation();
    if (e.key === 'Enter') {
      e.preventDefault();
      end(true);
    } else if (e.key === 'Escape') {
      end(false);
    }
  });
  group.addEventListener('focusout', e => {
    if (!group.contains(e.relatedTarget)) end(true);
  });
}

// ----------------------------------------------------------------------------
// The vault's contacts to tick, and a name typed that the vault has none for.
function pickContacts(fact, anchor, save) {
  const refs = [].concat(fact.value ?? []).map(String);
  const toggle = ref => save(refs.includes(ref) ? refs.filter(r => r !== ref) : [...refs, ref]);
  const items = [
    ...refs.filter(ref => !parseContact(ref).id)
      .map(ref => ({ label: parseContact(ref).name, checked: true, run: () => toggle(ref) })),
    ...vaultContacts().map(doc => ({
      label: doc.title || t('untitled'), checked: refs.includes(`@${doc.doc_id}`), run: () => toggle(`@${doc.doc_id}`),
    })),
  ];
  showMenu(anchor, items, {
    head: fact.label,
    filter: { placeholder: t('facts.filter_contacts'), extra: text => [{ label: t('facts.add_named', { name: text }), run: () => toggle(text) }] },
  });
}

// The vault's contacts at `anchor`, to pick one of (`onPick('@id')`) or
// nobody (null), `current` ticked (a name the vault has no contact for
// too); also the Accounts page's Work of (features/workspace/accounts.js).
export function contactMenu(anchor, current, onPick, head) {
  const named = current && !parseContact(current).id ? [{ label: parseContact(current).name, checked: true, run: () => {} }] : [];
  showMenu(anchor, [
    { label: t('facts.nobody'), checked: !current, run: () => onPick(null) },
    ...named,
    ...vaultContacts().map(doc => ({
      label: doc.title || t('untitled'), checked: current === `@${doc.doc_id}`, run: () => onPick(`@${doc.doc_id}`),
    })),
  ], { head, filter: { placeholder: t('facts.filter_contacts') } });
}

function vaultContacts() {
  return [...entities.values()].filter(doc => doc.kind === 'contact')
    .sort((a, b) => compareText(a.title || '', b.title || ''));
}

function pickCalendar(fact, anchor, save) {
  calendarMenu(anchor, fact.value, save, fact.label, { writable: true });
}

// The vault's calendars at `anchor`, to pick one of (`onPick(id)`) or none
// (null), `current` ticked; `writable`, for new events, those Google keeps
// as they are left out (`access`, api.md#events). Also New's
// (features/new.js) and When's (features/when.js).
export function calendarMenu(anchor, current, onPick, head = t('props.calendar'), { writable = false } = {}) {
  const shown = doc => doc.kind === 'calendar' && !(writable && doc.access === 'read-only');
  const calendars = [...entities.values()].filter(shown);
  showMenu(anchor, [
    { label: t('watch.none'), checked: !current, run: () => onPick(null) },
    ...calendars.map(doc => ({ label: doc.title || t('untitled'), checked: current === doc.doc_id, run: () => onPick(doc.doc_id) })),
  ], { head });
}

// A choice's values to pick one of, or none.
function pickChoice(fact, anchor, save) {
  showMenu(anchor, [
    { label: t('watch.none'), checked: !fact.value, run: () => save(null) },
    ...(fact.choices ?? []).map(choice => ({ label: choice, checked: fact.value === choice, run: () => save(choice) })),
  ], { head: fact.label });
}
