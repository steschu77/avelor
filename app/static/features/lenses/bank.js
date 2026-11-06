// The Bank lens (ui.md#bank): the bank accounts in the scope as tiles, with
// their total; the cut, money in and money out; then the bookings by month,
// newest first, each with its account's balance after it (`ledger`,
// model/bank.js). The scope is the vault panel's filter on the statements
// (`matchesFilter`): with nothing narrowing it, every bank account shows,
// one without statements too. A tile makes its account's source the
// filter; a booking selects its statement, whose card the panel shows.
// Without a bank account, the lens says where bookings come from and
// offers Add a bank account (the Folders page, `addBankAccount`).

import { $, accEl } from '../../core/dom.js';
import { DAY_MONTH, DAY_MONTH_YEAR, DAY_SHORT_MONTH, MONTH_YEAR } from '../../core/format.js';
import { amountText, money, plural, signedAmount } from '../../core/format.js';
import { store } from '../../model/state.js';
import { accountWatch, entities, matchesFilter, watchSourceKey, watches } from '../../model/entities.js';
import { ACCOUNT_KINDS, BOOKING_KINDS, cents, ledger, statementsEnd } from '../../model/bank.js';
import { KIND_ICONS, iconEl } from '../../ui/icons.js';
import { selectDocument, selectFilter } from '../selection.js';
import { addBankAccount } from '../workspace/folders.js';
import { registerLens } from './registry.js';

const lensEl = $('lens-bank');
const countEl = $('bank-count');

// The cut: whether money in and money out show, each switched by its
// toggle, for the session.
const cut = { in: true, out: true };

const KINDS = new Map(ACCOUNT_KINDS);

// ----------------------------------------------------------------------------
// The head's count, while the lens is shown.
store.subscribe(state => state.selected_lens, lens => {
  countEl.hidden = lens !== 'bank';
});

// A booking selects its statement, by click or Enter.
function onRow(e) {
  const row = e.target.closest('.bank__row');
  if (row && (e.type === 'click' || e.key === 'Enter')) selectDocument(row.dataset.statement);
}
lensEl.addEventListener('click', onRow);
lensEl.addEventListener('keydown', onRow);

// ----------------------------------------------------------------------------
// The accounts and statements in the scope: with nothing narrowing it,
// every bank account; else the accounts of the statements in it, and the
// account of a statements watch picked as the source.
function scopeOf(state) {
  const statements = new Set();
  const all = [];
  for (const doc of entities.values()) {
    if (doc.kind === 'bank-account') all.push(doc);
    else if (doc.transactions && matchesFilter(doc, state)) statements.add(doc);
  }
  const narrowed = state.selected_project || state.selected_source || state.selected_labels.length
    || state.word_filter;
  if (!narrowed) return { accounts: all, statements };
  const ids = new Set([...statements].map(doc => doc.transactions.account));
  for (const watch of watches.values()) {
    if (watch.account && watchSourceKey(watch.id) === state.selected_source) ids.add(watch.account);
  }
  return { accounts: all.filter(doc => ids.has(doc.id)), statements };
}

// The one currency of `accounts`, or null if they have several.
function currencyOf(accounts) {
  const currencies = new Set(accounts.map(doc => doc['bank-account']?.currency));
  return currencies.size === 1 ? [...currencies][0] : null;
}

function renderBankLens() {
  const state = store.getState();
  const accounts = [...entities.values()].filter(doc => doc.kind === 'bank-account');
  if (!accounts.length) {
    countEl.textContent = '';
    lensEl.replaceChildren(renderEmpty());
    return;
  }
  const scope = scopeOf(state);
  const ledgers = new Map(scope.accounts.map(doc => [doc.id, ledger(doc)]));

  // The bookings in the scope, newest first; a pending one before the
  // booked ones of its day.
  const bookings = [];
  for (const account of scope.accounts) {
    const { entries, pending } = ledgers.get(account.id);
    entries.forEach((entry, seq) => {
      if (scope.statements.has(entry.statement)) bookings.push({ ...entry, account, seq, pending: false });
    });
    pending.forEach((entry, i) => {
      if (scope.statements.has(entry.statement)) bookings.push({ ...entry, account, seq: 1e9 - i, pending: true });
    });
  }
  bookings.sort((a, b) => b.time - a.time || b.seq - a.seq);
  const isIn = entry => entry.booking.amount > 0;
  const counts = { in: bookings.filter(isIn).length, out: bookings.filter(entry => !isIn(entry)).length };
  const shown = bookings.filter(entry => (isIn(entry) ? cut.in : cut.out));

  countEl.textContent = `${plural(shown.length, 'booking')} \u00b7 ${plural(scope.accounts.length, 'account')}`;
  const parts = [];
  if (scope.accounts.length) parts.push(renderAccounts(scope.accounts, ledgers, state));
  parts.push(renderCut(counts));
  if (shown.length) {
    parts.push(renderBookings(shown, scope.accounts, ledgers));
  } else {
    const text = scope.accounts.length ? 'No bookings here.' : 'No bank account in this scope.';
    parts.push(accEl('p', 'note bank__none', text));
  }
  lensEl.replaceChildren(...parts);
}

// ----------------------------------------------------------------------------
// The accounts as tiles, then their total while there are several.
function renderAccounts(accounts, ledgers, state) {
  const strip = accEl('div', 'bank-accounts');
  for (const account of accounts) strip.append(renderAccount(account, ledgers.get(account.id), state));
  if (accounts.length > 1) {
    const currency = currencyOf(accounts);
    const sum = accounts.reduce((n, doc) => n + cents(ledgers.get(doc.id).balance), 0) / 100;
    const pending = accounts.reduce((n, doc) => n + cents(ledgers.get(doc.id).pendingSum), 0) / 100;
    const total = accEl('div', 'bank-total');
    total.append(accEl('span', 'label', plural(accounts.length, 'account')));
    if (currency !== null) {
      total.append(accEl('span', 'figure', money(sum, currency)));
      if (pending) total.append(accEl('span', 'meta', `${money(sum + pending, currency)} with what is pending`));
    } else {
      total.append(accEl('span', 'meta', 'in several currencies'));
    }
    strip.append(total);
  }
  return strip;
}

// An account: its kind and bank, its name and balance, its number, what is
// pending and how far its statements go. A click makes its watch's source
// the filter, or the whole vault again; an account without a watch shows
// its card.
function renderAccount(account, book, state) {
  const fields = account['bank-account'] ?? {};
  const name = account.title || 'Untitled';
  const watch = accountWatch(account.id);
  const key = watch ? watchSourceKey(watch.id) : null;
  const tile = accEl('button', 'tile bank-account');
  tile.type = 'button';
  if (key && state.selected_source === key) tile.setAttribute('aria-current', 'true');
  tile.title = !key ? `Open ${name}` : state.selected_source === key ? 'Show every source' : `Show only ${name}`;
  tile.addEventListener('click', () => {
    if (!key) selectDocument(account.id);
    else selectFilter({ selected_source: store.getState().selected_source === key ? null : key });
  });
  const kind = accEl('span', 'label bank-account__kind');
  kind.append(iconEl(KIND_ICONS['bank-account'].icon),
    [KINDS.get(fields.type), fields.institution].filter(Boolean).join(' \u00b7 ') || 'Bank account');
  const balance = book.balance ?? fields.balance;
  const end = statementsEnd(account.id);
  const meta = [
    fields.number,
    book.pending.length ? `${signedAmount(book.pendingSum)} pending` : null,
    end ? `statement to ${DAY_SHORT_MONTH.format(end)}` : 'no statement yet',
  ];
  tile.append(kind, accEl('span', 'bank-account__name', name),
    accEl('span', 'figure', balance != null ? money(balance, fields.currency) : '\u2014'),
    accEl('span', 'meta bank-account__meta', meta.filter(Boolean).join(' \u00b7 ')));
  return tile;
}

// ----------------------------------------------------------------------------
// The cut: a toggle for money in and one for money out, with their counts.
function renderCut(counts) {
  const bar = accEl('div', 'filter-bar');
  for (const [key, label, what] of [['in', 'In', 'money in'], ['out', 'Out', 'money out']]) {
    const toggle = accEl('button', 'type-toggle', label);
    toggle.type = 'button';
    toggle.setAttribute('aria-pressed', String(cut[key]));
    toggle.classList.toggle('is-empty', !counts[key]);
    toggle.title = `${cut[key] ? 'Hide' : 'Show'} ${what}`;
    toggle.append(accEl('span', 'type-toggle__n', String(counts[key])));
    toggle.addEventListener('click', () => {
      cut[key] = !cut[key];
      renderBankLens();
    });
    bar.append(toggle);
  }
  return bar;
}

// ----------------------------------------------------------------------------
// The bookings under their months, then where the statements start.
function renderBookings(shown, accounts, ledgers) {
  const table = accEl('div', 'table bank');
  const head = accEl('div', 'list-row table__head');
  head.append(accEl('span', 'label', 'Date'), accEl('span', 'label', 'Booking'),
    accEl('span', 'label bank__head-end', 'Amount'), accEl('span', 'label bank__head-end', 'Balance'));
  const body = accEl('div', 'table__body');
  const several = accounts.length > 1;
  const currency = currencyOf(accounts);
  let month = null;
  let group = [];
  const flush = () => {
    if (group.length) body.append(renderMonth(group, accounts, ledgers, currency), ...group.map(entry => renderRow(entry, several)));
    group = [];
  };
  for (const entry of shown) {
    const date = new Date(entry.time);
    const key = date.getFullYear() * 12 + date.getMonth();
    if (key !== month) {
      flush();
      month = key;
    }
    group.push(entry);
  }
  flush();
  table.append(head, body);
  const starts = accounts.map(doc => [doc, ledgers.get(doc.id)]).filter(([, book]) => book.entries.length);
  if (starts.length) {
    const first = Math.min(...starts.map(([, book]) => book.entries[0].time));
    const at = starts.filter(([, book]) => book.opening != null)
      .map(([doc, book]) => `${doc.title || 'Untitled'} at ${amountText(book.opening)}`);
    const note = accEl('div', 'lens-hidden');
    note.append(accEl('span', 'note',
      `The first statements start on ${DAY_MONTH_YEAR.format(new Date(first))}${at.length ? `: ${at.join(', ')}` : ''}.`));
    table.append(note);
  }
  return table;
}

// A month's head: its name, its bookings and what is pending of them, the
// sum of the booked ones, and the accounts' balance at its end (while they
// share a currency).
function renderMonth(group, accounts, ledgers, currency) {
  const first = new Date(group[0].time);
  const end = new Date(first.getFullYear(), first.getMonth() + 1, 1).getTime();
  const pending = group.filter(entry => entry.pending).length;
  const net = group.filter(entry => !entry.pending).reduce((n, entry) => n + cents(entry.booking.amount), 0) / 100;
  let balance = null;
  if (currency !== null) {
    for (const account of accounts) {
      const before = ledgers.get(account.id).entries.filter(entry => entry.time < end && entry.balance != null);
      if (before.length) balance = (balance ?? 0) + before.at(-1).balance;
    }
  }
  const row = accEl('div', 'list-row group-row');
  const name = accEl('span', 'bank__group-name');
  name.append(accEl('span', 'group-row__name', MONTH_YEAR.format(first)),
    accEl('span', 'meta', [plural(group.length, 'booking'), pending ? `${pending} pending` : null].filter(Boolean).join(' \u00b7 ')));
  row.append(name, accEl('span', 'bank__net', signedAmount(net)), accEl('span', 'meta bank__balance', amountText(balance)));
  return row;
}

// A booking: its day; who paid or was paid, "Pending", then what for (and
// the account, while the scope holds several); the amount and the
// account's balance after it.
function renderRow(entry, several) {
  const { booking, account } = entry;
  const amount = booking.amount ?? 0;
  const who = (amount > 0 ? booking.sender : booking.receiver) || booking.title || 'Booking';
  const what = booking.purpose
    || (booking.title && booking.title !== who ? booking.title : BOOKING_KINDS.get(booking.kind) ?? '');
  const row = accEl('div', `list-row table__row bank__row${entry.pending ? ' bank__row--pending' : ''}`);
  row.tabIndex = 0;
  row.setAttribute('role', 'button');
  row.dataset.statement = entry.statement.doc_id;
  row.title = `Open ${entry.statement.title || 'the statement'}`;
  const cell = accEl('span', 'bank__cell');
  cell.append(iconEl(KIND_ICONS.transactions.icon, 'table__icon'), accEl('span', 'row-title', who));
  if (entry.pending) cell.append(accEl('span', 'chip chip--pending', 'Pending'));
  const detail = [several ? account.title || 'Untitled' : null, what].filter(Boolean).join(' \u00b7 ');
  if (detail) cell.append(accEl('span', 'meta bank__detail', detail));
  row.append(accEl('span', 'meta', DAY_MONTH.format(new Date(entry.time))), cell,
    accEl('span', `bank__amount${amount > 0 ? ' bank__amount--in' : ''}`, signedAmount(amount)),
    accEl('span', 'meta bank__balance', entry.pending ? '' : amountText(entry.balance)));
  return row;
}

// ----------------------------------------------------------------------------
// No bank account yet: where bookings come from, and Add a bank account.
function renderEmpty() {
  const empty = accEl('div', 'empty bank-empty');
  const add = accEl('button', 'btn btn--primary', 'Add a bank account');
  add.type = 'button';
  add.title = 'Folders: a new bank account and the folder of its statements';
  add.addEventListener('click', addBankAccount);
  empty.append(iconEl(KIND_ICONS['bank-account'].icon, 'empty__icon'),
    accEl('span', 'empty__title', 'No bank accounts yet'),
    accEl('p', 'note',
      'Bookings come from your bank\u2019s statements. Add a bank account, export its statements as CSV files into a folder, and each one is read into the account.'),
    add);
  return empty;
}

registerLens('bank', lensEl, renderBankLens);
