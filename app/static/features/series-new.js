// New recurring payment (vault.md#recurring): the form a booking's Series
// menu opens (features/panel/booking.js), filled from the booking, in the
// expanded vault area while its op is "series" (panel.md#a-new-series):
// the title (the party's name), the account and party, the schedule (a
// period and its day, `everyLine`), from and until, the amount, how far it
// may vary, the category, what it is linked to (the contract or policy it
// comes from), its projects and labels, and the bookings it starts with --
// the account's bookings of the party not in a series, ticked, and those of
// a similar name (`similarNames`) to tick by hand. From follows the first
// ticked booking until it is set. Create sends `create_recurring`, then
// `set_booking_recurring` for the ticked bookings, and selects the series.

import { accEl } from '../core/dom.js';
import { rpcCall } from '../core/rpc.js';
import { DAY_MONTH, localDay, parseAmount, parseDate, plural, signedAmount } from '../core/format.js';
import { entities } from '../model/entities.js';
import { PERIODS, everyLine } from '../model/calendar.js';
import { bookingDay, partyName, similarNames, statementsOf } from '../model/bank.js';
import { setBookingRecurringLocal } from '../model/structure.js';
import { KIND_ICONS, iconEl } from '../ui/icons.js';
import { closeVaultArea, openVaultOp, registerVaultPage, replaceVaultPage, setPageFocus } from '../ui/page.js';
import { accSeg } from '../ui/segmented.js';
import { linkMenuChip } from './linked.js';
import { draftChips, newDateInput, newTextInput } from './new.js';
import { everyDayInput, everyField, offerCategories } from './panel/details.js';
import { linkChip, linkTarget } from './panel/panel.js';
import { selectDocument } from './selection.js';
import { watchFormGrid } from './workspace/folders.js';

let draft = null; // the form shown

// Open the form for the booking `booking` of the statement `statement`.
export function openNewSeries(statement, booking) {
  const account = statement.transactions.account;
  const party = partyName(booking);
  const day = bookingDay(booking) ?? new Date();
  const d = {
    account, party,
    title: party,
    period: 'month',
    day: day.getDate(),
    from: localDay(day),
    fromSet: false,
    until: '',
    amount: String(booking.amount ?? ''),
    vary: '',
    category: '',
    linked: null,
    projects: [],
    tags: [],
    candidates: candidatesOf(account, party, booking),
    ticked: new Set([bookingKey(statement.doc_id, booking.id)]),
    busy: false,
    error: null,
    els: {},
  };
  draft = d;
  openVaultOp('series', 'New recurring payment');
  setPageFocus('series:title');
  replaceVaultPage(renderForm(d));
  renderChips(d);
  renderTicks(d);
  renderFoot(d);
}

registerVaultPage('series', {
  close: closeVaultArea,
  submit: () => {
    if (draft) createSeries(draft);
  },
});

const bookingKey = (statement, booking) => `${statement}/${booking}`;

// The account's bookings not in a series that go the way `booking` goes:
// those of its party (`same`), then those of a similar name, newest first;
// `{ key, statement, booking, day, same }` each.
function candidatesOf(account, party, booking) {
  const name = party.trim().toLowerCase();
  const found = [];
  for (const statement of statementsOf(account)) {
    for (const item of statement.transactions.transactions ?? []) {
      if (item.recurring || (item.amount < 0) !== (booking.amount < 0)) continue;
      const itemName = partyName(item);
      const other = itemName.trim().toLowerCase();
      const same = other === name || item.id === booking.id;
      if (!same && !similarNames(other, name)) continue;
      const day = bookingDay(item) ?? new Date();
      found.push({ key: bookingKey(statement.doc_id, item.id), statement, booking: item, day, same, name: itemName });
    }
  }
  return found.sort((a, b) => b.day - a.day);
}

// ----------------------------------------------------------------------------
// The form, built once; its chips, ticks and foot are built again.
function renderForm(d) {
  const sheet = accEl('section', 'new__sheet');
  const kind = accEl('h2', 'pane-title new__kind');
  kind.append(iconEl(KIND_ICONS.recurring.icon), 'Recurring payment');
  const onInput = () => renderFoot(d);
  const title = newTextInput(d, 'title', 'What it is for', { className: 'input new__title', onInput });
  title.dataset.focusKey = 'series:title';
  const path = accEl('span', 'mono mono--small truncate-start new__path', 'Accounts\\');
  path.title = 'The file is named after the date, the title and the type when it is created.';

  const [rows, row] = watchFormGrid();
  rows.classList.add('new__rows');
  const account = entities.get(d.account);
  row('Account', accEl('span', 'chip', account?.title || 'Untitled'),
    accEl('span', 'meta', account?.['bank-account']?.number ?? ''));
  row('Party', newTextInput(d, 'party', 'Who is paid, or pays', { onInput }), accEl('span', 'note', 'as the bank names it'));

  const every = row('Every');
  const dayInput = everyDayInput(d.day, 'series:day');
  dayInput.addEventListener('input', () => {
    d.day = Number(dayInput.value);
    renderFoot(d);
  });
  const renderEvery = () => {
    const seg = accSeg(PERIODS, d.period, period => {
      d.period = period;
      renderEvery();
    });
    every.replaceChildren(everyField(seg, d.period, dayInput));
  };
  renderEvery();

  d.els.from = newDateInput(d.from, value => {
    d.from = value;
    d.fromSet = true;
    renderFoot(d);
  });
  row('From', d.els.from, accEl('span', 'meta', 'the first booking ticked, until you set it'));
  const until = newDateInput(d.until, value => { d.until = value; });
  row('Until', until, accEl('span', 'meta', 'open-ended without it'));
  const amount = newTextInput(d, 'amount', '0.00', { className: 'form-input series-amount', onInput });
  row('Amount', amount, accEl('span', 'meta', 'each time; negative for money out'));
  const vary = newTextInput(d, 'vary', '0.00', { className: 'form-input series-amount', onInput });
  row('Varies by', vary, accEl('span', 'note', 'how far a booking may differ and still pay it as it is'));
  const category = newTextInput(d, 'category', 'None', { onInput });
  offerCategories(category);
  row('Category', category);
  d.els.linked = row('Linked to');
  d.els.chips = row('Projects, labels');

  const head = accEl('div', 'section-head new__body-head', 'Bookings');
  head.append(accEl('span', 'section-head__note', 'the ticked ones join the series'));
  d.els.ticks = accEl('div', 'series-ticks');
  d.els.error = accEl('div', 'error-note new__error');
  d.els.foot = accEl('div', 'new__foot');
  sheet.append(kind, accEl('span', 'label new__title-label', 'Title'), title, path, rows, head, d.els.ticks, d.els.error, d.els.foot);
  const page = accEl('div', 'page');
  page.append(sheet);
  return page;
}

// What it is linked to (a chip with x, then "+", `linkMenuChip`), its
// projects and labels (`draftChips`).
function renderChips(d) {
  const link = to => {
    d.linked = to;
    renderChips(d);
  };
  const add = linkMenuChip('Link to the contract or policy it comes from', { current: d.linked, onPick: link });
  d.els.linked.replaceChildren(...(d.linked && linkTarget(d.linked) ? [linkChip(d.linked, { onRemove: () => link(null) })] : []), add,
    accEl('span', 'note', 'its card then shows the series'));
  draftChips(d, d.els.chips, () => renderChips(d));
}

// The bookings to start with: the party's, then those of a similar name
// with Tick all; each with its check, day, name, purpose and amount.
function renderTicks(d) {
  const row = candidate => {
    const el = accEl('div', 'list-row series-tick');
    const on = d.ticked.has(candidate.key);
    const check = accEl('button', 'check', on ? '\u2713' : '');
    check.type = 'button';
    check.setAttribute('aria-pressed', String(on));
    check.title = on ? 'Leave it out' : 'Add it';
    check.addEventListener('click', () => {
      if (on) d.ticked.delete(candidate.key);
      else d.ticked.add(candidate.key);
      renderTicks(d);
      renderFoot(d);
    });
    const { booking } = candidate;
    const cell = accEl('span', 'bank__cell');
    cell.append(accEl('span', 'row-title', candidate.name || booking.title || ''),
      accEl('span', 'meta bank__detail', booking.purpose || booking.title || ''));
    el.append(check, accEl('span', 'meta', DAY_MONTH.format(candidate.day)), cell,
      accEl('span', 'bank__amount', signedAmount(booking.amount ?? 0)));
    return el;
  };
  const same = d.candidates.filter(candidate => candidate.same);
  const similar = d.candidates.filter(candidate => !candidate.same);
  const parts = same.map(row);
  if (similar.length) {
    const head = accEl('div', 'series-ticks__head');
    const all = accEl('button', 'tool label push', 'Tick all');
    all.type = 'button';
    all.addEventListener('click', () => {
      for (const candidate of similar) d.ticked.add(candidate.key);
      renderTicks(d);
      renderFoot(d);
    });
    const names = [...new Set(similar.map(candidate => candidate.name))].join(', ');
    head.append(accEl('span', 'label', 'Similar names'), accEl('span', 'meta', `${names} \u00b7 ${similar.length}`), all);
    parts.push(head, ...similar.map(row));
  }
  d.els.ticks.replaceChildren(...parts);
  if (!d.fromSet) {
    const first = d.candidates.filter(candidate => d.ticked.has(candidate.key)).at(-1);
    if (first) {
      d.from = localDay(first.day);
      d.els.from.value = d.from;
    }
  }
}

// Why the form can't be created yet, or ''.
function blocked(d) {
  if (!d.title.trim()) return 'Enter a title first';
  if (!d.from) return 'Choose the first day';
  if (d.period !== 'week' && !(d.day >= 1 && d.day <= 31)) return 'Choose a day of the month, 1 to 31';
  if (parseAmount(d.amount) == null) return 'Enter the amount';
  if (d.vary.trim() && parseAmount(d.vary) == null) return 'Enter how far it may vary, or nothing';
  return '';
}

// Create and Cancel, the bookings ticked, and the error of a failed Create.
function renderFoot(d) {
  const why = blocked(d);
  const create = accEl('button', 'btn btn--primary', d.busy ? 'Creating\u2026' : 'Create');
  create.type = 'button';
  create.disabled = !!why || d.busy;
  create.title = why || 'Create the recurring payment (Ctrl+Enter)';
  create.addEventListener('click', () => createSeries(d));
  const cancel = accEl('button', 'btn btn--ghost', 'Cancel');
  cancel.type = 'button';
  cancel.addEventListener('click', closeVaultArea);
  const n = d.ticked.size;
  const statements = new Set([...d.ticked].map(key => key.split('/')[0])).size;
  const note = n ? `A file in Accounts\\, and ${plural(n, 'booking')} in ${plural(statements, 'statement')}` : 'A file in Accounts\\';
  d.els.foot.replaceChildren(create, cancel, accEl('span', 'meta', note));
  d.els.error.textContent = d.error ?? '';
  d.els.error.hidden = !d.error;
}

function createSeries(d) {
  if (d.busy || blocked(d)) return;
  const from = parseDate(d.from);
  const amount = parseAmount(d.amount);
  const recurring = {
    account: d.account,
    party: d.party.trim() || undefined,
    every: [everyLine(d.period, d.day, from)],
    from: d.from,
    until: d.until || undefined,
    amounts: [{ from: d.from, amount }],
    vary: d.vary.trim() ? Math.abs(parseAmount(d.vary)) : undefined,
    category: d.category.trim() || undefined,
  };
  const args = { title: d.title.trim(), recurring, projects: d.projects, tags: d.tags };
  if (d.linked) args.linked_to = d.linked;
  d.busy = true;
  d.error = null;
  renderFoot(d);
  rpcCall('create_recurring', args).then(({ id }) => {
    const bookings = [...d.ticked].map(key => {
      const [statement, booking] = key.split('/');
      return { id: statement, booking };
    });
    if (bookings.length) setBookingRecurringLocal(bookings, id);
    if (draft === d) draft = null;
    closeVaultArea();
    selectDocument(id);
  }, e => {
    d.busy = false;
    d.error = e.message;
    renderFoot(d);
  });
}
