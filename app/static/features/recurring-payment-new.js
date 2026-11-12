// New recurring payment (vault.md#recurring): the form a transaction's Recurring payment
// menu opens (features/panel/transaction.js), filled from the transaction, in the
// expanded vault area while its op is "recurring-payment" (panel.md#a-new-recurring):
// the title (the party's name), the account and party, the schedule (a
// period and a week's days or its day, `scheduleLine`), from and until, the amount, how far it
// may vary, the category, what it is linked to (the contract or policy it
// comes from), its projects and labels, and the transactions it starts with --
// the account's transactions of the party not in a recurring payment, ticked, and those of
// a similar name (`similarNames`) to tick by hand. From follows the first
// ticked transaction until it is set. Create sends `create_recurring_payment`, then
// `set_transaction_recurring_payment` for the ticked transactions, and selects the recurring payment.

import { accEl } from '../core/dom.js';
import { titled } from '../core/dom.js';
import { newGroup, rpcCall } from '../core/rpc.js';
import { DAY_MONTH, amountText, localDay, parseAmount, parseDate, signedAmount } from '../core/format.js';
import { t } from '../core/text.js';
import { entities } from '../model/entities.js';
import { PERIODS, scheduleLine } from '../model/schedule.js';
import { transactionDay, partyName, similarNames, statementsOf } from '../model/bank.js';
import { setTransactionRecurringPaymentLocal } from '../model/structure.js';
import { KIND_ICONS, iconEl } from '../ui/icons.js';
import { closeVaultArea, openVaultOp, registerVaultPage, replaceVaultPage, setPageFocus } from '../ui/page.js';
import { linkMenuChip } from './linked.js';
import { draftChips, newDateInput, newTextInput } from './new.js';
import { offerCategories } from './panel/details.js';
import { scheduleField } from './panel/schedule-field.js';
import { linkChip, linkTarget } from './panel/panel.js';
import { selectDocument } from './selection.js';
import { watchFormGrid } from './workspace/folders.js';

let draft = null; // the form shown

// Open the form for the transaction `transaction` of the statement `statement`.
export function openNewRecurringPayment(statement, transaction) {
  const account = statement.statement.account;
  const party = partyName(transaction);
  const day = transactionDay(transaction) ?? new Date();
  const d = {
    account, party,
    title: party,
    rule: { period: 'month', days: null, day: day.getDate() },
    from: localDay(day),
    fromSet: false,
    until: '',
    amount: String(transaction.amount ?? ''),
    vary: '',
    category: '',
    linked: null,
    projects: [],
    labels: [],
    candidates: candidatesOf(account, party, transaction),
    ticked: new Set([transactionKey(statement.doc_id, transaction.id)]),
    busy: false,
    error: null,
    els: {},
  };
  draft = d;
  openVaultOp('recurring-payment', t('rp_new.head'));
  setPageFocus('recurring-payment:title');
  replaceVaultPage(renderForm(d));
  renderChips(d);
  renderTicks(d);
  renderFoot(d);
}

registerVaultPage('recurring-payment', {
  close: closeVaultArea,
  submit: () => {
    if (draft) createRecurringPayment(draft);
  },
});

const transactionKey = (statement, transaction) => `${statement}/${transaction}`;

// The account's transactions not in a recurring payment that go the way `transaction` goes:
// those of its party (`same`), then those of a similar name, newest first;
// `{ key, statement, transaction, day, same }` each.
function candidatesOf(account, party, transaction) {
  const name = party.trim().toLowerCase();
  const found = [];
  for (const statement of statementsOf(account)) {
    for (const item of statement.statement.transactions ?? []) {
      if (item['recurring-payment'] || (item.amount < 0) !== (transaction.amount < 0)) continue;
      const itemName = partyName(item);
      const other = itemName.trim().toLowerCase();
      const same = other === name || item.id === transaction.id;
      if (!same && !similarNames(other, name)) continue;
      const day = transactionDay(item) ?? new Date();
      found.push({ key: transactionKey(statement.doc_id, item.id), statement, transaction: item, day, same, name: itemName });
    }
  }
  return found.sort((a, b) => b.day - a.day);
}

// ----------------------------------------------------------------------------
// The form, built once; its chips, ticks and foot are built again.
function renderForm(d) {
  const sheet = accEl('section', 'new__sheet');
  const kind = accEl('h2', 'pane-title new__kind');
  kind.append(iconEl(KIND_ICONS['recurring-payment'].icon), t('props.recurring_payment'));
  const onInput = () => renderFoot(d);
  const title = newTextInput(d, 'title', t('rp_new.what_for'), { className: 'input new__title', onInput });
  title.dataset.focusKey = 'recurring-payment:title';
  const path = accEl('span', 'mono mono--small truncate-start new__path', 'Accounts\\');
  path.title = t('new.path_title');

  const [rows, row] = watchFormGrid();
  rows.classList.add('new__rows');
  const account = entities.get(d.account);
  row(t('props.account'), accEl('span', 'chip', account?.title || t('untitled')),
    accEl('span', 'meta', account?.['bank-account']?.number ?? ''));
  row(t('props.party'), newTextInput(d, 'party', t('rp_new.party_placeholder'), { onInput }), accEl('span', 'note', t('rp_new.party_note')));

  const schedule = row(t('props.schedule'));
  const renderEvery = () => {
    schedule.replaceChildren(scheduleField(d.rule, {
      periods: PERIODS, from: parseDate(d.from) ?? new Date(), key: 'recurring-payment:schedule',
      bind: (input, set) => input.addEventListener('change', () => set(input.value)),
      onChange: rule => {
        d.rule = rule;
        renderEvery();
        renderFoot(d);
      },
    }));
  };
  renderEvery();

  d.els.from = newDateInput(d.from, value => {
    d.from = value;
    d.fromSet = true;
    renderFoot(d);
  });
  row(t('props.from'), d.els.from, accEl('span', 'meta', t('rp_new.from_note')));
  const until = newDateInput(d.until, value => { d.until = value; });
  row(t('props.until'), until, accEl('span', 'meta', t('rp_new.until_note')));
  const amount = newTextInput(d, 'amount', amountText(0), { className: 'form-input recurring-payment-amount', onInput });
  row(t('props.amount'), amount, accEl('span', 'meta', t('rp_new.amount_note')));
  const vary = newTextInput(d, 'vary', amountText(0), { className: 'form-input recurring-payment-amount', onInput });
  row(t('props.varies'), vary, accEl('span', 'note', t('rp_new.vary_note')));
  const category = newTextInput(d, 'category', t('watch.none'), { onInput });
  offerCategories(category);
  row(t('props.category'), category);
  d.els.linked = row(t('bank.linked_to'));
  d.els.chips = row(t('new.projects_labels'));

  const head = accEl('div', 'section-head new__body-head', t('props.transactions'));
  head.append(accEl('span', 'section-head__note', t('rp_new.ticked_join')));
  d.els.ticks = accEl('div', 'recurring-payment-ticks');
  d.els.error = accEl('div', 'error-note new__error');
  d.els.foot = accEl('div', 'new__foot');
  sheet.append(kind, accEl('span', 'label new__title-label', t('new.label.title')), title, path, rows, head, d.els.ticks, d.els.error, d.els.foot);
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
  const add = linkMenuChip(t('rp_new.link_title'), { current: d.linked, onPick: link });
  d.els.linked.replaceChildren(...(d.linked && linkTarget(d.linked) ? [linkChip(d.linked, { onRemove: () => link(null) })] : []), add,
    accEl('span', 'note', t('rp_new.link_note')));
  draftChips(d, d.els.chips, () => renderChips(d));
}

// The transactions to start with: the party's, then those of a similar name
// with Tick all; each with its check, day, name, memo and amount.
function renderTicks(d) {
  const row = candidate => {
    const el = accEl('div', 'list-row recurring-payment-tick');
    const on = d.ticked.has(candidate.key);
    const check = accEl('button', 'check', on ? '\u2713' : '');
    check.type = 'button';
    check.setAttribute('aria-pressed', String(on));
    titled(check, on ? t('rp_new.leave_out') : t('rp_new.add_it'));
    check.addEventListener('click', () => {
      if (on) d.ticked.delete(candidate.key);
      else d.ticked.add(candidate.key);
      renderTicks(d);
      renderFoot(d);
    });
    const { transaction } = candidate;
    const cell = accEl('span', 'bank__cell');
    cell.append(accEl('span', 'row-title', candidate.name || transaction.title || ''),
      accEl('span', 'meta bank__detail', transaction.memo || transaction.title || ''));
    el.append(check, accEl('span', 'meta', DAY_MONTH.format(candidate.day)), cell,
      accEl('span', 'bank__amount', signedAmount(transaction.amount ?? 0)));
    return el;
  };
  const same = d.candidates.filter(candidate => candidate.same);
  const similar = d.candidates.filter(candidate => !candidate.same);
  const parts = same.map(row);
  if (similar.length) {
    const head = accEl('div', 'recurring-payment-ticks__head');
    const all = accEl('button', 'tool label push', t('rp_new.tick_all'));
    all.type = 'button';
    all.addEventListener('click', () => {
      for (const candidate of similar) d.ticked.add(candidate.key);
      renderTicks(d);
      renderFoot(d);
    });
    const names = [...new Set(similar.map(candidate => candidate.name))].join(', ');
    head.append(accEl('span', 'label', t('rp_new.similar_names')), accEl('span', 'meta', `${names} \u00b7 ${similar.length}`), all);
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
  if (!d.title.trim()) return t('new.enter_title');
  if (!d.from) return t('rp_new.choose_first_day');
  if (d.rule.period !== 'week' && !(d.rule.day >= 1 && d.rule.day <= 31)) return t('rp_new.choose_month_day');
  if (parseAmount(d.amount) == null) return t('rp_new.enter_amount');
  if (d.vary.trim() && parseAmount(d.vary) == null) return t('rp_new.enter_vary');
  return '';
}

// Create and Cancel, the transactions ticked, and the error of a failed Create.
function renderFoot(d) {
  const why = blocked(d);
  const create = accEl('button', 'btn btn--primary', d.busy ? t('new.creating') : t('rp_new.create'));
  create.type = 'button';
  create.disabled = !!why || d.busy;
  create.title = why || t('rp_new.create_title');
  create.addEventListener('click', () => createRecurringPayment(d));
  const cancel = accEl('button', 'btn btn--ghost', t('field.cancel'));
  cancel.type = 'button';
  cancel.addEventListener('click', closeVaultArea);
  const n = d.ticked.size;
  const statements = new Set([...d.ticked].map(key => key.split('/')[0])).size;
  const note = n ? t('rp_new.foot_with', { transactions: t('bank.count.transactions', { n }), statements: t('bank.count.statements', { n: statements }) })
    : t('rp_new.foot');
  d.els.foot.replaceChildren(create, cancel, accEl('span', 'meta', note));
  d.els.error.textContent = d.error ?? '';
  d.els.error.hidden = !d.error;
}

function createRecurringPayment(d) {
  if (d.busy || blocked(d)) return;
  const from = parseDate(d.from);
  const amount = parseAmount(d.amount);
  const recurring = {
    account: d.account,
    party: d.party.trim() || undefined,
    schedule: [scheduleLine(d.rule, from)],
    from: d.from,
    until: d.until || undefined,
    amounts: [{ from: d.from, amount }],
    vary: d.vary.trim() ? Math.abs(parseAmount(d.vary)) : undefined,
    category: d.category.trim() || undefined,
  };
  const args = { title: d.title.trim(), recurring_payment: recurring, projects: d.projects, labels: d.labels };
  if (d.linked) args.linked_to = d.linked;
  d.busy = true;
  d.error = null;
  renderFoot(d);
  // The payment and its transactions joining it are one step.
  const step = newGroup('recurring-payment');
  rpcCall('create_recurring_payment', args, { step }).then(({ id }) => {
    const transactions = [...d.ticked].map(key => {
      const [statement, transaction] = key.split('/');
      return { id: statement, transaction };
    });
    if (transactions.length) setTransactionRecurringPaymentLocal(transactions, id, step);
    if (draft === d) draft = null;
    closeVaultArea();
    selectDocument(id);
  }, e => {
    d.busy = false;
    d.error = e.message;
    renderFoot(d);
  });
}
