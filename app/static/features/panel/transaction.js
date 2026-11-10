// A transaction's card (panel.md#transaction-card): a transaction of a statement,
// shown in the panel's transaction view while it is selected (`selectTransaction`,
// `panelMode`). Its head is who paid or was paid (a transfer's other
// account), the day and the statement's file; then the amount, the
// account, the other side, a transfer's other half with Not a transfer
// (`unpair_transfer`), the memo and reference, the statement, the
// balance after it and its recurring payment (`set_transaction_recurring_payment`); the projects
// it is in through its link or its recurring payment (no x: they are the linked
// entity's), and the link, set or removed with `set_transaction_link`. A
// transaction is an item of its statement's file: nothing of it is renamed,
// edited or deleted here.

import { $, accEl } from '../../core/dom.js';
import { titled } from '../../core/dom.js';
import { DATE_FORMAT, DAY_MONTH_YEAR, amountText, parseDate, signedAmount, signedMoney } from '../../core/format.js';
import { t } from '../../core/text.js';
import { store } from '../../model/state.js';
import { entities, projects, projectsInOrder } from '../../model/entities.js';
import { TRANSACTION_TYPES, transactionName, transactionOf, ledger, linkProjects, pairCandidates, pairOf, partyRef } from '../../model/bank.js';
import { amountOn, transactionDay, howOften, partyName, partyNames, recurringPaymentEnded, recurringPaymentsIn, recurringPaymentOf, similarNames } from '../../model/bank.js';
import { pairTransferLocal, setTransactionLinkLocal, setTransactionRecurringPaymentLocal, unpairTransferLocal } from '../../model/structure.js';
import { openNewRecurringPayment } from '../recurring-payment-new.js';
import { armedButton } from '../../ui/confirm.js';
import { KIND_ICONS, transactionIcon, iconEl } from '../../ui/icons.js';
import { showMenu } from '../../ui/menu.js';
import { linkMenuChip } from '../linked.js';
import { panelMode } from '../selection.js';
import { contactChip, entityChip } from './details.js';
import { linkChip, linkTarget, pairChip, projectChips } from './panel.js';

const cardEl = $('transaction-card');

// Drawn again while it is shown, whenever an entity, project or thread
// changes: the link set comes back as the statement's `entity` event.
function renderTransactionCard(state) {
  if (panelMode(state) !== 'transaction') return;
  const statement = entities.get(state.selected_entity);
  const transaction = transactionOf(statement, state.selected_transaction);
  const account = entities.get(statement.statement.account);
  const fields = account?.['bank-account'] ?? {};
  const amount = transaction.amount ?? 0;
  const pending = transaction.status === 'pending';
  const pair = pairOf(transaction);
  const card = accEl('div', 'msg');

  const head = accEl('div', 'pane-section pane-section--ruled doc-head');
  const title = accEl('div', 'doc-head__title');
  title.append(iconEl(transactionIcon(pair, recurringPaymentOf(transaction))), accEl('span', 'doc-title', transactionName(transaction, pair)));
  const sub = accEl('div', 'doc-head__sub');
  const date = parseDate(transaction.date);
  const path = accEl('span', 'meta truncate-start', statement.path ?? '');
  path.title = statement.path ?? '';
  sub.append(accEl('span', 'label', date ? DATE_FORMAT.format(date) : ''), path);
  head.append(title, sub);

  const sum = accEl('div', 'pane-section pane-section--ruled card-amount');
  const status = [pending ? t('transaction.pending') : t('transaction.posted'), pair ? t('transaction.transfer') : TRANSACTION_TYPES.get(transaction.type)];
  sum.append(accEl('span', 'figure figure--lg', signedMoney(amount, fields.currency)),
    accEl('span', 'meta', status.filter(Boolean).join(' \u00b7 ')));
  card.append(head, sum, renderProps(statement, transaction, account, pair));

  const to = transaction['linked-to'] ?? null;
  const through = linkProjects(transaction);
  if (through.length) {
    const row = accEl('div', 'pane-section msg__chips');
    const via = to ?? transaction['recurring-payment'];
    const how = projects.has(to) ? t('transaction.linked_project')
      : t('transaction.through', { name: linkTarget(via)?.title || t('transaction.its_link') });
    row.append(...projectChips(through, null), accEl('span', 'meta', how));
    card.append(row);
  }
  const links = accEl('div', 'pane-section msg__chips');
  const unlink = () => setTransactionLinkLocal(statement.doc_id, transaction.id, null);
  if (to) links.append(linkChip(to, { onRemove: statement.writable ? unlink : null }));
  if (statement.writable) links.append(addLinkChip(statement, transaction));
  if (links.childElementCount) card.append(links);
  cardEl.replaceChildren(card);
}

// The transaction's facts: its account, the other side with its IBAN and BIC
// as the bank gave them, a transfer's other half (`pair`; else, while there
// are halves it may pair with, their menu), the memo and
// the bank's reference, its statement, and the account's balance after it
// (a pending transaction has none yet).
function renderProps(statement, transaction, account, pair) {
  const props = accEl('dl', 'pane-section props');
  const add = (label, ...content) => {
    const dd = accEl('dd');
    dd.append(...content);
    props.append(accEl('dt', 'label', label), dd);
  };
  const fields = account?.['bank-account'] ?? {};
  const amount = transaction.amount ?? 0;
  if (account) {
    add(t('props.account'), entityChip(account.id, account.title || t('untitled')),
      accEl('span', 'meta', [fields.bank, fields.number].filter(Boolean).join(' \u00b7 ')));
  }
  const other = partyRef(transaction);
  if (other) add(amount < 0 ? t('props.to') : t('props.from'), contactChip(other));
  if (transaction.iban) add(t('props.iban'), accEl('span', 'mono mono--small', transaction.iban));
  if (transaction.bic) add(t('props.bic'), accEl('span', 'mono mono--small', transaction.bic));
  if (pair) add(t('props.transfer'), ...renderPair(statement, transaction, pair));
  else if (statement.writable && transaction.status !== 'pending') {
    const halves = pairCandidates(statement, transaction).filter(half => half.statement.writable);
    if (halves.length) add(t('props.transfer'), addPairChip(statement, transaction, halves));
  }
  if (transaction.memo) add(t('props.memo'), accEl('span', 'mono mono--small', transaction.memo));
  if (transaction.ref) add(t('props.reference'), accEl('span', 'mono mono--small', transaction.ref));
  add(t('props.statement'), linkChip(statement.doc_id));
  if (transaction.status === 'pending') {
    add(t('props.balance'), accEl('span', 'meta', t('transaction.pending_note')));
  } else {
    const balance = account ? ledger(account).entries.find(entry => entry.transaction.id === transaction.id)?.balance : null;
    add(t('props.balance'), balance != null ? amountText(balance) : '\u2014', ' ', accEl('span', 'meta', t('transaction.after_this')));
  }
  const chips = renderRecurringPayment(statement, transaction);
  if (chips.length) add(t('props.recurring_payment'), ...chips);
  return props;
}

// The transaction's recurring payment (vault.md#recurring): a chip that selects it, with
// an x while the statement can be written; then "+", the recurring payment menu.
function renderRecurringPayment(statement, transaction) {
  const current = recurringPaymentOf(transaction)?.id ?? null;
  const set = to => setTransactionRecurringPaymentLocal([{ id: statement.doc_id, transaction: transaction.id }], to);
  const parts = [];
  if (current) parts.push(linkChip(current, { onRemove: statement.writable ? () => set(null) : null }));
  if (statement.writable) parts.push(addRecurringPaymentChip(statement, transaction, current, set));
  return parts;
}

// The "+" after the recurring payment: a menu of the account's recurring payments running on the
// transaction's day (an ended one isn't offered), its party's first (the same
// name, or a similar one, `similarNames`), then New recurring payment, the
// form filled from the transaction (features/recurring-payment-new.js).
function addRecurringPaymentChip(statement, transaction, current, set) {
  const add = accEl('button', 'chip chip--action', '+');
  add.type = 'button';
  titled(add, current ? t('transaction.move_recurring') : t('transaction.add_recurring'));
  add.addEventListener('click', () => {
    const account = statement.statement.account;
    const day = transactionDay(transaction) ?? new Date();
    const name = partyName(transaction);
    const party = name.trim().toLowerCase();
    const running = recurringPaymentsIn(new Set([account])).filter(recurringPayment => recurringPayment.id !== current && !recurringPaymentEnded(recurringPayment, day));
    const isParty = recurringPayment => [...partyNames(recurringPayment)].some(name => name === party || similarNames(name, party));
    const item = recurringPayment => ({
      label: recurringPayment.title || t('untitled'),
      icon: KIND_ICONS['recurring-payment'].icon,
      hint: [howOften(recurringPayment), signedAmount(amountOn(recurringPayment, day) ?? 0)].join(' \u00b7 '),
      run: () => set(recurringPayment.id),
    });
    const theirs = running.filter(isParty);
    const others = running.filter(recurringPayment => !isParty(recurringPayment));
    const items = [];
    if (theirs.length) items.push({ head: name || t('transaction.its_party') }, ...theirs.map(item));
    if (others.length) items.push({ head: entities.get(account)?.title || t('transaction.the_account') }, ...others.map(item));
    items.push({
      label: t('transaction.new_recurring'), icon: '#i-add', hint: t('transaction.from_this'), separator: items.length > 0,
      run: () => openNewRecurringPayment(statement, transaction),
    });
    showMenu(add, items);
  });
  return add;
}

// The "+" of a transaction that is no transfer's half: a menu of the `halves`
// it may pair with (`pairCandidates`), each its account, amount and day;
// a pick pairs them (`pair_transfer`).
function addPairChip(statement, transaction, halves) {
  const add = accEl('button', 'chip chip--action', '+');
  add.type = 'button';
  titled(add, t('transaction.pair_title'));
  add.addEventListener('click', () => showMenu(add, halves.map(half => ({
    label: half.account?.title || t('untitled'),
    icon: KIND_ICONS['bank-account'].icon,
    hint: [signedAmount(half.transaction.amount ?? 0), DAY_MONTH_YEAR.format(transactionDay(half.transaction))].join(' \u00b7 '),
    run: () => pairTransferLocal(statement.doc_id, transaction.id, { id: half.statement.doc_id, transaction: half.transaction.id }),
  })), { head: t('transaction.pair_head') }));
  return add;
}

// A transfer's other half: a chip of its account that selects it, its
// amount and day, and Not a transfer, armed by a first click, while both
// statements can be written.
function renderPair(statement, transaction, pair) {
  const other = pair.transaction;
  const chip = pairChip(pair);
  const day = parseDate(other.date);
  const when = accEl('span', 'meta', [signedAmount(other.amount ?? 0), day ? t('transaction.on_day', { day: DAY_MONTH_YEAR.format(day) }) : null]
    .filter(Boolean).join(' '));
  if (!statement.writable || !pair.statement.writable) return [chip, when];
  const unpair = accEl('button', 'tool label', t('transaction.not_transfer'));
  unpair.type = 'button';
  unpair.title = t('transaction.unpair_title');
  armedButton(unpair, t('transaction.unpair_armed'), () => unpairTransferLocal(statement.doc_id, transaction.id),
    `unpair_transfer ${transaction.id}`);
  return [chip, when, ' ', unpair];
}

// The "+" after the link: a menu with a filter of what the transaction can be
// linked to -- the active projects, then the newest entities and threads
// in the vault panel's filter (`linkedResults`), the search finding older
// ones -- but its statement and what it links to. A pick replaces the link.
function addLinkChip(statement, transaction) {
  const current = transaction['linked-to'] ?? null;
  const link = to => setTransactionLinkLocal(statement.doc_id, transaction.id, to);
  const projectItems = () => projectsInOrder().filter(p => !p.archived && p.id !== current)
    .map(p => ({ label: p.name, icon: KIND_ICONS.project.icon, run: () => link(p.id) }));
  return linkMenuChip(current ? t('transaction.link_else') : t('transaction.link_paid_for'), {
    target: statement.doc_id,
    current,
    before: () => [{ head: t('sidebar.projects') }, ...projectItems()],
    onPick: link,
  });
}

store.subscribe([
  state => state.selected_lens, state => state.selected_entity, state => state.selected_transaction,
  state => state.entities_rev, state => state.projects_rev, state => state.threads_rev,
], (_values, state) => renderTransactionCard(state));
