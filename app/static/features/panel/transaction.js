// A transaction's card (panel.md#transaction-card): a transaction of a statement,
// shown in the panel's transaction view while it is selected (`selectTransaction`,
// `panelMode`). Its head is who paid or was paid (a transfer's other
// account), the day and the statement's file; then the amount, the
// account, the other side, a transfer's other half with Not a transfer
// (`unpair_transfer`), the memo and reference, the statement and the
// balance after it; the projects it is in through its link (no x: they are
// the linked entity's), and the link, set or removed with
// `set_transaction_link`. A transaction is an item of its statement's file: nothing
// of it is renamed, edited or deleted here.

import { $, accEl } from '../../core/dom.js';
import { DATE_FORMAT, DAY_MONTH_YEAR, amountText, parseDate, signedAmount, signedMoney } from '../../core/format.js';
import { t } from '../../core/text.js';
import { store } from '../../model/state.js';
import { entities, projects, projectsInOrder } from '../../model/entities.js';
import { TRANSACTION_TYPES, transactionName, transactionOf, ledger, linkProjects, pairOf, partyRef } from '../../model/bank.js';
import { setTransactionLinkLocal, unpairTransferLocal } from '../../model/structure.js';
import { armedButton } from '../../ui/confirm.js';
import { KIND_ICONS, TRANSFER_ICON, iconEl } from '../../ui/icons.js';
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
  title.append(iconEl(pair ? TRANSFER_ICON : KIND_ICONS.statement.icon), accEl('span', 'doc-title', transactionName(transaction, pair)));
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
    const how = projects.has(to) ? t('transaction.linked_project') : `through ${linkTarget(to)?.title || t('transaction.its_link')}`;
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
// as the bank gave them, a transfer's other half (`pair`), the memo and
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
  if (transaction.memo) add(t('props.memo'), accEl('span', 'mono mono--small', transaction.memo));
  if (transaction.ref) add(t('props.reference'), accEl('span', 'mono mono--small', transaction.ref));
  add(t('props.statement'), linkChip(statement.doc_id));
  if (transaction.status === 'pending') {
    add(t('props.balance'), accEl('span', 'meta', t('transaction.pending_note')));
  } else {
    const balance = account ? ledger(account).entries.find(entry => entry.transaction.id === transaction.id)?.balance : null;
    add(t('props.balance'), balance != null ? amountText(balance) : '\u2014', ' ', accEl('span', 'meta', t('transaction.after_this')));
  }
  return props;
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
