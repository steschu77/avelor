// The Bank lens (ui.md#bank): the bank accounts in the scope as tiles, with
// their total; the cut, money in, money out and transfers between the
// vault's accounts (`pairOf`, in neither In nor Out), and Linked or Not
// linked; the cash flow, a scrubber (features/lenses/scrub.js) whose range
// of weeks the transactions keep to; then the transactions by month, newest first,
// each with what it is linked to (a transfer: its other half) and its
// account's balance after it (`ledger`, model/bank.js). The scope is the vault panel's filter on the statements
// (`matchesFilter`): with nothing narrowing it, every bank account shows,
// one without statements too. Under a project, a transaction is in it through
// what it is linked to (`linkProjects`), and the project's In, Out and Net
// take the accounts' place. A tile makes its account's source the filter; a
// transaction selects itself, its card in the panel (features/panel/transaction.js).
// Without a bank account, the lens says where transactions come from and offers
// Add a bank account (the Folders page, `addBankAccount`).

import { $, accEl, setAria } from '../../core/dom.js';
import { pressable } from '../../core/dom.js';
import { DAY_MONTH, DAY_MONTH_YEAR, DAY_SHORT_MONTH, MONTH_YEAR, SHORT_MONTH_YEAR } from '../../core/format.js';
import { amountText, dayStart, everyMidnight, money, signedAmount } from '../../core/format.js';
import { registerAction } from '../../core/actions.js';
import { t } from '../../core/text.js';
import { store, vaultOpen } from '../../model/state.js';
import { NO_PROJECT, accountWatch, entities, matchesFilter, watchSourceKey, watches } from '../../model/entities.js';
import { ACCOUNT_TYPES, TRANSACTION_TYPES, transactionName, cents, ledger, linkProjects, pairOf, recurringPaymentOf, statementsEnd } from '../../model/bank.js';
import { KIND_ICONS, transactionIcon, iconEl } from '../../ui/icons.js';
import { stat, stats } from '../../ui/page.js';
import { accSeg } from '../../ui/segmented.js';
import { linkChip, linkTarget, pairChip } from '../panel/panel.js';
import { selectTransaction, selectDocument, selectFilter } from '../selection.js';
import { addBankAccount } from '../workspace/folders.js';
import { invalidateLenses, registerLens } from './registry.js';
import { scrubber, weekOf, weeksBetween } from './scrub.js';

const lensEl = $('lens-bank');
const countEl = $('bank-count');

registerAction({
  id: 'bank.add_account', group: 'get', title: t('bank.add_account'), icon: KIND_ICONS['bank-account'].icon, page: true,
  when: vaultOpen, run: addBankAccount,
});

// The cut, for the session: whether money in, money out and transfers
// show, each switched by its toggle; and which of them by their link
// ('all', 'linked', 'unlinked').
const cut = { in: true, out: true, transfers: true, link: 'all' };

const TYPES = new Map(ACCOUNT_TYPES);
const isIn = entry => entry.transaction.amount > 0;
// A transaction's side of the cut: a transfer is neither money in nor out.
const sideOf = entry => (entry.pair ? 'transfers' : isIn(entry) ? 'in' : 'out');
// A transfer is linked by its other half.
const isLinked = entry => !!entry.transaction['linked-to'] || !!entry.pair;

// ----------------------------------------------------------------------------
// The head's count, while the lens is shown.
store.subscribe(state => state.selected_lens, lens => {
  countEl.hidden = lens !== 'bank';
});

// A transaction selects itself, by click, Enter or Space; its link chip
// selects what it links to instead.
function onRow(e) {
  if (e.target.closest('.chip')) return;
  const row = e.target.closest('.bank__row');
  if (!row) return;
  if (row.dataset.transaction) selectTransaction(row.dataset.statement, row.dataset.transaction);
  else selectDocument(row.dataset.statement);
}
lensEl.addEventListener('click', onRow);

// The selected transaction's row is marked, without a render.
function markTransaction() {
  const { selected_entity, selected_transaction } = store.getState();
  for (const row of lensEl.querySelectorAll('.bank__row')) {
    setAria(row, 'aria-selected', !!selected_transaction && row.dataset.transaction === selected_transaction
      && row.dataset.statement === selected_entity);
  }
}
store.subscribe([state => state.selected_entity, state => state.selected_transaction], markTransaction);

// ----------------------------------------------------------------------------
// The accounts and statements in the scope. Under a project, or No project,
// the statements are filtered without it: their transactions are in it through
// their links (`inProject`). With nothing narrowing the scope, every bank
// account; else the accounts of the statements in it, and the account of a
// statements watch picked as the source.
function scopeOf(state) {
  const filter = state.selected_project ? { ...state, selected_project: null } : state;
  const statements = new Set();
  const all = [];
  for (const doc of entities.values()) {
    if (doc.kind === 'bank-account') all.push(doc);
    else if (doc.statement && matchesFilter(doc, filter)) statements.add(doc);
  }
  const narrowed = state.selected_project || state.selected_source || state.selected_labels.length
    || state.word_filter;
  if (!narrowed) return { accounts: all, statements };
  const ids = new Set([...statements].map(doc => doc.statement.account));
  for (const watch of watches.values()) {
    if (watch.account && watchSourceKey(watch.id) === state.selected_source) ids.add(watch.account);
  }
  return { accounts: all.filter(doc => ids.has(doc.id)), statements };
}

// Whether `entry`'s transaction is in the project selected, through its link:
// for No project, it has none that way; without a project, every transaction.
function inProject(entry, selected_project) {
  if (!selected_project) return true;
  const ids = linkProjects(entry.transaction);
  return selected_project === NO_PROJECT ? !ids.length : ids.includes(selected_project);
}

// The one currency of `accounts`, or null if they have several.
function currencyOf(accounts) {
  const currencies = new Set(accounts.map(doc => doc['bank-account']?.currency));
  return currencies.size === 1 ? [...currencies][0] : null;
}

function renderBankLens() {
  const state = store.getState();
  if (![...entities.values()].some(doc => doc.kind === 'bank-account')) {
    countEl.textContent = '';
    lensEl.replaceChildren(renderEmpty());
    return;
  }
  const project = state.selected_project && state.selected_project !== NO_PROJECT ? state.selected_project : null;
  const scope = scopeOf(state);
  const ledgers = new Map(scope.accounts.map(doc => [doc.id, ledger(doc)]));

  // The transactions in the scope, newest first; a pending one before the
  // posted ones of its day.
  const transactions = [];
  for (const account of scope.accounts) {
    const { entries, pending } = ledgers.get(account.id);
    const take = (entry, seq, isPending) => {
      if (scope.statements.has(entry.statement) && inProject(entry, state.selected_project)) {
        transactions.push({ ...entry, account, seq, pending: isPending, pair: pairOf(entry.transaction) });
      }
    };
    entries.forEach((entry, seq) => take(entry, seq, false));
    pending.forEach((entry, i) => take(entry, 1e9 - i, true));
  }
  transactions.sort((a, b) => b.time - a.time || b.seq - a.seq);
  const sideCut = entry => cut[sideOf(entry)];
  const linkCut = entry => cut.link === 'all' || isLinked(entry) === (cut.link === 'linked');
  const picked = transactions.filter(entry => flow.holds(new Date(entry.time)));
  const counts = { in: 0, out: 0, transfers: 0 };
  for (const entry of picked) counts[sideOf(entry)]++;
  const cutIn = picked.filter(sideCut);
  const linked = cutIn.filter(isLinked).length;
  const links = { linked, unlinked: cutIn.length - linked };
  const shown = cutIn.filter(linkCut);

  const accounts = project ? [...new Set(transactions.map(entry => entry.account))] : scope.accounts;
  countEl.textContent = `${t('bank.count.transactions', { n: shown.length })} \u00b7 ${t('bank.count.accounts', { n: accounts.length })}`;
  const parts = [];
  if (project) parts.push(renderProjectSums(picked, accounts));
  else if (accounts.length) parts.push(renderAccounts(accounts, ledgers, state));
  parts.push(renderCut(counts, links));
  if (transactions.length) parts.push(renderFlow(transactions, transactions.filter(entry => sideCut(entry) && linkCut(entry))));
  if (shown.length) {
    parts.push(renderTransactions(shown, accounts, ledgers, project));
  } else {
    const text = !transactions.length
      ? (project ? t('bank.none_project')
        : accounts.length ? t('bank.none_here') : t('bank.no_account'))
      : picked.length ? t('bank.none_here') : t('bank.none_weeks');
    parts.push(accEl('p', 'note bank__none', text));
  }
  lensEl.replaceChildren(...parts);
  markTransaction();
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
    total.append(accEl('span', 'label', t('bank.count.accounts', { n: accounts.length })));
    if (currency !== null) {
      total.append(accEl('span', 'figure', money(sum, currency)));
      if (pending) total.append(accEl('span', 'meta', t('bank.with_pending', { amount: money(sum + pending, currency) })));
    } else {
      total.append(accEl('span', 'meta', t('bank.several_currencies')));
    }
    strip.append(total);
  }
  return strip;
}

// An account: its type and bank, its name and balance, its number, what is
// pending and how far its statements go. A click makes its watch's source
// the filter, or the whole vault again; an account without a watch shows
// its card.
function renderAccount(account, book, state) {
  const fields = account['bank-account'] ?? {};
  const name = account.title || t('untitled');
  const watch = accountWatch(account.id);
  const key = watch ? watchSourceKey(watch.id) : null;
  const tile = accEl('button', 'tile bank-account');
  tile.type = 'button';
  if (key && state.selected_source === key) tile.setAttribute('aria-current', 'true');
  tile.title = !key ? t('open.named', { name })
    : state.selected_source === key ? t('bank.show_every_source') : t('bank.show_only', { name });
  tile.addEventListener('click', () => {
    if (!key) selectDocument(account.id);
    else selectFilter({ selected_source: store.getState().selected_source === key ? null : key });
  });
  const type = accEl('span', 'label bank-account__type');
  type.append(iconEl(KIND_ICONS['bank-account'].icon),
    [TYPES.get(fields.type), fields.bank].filter(Boolean).join(' \u00b7 ') || t('kind.bank_account'));
  const balance = book.balance ?? fields.balance;
  const end = statementsEnd(account.id);
  const meta = [
    fields.number,
    book.pending.length ? t('bank.amount_pending', { amount: signedAmount(book.pendingSum) }) : null,
    end ? t('bank.statement_to', { day: DAY_SHORT_MONTH.format(end) }) : t('bank.no_statement'),
  ];
  tile.append(type, accEl('span', 'bank-account__name', name),
    accEl('span', 'figure', balance != null ? money(balance, fields.currency) : '\u2014'),
    accEl('span', 'meta bank-account__meta', meta.filter(Boolean).join(' \u00b7 ')));
  return tile;
}

// Under a project, in the accounts' place: what came in and went out, the
// net (each without and with what is pending; transfers in none of them),
// and the accounts and months the transactions come from.
function renderProjectSums(transactions, accounts) {
  const sum = list => list.reduce((n, entry) => n + cents(entry.transaction.amount), 0) / 100;
  const counted = transactions.filter(entry => !entry.pair);
  const posted = counted.filter(entry => !entry.pending);
  const pending = counted.filter(entry => entry.pending);
  const side = (label, take, sign) => {
    const own = posted.filter(take);
    const waiting = pending.filter(take);
    const note = waiting.length ? t('bank.amount_pending', { amount: signedAmount(sum(waiting)) })
      : t('bank.count.transactions', { n: own.length });
    return stat(label, accEl('span', 'figure', amountText(sign * sum(own))), note);
  };
  const net = sum(posted);
  const withPending = sum(counted);
  const times = transactions.map(entry => entry.time);
  const [from, to] = times.length
    ? [Math.min(...times), Math.max(...times)].map(time => SHORT_MONTH_YEAR.format(new Date(time))) : [];
  return stats(
    side(t('bank.in'), isIn, 1),
    side(t('bank.out'), entry => !isIn(entry), -1),
    stat(t('bank.net'), accEl('span', 'figure', signedAmount(net)),
      withPending !== net ? t('bank.with_pending', { amount: signedAmount(withPending) }) : null),
    stat(t('props.from'), accounts.map(doc => doc.title || t('untitled')).join(', ') || '\u2014',
      from ? (from === to ? from : `${from} \u2013 ${to}`) : null));
}

// ----------------------------------------------------------------------------
// The cut: a toggle each for money in, money out and transfers, with their
// counts; then All, Linked and Not linked, counted within them.
const SIDES = [
  ['in', t('bank.in'), { show: t('bank.show_money_in'), hide: t('bank.hide_money_in') }],
  ['out', t('bank.out'), { show: t('bank.show_money_out'), hide: t('bank.hide_money_out') }],
  ['transfers', t('bank.transfers'), { show: t('bank.show_transfers'), hide: t('bank.hide_transfers') }],
];

function renderCut(counts, links) {
  const bar = accEl('div', 'filter-bar');
  for (const [key, label, what] of SIDES) {
    const toggle = accEl('button', 'type-toggle', label);
    toggle.type = 'button';
    toggle.setAttribute('aria-pressed', String(cut[key]));
    toggle.classList.toggle('is-empty', !counts[key]);
    toggle.title = cut[key] ? what.hide : what.show;
    toggle.append(accEl('span', 'type-toggle__n', String(counts[key])));
    toggle.addEventListener('click', () => {
      cut[key] = !cut[key];
      renderBankLens();
    });
    bar.append(toggle);
  }
  const options = [['all', t('bank.all')], ['linked', t('bank.linked_n', { n: links.linked })], ['unlinked', t('bank.unlinked_n', { n: links.unlinked })]];
  const link = accSeg(options, cut.link, value => {
    cut.link = value;
    renderBankLens();
  });
  link.classList.add('push');
  bar.append(link);
  return bar;
}

// ----------------------------------------------------------------------------
// The cash flow: a pair of bars per week, from the first transaction's week to
// the later of today's and the last transaction's, at most two years; money in
// above the line, out below, each as high as the square root of its sum
// against the largest week's, so one large transaction doesn't flatten the
// paydays. The bars count the posted transactions of the cut but transfers, in
// every week shown; its range keeps the transactions, the cut's counts and a
// project's sums to the weeks picked.
const FLOW_WEEKS = 104;
const flow = scrubber({ lens: 'bank', meta: t('bank.flow_meta'), track: 'scrub__track--flow' });

function renderFlow(transactions, inCut) {
  const today = dayStart(new Date());
  const times = transactions.map(entry => entry.time);
  const weeks = weeksBetween(new Date(Math.min(...times)), new Date(Math.max(today.getTime(), ...times)))
    .slice(-FLOW_WEEKS);
  const sums = weeks.map(() => ({ in: 0, out: 0 }));
  for (const entry of inCut) {
    const week = entry.pending || entry.pair ? -1 : weekOf(weeks, new Date(entry.time));
    if (week < 0) continue;
    const amount = cents(entry.transaction.amount);
    if (amount > 0) sums[week].in += amount;
    else sums[week].out -= amount;
  }
  const largest = Math.max(1, ...sums.flatMap(sum => [sum.in, sum.out]));
  const height = sum => `${Math.sqrt(sum / largest) * 100}%`;
  flow.render({
    title: t('bank.cash_flow', { n: weeks.length }),
    weeks,
    today,
    labels: weeks.length > 53 ? 'years' : 'months',
    bar(i, kept) {
      const week = kept ?? flowWeek();
      const { in: moneyIn, out } = sums[i];
      week.firstChild.firstChild.style.height = height(moneyIn);
      week.lastChild.firstChild.style.height = height(out);
      const what = [moneyIn ? `in ${amountText(moneyIn / 100)}` : null, out ? `out ${amountText(out / 100)}` : null];
      week.title = t('week.of', { day: DAY_MONTH_YEAR.format(weeks[i]), what: what.filter(Boolean).join(' \u00b7 ') || t('bank.no_transactions') });
      return week;
    },
  });
  return flow.el;
}

// A week of the cash flow: its bar of money in over its bar of money out.
function flowWeek() {
  const week = accEl('span', 'scrub__week');
  const moneyIn = accEl('span', 'scrub__in');
  const out = accEl('span', 'scrub__out');
  moneyIn.append(accEl('i'));
  out.append(accEl('i'));
  week.append(moneyIn, out);
  return week;
}

// ----------------------------------------------------------------------------
// The transactions under their months, then a note: where the statements start,
// or, under a project, how a transaction is in it; and, while transfers show,
// that they count in neither In nor Out. Under a project there is no
// balance: balances belong to accounts.
function renderTransactions(shown, accounts, ledgers, project) {
  const table = accEl('div', project ? 'table bank bank--project' : 'table bank');
  const head = accEl('div', 'list-row table__head');
  head.append(accEl('span', 'label', t('docs.date')), accEl('span', 'label', t('bank.transaction')),
    accEl('span', 'label', t('bank.linked_to')), accEl('span', 'label bank__head-end', t('props.amount')));
  if (!project) head.append(accEl('span', 'label bank__head-end bank__balance', t('props.balance')));
  const body = accEl('div', 'table__body');
  const several = accounts.length > 1;
  const currency = project ? null : currencyOf(accounts);
  let month = null;
  let group = [];
  const flush = () => {
    if (group.length) {
      body.append(renderMonth(group, accounts, ledgers, currency, project),
        ...group.map(entry => renderRow(entry, several, project)));
    }
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
  const note = [
    project
      ? t('bank.project_note')
      : startsNote(accounts, ledgers),
    shown.some(entry => entry.pair) ? t('bank.transfers_note') : null,
  ].filter(Boolean).join(' ');
  if (note) {
    const foot = accEl('div', 'lens-hidden');
    foot.append(accEl('span', 'note', note));
    table.append(foot);
  }
  return table;
}

// Where the statements start: the day of the first transaction, and each
// account's balance then.
function startsNote(accounts, ledgers) {
  const starts = accounts.map(doc => [doc, ledgers.get(doc.id)]).filter(([, book]) => book.entries.length);
  if (!starts.length) return null;
  const first = Math.min(...starts.map(([, book]) => book.entries[0].time));
  const at = starts.filter(([, book]) => book.opening != null)
    .map(([doc, book]) => t('bank.opening_at', { name: doc.title || t('untitled'), amount: amountText(book.opening) }));
  const day = DAY_MONTH_YEAR.format(new Date(first));
  return at.length ? t('bank.first_statements_at', { day, at: at.join(', ') }) : t('bank.first_statements', { day });
}

// A month's head: its name, its transactions and the transfers and what is
// pending of them, the sum of the posted ones (transfers too, so it agrees
// with the balances), and (but under a project) the accounts' balance at
// its end, while they share a currency.
function renderMonth(group, accounts, ledgers, currency, project) {
  const first = new Date(group[0].time);
  const end = new Date(first.getFullYear(), first.getMonth() + 1, 1).getTime();
  const pending = group.filter(entry => entry.pending).length;
  const transfers = group.filter(entry => entry.pair).length;
  const net = group.filter(entry => !entry.pending).reduce((n, entry) => n + cents(entry.transaction.amount), 0) / 100;
  const row = accEl('div', 'list-row group-row');
  const name = accEl('span', 'bank__group-name');
  const counts = [t('bank.count.transactions', { n: group.length }), transfers ? t('count.transfers', { n: transfers }) : null,
    pending ? t('statement.pending', { n: pending }) : null];
  name.append(accEl('span', 'group-row__name', MONTH_YEAR.format(first)),
    accEl('span', 'meta', counts.filter(Boolean).join(' \u00b7 ')));
  row.append(name, accEl('span'), accEl('span', 'bank__net', signedAmount(net)));
  if (!project) {
    let balance = null;
    if (currency !== null) {
      for (const account of accounts) {
        const before = ledgers.get(account.id).entries.filter(entry => entry.time < end && entry.balance != null);
        if (before.length) balance = (balance ?? 0) + before.at(-1).balance;
      }
    }
    row.append(accEl('span', 'meta bank__balance', amountText(balance)));
  }
  return row;
}

// A transaction: its day; who paid or was paid (a transfer: its other
// account, with the transfer's arrows), "Pending", then what for (and the
// account, while the scope holds several); what it is linked to (a
// transfer: its other half); the amount, a transfer's not marked as money
// in, and (but under a project) the account's balance after it.
function renderRow(entry, several, project) {
  const { transaction, account, pair } = entry;
  const amount = transaction.amount ?? 0;
  const who = transactionName(transaction, pair);
  const kind = TRANSACTION_TYPES.get(pair ? 'transfer' : transaction.type) ?? '';
  const what = transaction.memo || (!pair && transaction.title && transaction.title !== who ? transaction.title : kind);
  const row = pressable(accEl('div', `list-row table__row bank__row${entry.pending ? ' bank__row--pending' : ''}`));
  row.dataset.statement = entry.statement.doc_id;
  if (transaction.id) row.dataset.transaction = transaction.id;
  row.title = t('open.named', { name: who });
  const cell = accEl('span', 'bank__cell');
  const recurringPayment = recurringPaymentOf(transaction);
  const iconPart = iconEl(transactionIcon(pair, recurringPayment), 'table__icon');
  if (recurringPayment) iconPart.setAttribute('aria-label', t('bank.recurring_label', { name: recurringPayment.title || t('untitled') }));
  cell.append(iconPart, accEl('span', 'row-title', who));
  if (entry.pending) cell.append(accEl('span', 'chip chip--pending', t('bank.pending_chip')));
  const detail = [several ? account.title || t('untitled') : null, what].filter(Boolean).join(' \u00b7 ');
  if (detail) cell.append(accEl('span', 'meta bank__detail', detail));
  const link = accEl('span', 'bank__link');
  const to = transaction['linked-to'];
  if (pair) link.append(pairChip(pair));
  else if (to) link.append(linkTarget(to) ? linkChip(to) : accEl('span', 'meta', '\u2014'));
  else if (recurringPayment) link.append(linkChip(recurringPayment.id));
  row.append(accEl('span', 'meta', DAY_MONTH.format(new Date(entry.time))), cell, link,
    accEl('span', `bank__amount${amount > 0 && !pair ? ' bank__amount--in' : ''}`, signedAmount(amount)));
  if (!project) row.append(accEl('span', 'meta bank__balance', entry.pending ? '' : amountText(entry.balance)));
  return row;
}

// ----------------------------------------------------------------------------
// No bank account yet: where transactions come from, and Add a bank account.
function renderEmpty() {
  const empty = accEl('div', 'empty bank-empty');
  const add = accEl('button', 'btn btn--primary', t('bank.add_account'));
  add.type = 'button';
  add.title = t('bank.add_account_title');
  add.addEventListener('click', addBankAccount);
  empty.append(iconEl(KIND_ICONS['bank-account'].icon, 'empty__icon'),
    accEl('span', 'empty__title', t('bank.no_accounts')),
    accEl('p', 'note', t('bank.no_accounts_text')),
    add);
  return empty;
}

// At midnight, today moves on along the cash flow.
everyMidnight(() => invalidateLenses('bank'));

registerLens('bank', lensEl, renderBankLens);
