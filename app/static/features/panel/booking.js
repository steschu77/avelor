// A booking's card (panel.md#a-bookings-card): a booking of a statement,
// shown in the panel's booking view while it is selected (`selectBooking`,
// `panelMode`). Its head is who paid or was paid (a transfer's other
// account), the day and the statement's file; then the amount, the
// account, the other side, a transfer's other half with Not a transfer
// (`unpair_transfer`), the purpose and reference, the statement, the
// balance after it and its series (`set_booking_recurring`); the projects
// it is in through its link or its series (no x: they are the linked
// entity's), and the link, set or removed with `set_booking_link`. A
// booking is an item of its statement's file: nothing of it is renamed,
// edited or deleted here.

import { $, accEl } from '../../core/dom.js';
import { DATE_FORMAT, DAY_MONTH_YEAR, amountText, parseDate, signedAmount, signedMoney } from '../../core/format.js';
import { store } from '../../model/state.js';
import { entities, projects, projectsInOrder } from '../../model/entities.js';
import { BOOKING_KINDS, bookingName, bookingOf, ledger, linkProjects, pairOf, partyRef } from '../../model/bank.js';
import { amountOn, bookingDay, howOften, partyName, partyNames, seriesEnded, seriesIn, seriesOf, similarNames } from '../../model/bank.js';
import { setBookingLinkLocal, setBookingRecurringLocal, unpairTransferLocal } from '../../model/structure.js';
import { openNewSeries } from '../series-new.js';
import { armedButton } from '../../ui/confirm.js';
import { KIND_ICONS, bookingIcon, iconEl } from '../../ui/icons.js';
import { showMenu } from '../../ui/menu.js';
import { linkMenuChip } from '../linked.js';
import { panelMode } from '../selection.js';
import { contactChip, entityChip } from './details.js';
import { linkChip, linkTarget, pairChip, projectChips } from './panel.js';

const cardEl = $('booking-card');

// Drawn again while it is shown, whenever an entity, project or thread
// changes: the link set comes back as the statement's `entity` event.
function renderBookingCard(state) {
  if (panelMode(state) !== 'booking') return;
  const statement = entities.get(state.selected_entity);
  const booking = bookingOf(statement, state.selected_booking);
  const account = entities.get(statement.transactions.account);
  const fields = account?.['bank-account'] ?? {};
  const amount = booking.amount ?? 0;
  const pending = booking.status === 'pending';
  const pair = pairOf(booking);
  const card = accEl('div', 'msg');

  const head = accEl('div', 'pane-section pane-section--ruled doc-head');
  const title = accEl('div', 'doc-head__title');
  title.append(iconEl(bookingIcon(pair, seriesOf(booking))), accEl('span', 'doc-title', bookingName(booking, pair)));
  const sub = accEl('div', 'doc-head__sub');
  const date = parseDate(booking.date);
  const path = accEl('span', 'meta truncate-start', statement.path ?? '');
  path.title = statement.path ?? '';
  sub.append(accEl('span', 'label', date ? DATE_FORMAT.format(date) : ''), path);
  head.append(title, sub);

  const sum = accEl('div', 'pane-section pane-section--ruled card-amount');
  const status = [pending ? 'pending' : 'booked', pair ? 'transfer' : BOOKING_KINDS.get(booking.kind)?.toLowerCase()];
  sum.append(accEl('span', 'figure figure--lg', signedMoney(amount, fields.currency)),
    accEl('span', 'meta', status.filter(Boolean).join(' \u00b7 ')));
  card.append(head, sum, renderProps(statement, booking, account, pair));

  const to = booking['linked-to'] ?? null;
  const through = linkProjects(booking);
  if (through.length) {
    const row = accEl('div', 'pane-section msg__chips');
    const via = to ?? booking.recurring;
    const how = projects.has(to) ? 'linked to the project' : `through ${linkTarget(via)?.title || 'its link'}`;
    row.append(...projectChips(through, null), accEl('span', 'meta', how));
    card.append(row);
  }
  const links = accEl('div', 'pane-section msg__chips');
  const unlink = () => setBookingLinkLocal(statement.doc_id, booking.id, null);
  if (to) links.append(linkChip(to, { onRemove: statement.writable ? unlink : null }));
  if (statement.writable) links.append(addLinkChip(statement, booking));
  if (links.childElementCount) card.append(links);
  cardEl.replaceChildren(card);
}

// The booking's facts: its account, the other side, a transfer's other
// half (`pair`), the purpose and the bank's reference, its statement, and
// the account's balance after it (a pending booking has none yet).
function renderProps(statement, booking, account, pair) {
  const props = accEl('dl', 'pane-section props');
  const add = (label, ...content) => {
    const dd = accEl('dd');
    dd.append(...content);
    props.append(accEl('dt', 'label', label), dd);
  };
  const fields = account?.['bank-account'] ?? {};
  const amount = booking.amount ?? 0;
  if (account) {
    add('Account', entityChip(account.id, account.title || 'Untitled'),
      accEl('span', 'meta', [fields.institution, fields.number].filter(Boolean).join(' \u00b7 ')));
  }
  const other = partyRef(booking);
  if (other) add(amount < 0 ? 'To' : 'From', contactChip(other));
  if (pair) add('Transfer', ...renderPair(statement, booking, pair));
  if (booking.purpose) add('Purpose', accEl('span', 'mono mono--small', booking.purpose));
  if (booking.ref) add('Reference', accEl('span', 'mono mono--small', booking.ref));
  add('Statement', linkChip(statement.doc_id));
  if (booking.status === 'pending') {
    add('Balance', accEl('span', 'meta', 'pending: not in the balance yet'));
  } else {
    const balance = account ? ledger(account).entries.find(entry => entry.booking.id === booking.id)?.balance : null;
    add('Balance', balance != null ? amountText(balance) : '\u2014', ' ', accEl('span', 'meta', 'after this booking'));
  }
  const series = renderSeries(statement, booking);
  if (series.length) add('Series', ...series);
  return props;
}

// The booking's series (vault.md#recurring): a chip that selects it, with
// an x while the statement can be written; then "+", the series menu.
function renderSeries(statement, booking) {
  const current = seriesOf(booking)?.id ?? null;
  const set = to => setBookingRecurringLocal([{ id: statement.doc_id, booking: booking.id }], to);
  const parts = [];
  if (current) parts.push(linkChip(current, { onRemove: statement.writable ? () => set(null) : null }));
  if (statement.writable) parts.push(addSeriesChip(statement, booking, current, set));
  return parts;
}

// The "+" after the series: a menu of the account's series running on the
// booking's day (an ended one isn't offered), its party's first (the same
// name, or a similar one, `similarNames`), then New recurring payment, the
// form filled from the booking (features/series-new.js).
function addSeriesChip(statement, booking, current, set) {
  const add = accEl('button', 'chip chip--action', '+');
  add.type = 'button';
  add.title = current ? 'Move to another series' : 'Add to a series';
  add.addEventListener('click', () => {
    const account = statement.transactions.account;
    const day = bookingDay(booking) ?? new Date();
    const name = partyName(booking);
    const party = name.trim().toLowerCase();
    const running = seriesIn(new Set([account])).filter(series => series.id !== current && !seriesEnded(series, day));
    const isParty = series => [...partyNames(series)].some(name => name === party || similarNames(name, party));
    const item = series => ({
      label: series.title || 'Untitled',
      icon: KIND_ICONS.recurring.icon,
      hint: [howOften(series), signedAmount(amountOn(series, day) ?? 0)].join(' \u00b7 '),
      run: () => set(series.id),
    });
    const theirs = running.filter(isParty);
    const others = running.filter(series => !isParty(series));
    const items = [];
    if (theirs.length) items.push({ head: name || 'Its party' }, ...theirs.map(item));
    if (others.length) items.push({ head: entities.get(account)?.title || 'The account' }, ...others.map(item));
    items.push({
      label: 'New recurring payment\u2026', icon: '#i-add', hint: 'from this booking', separator: items.length > 0,
      run: () => openNewSeries(statement, booking),
    });
    showMenu(add, items);
  });
  return add;
}

// A transfer's other half: a chip of its account that selects it, its
// amount and day, and Not a transfer, armed by a first click, while both
// statements can be written.
function renderPair(statement, booking, pair) {
  const other = pair.booking;
  const chip = pairChip(pair);
  const day = parseDate(other.date);
  const when = accEl('span', 'meta', [signedAmount(other.amount ?? 0), day ? `on ${DAY_MONTH_YEAR.format(day)}` : null]
    .filter(Boolean).join(' '));
  if (!statement.writable || !pair.statement.writable) return [chip, when];
  const unpair = accEl('button', 'tool label', 'Not a transfer');
  unpair.type = 'button';
  unpair.title = 'Count both halves in In and Out again';
  armedButton(unpair, 'Click again to unpair', () => unpairTransferLocal(statement.doc_id, booking.id),
    `unpair_transfer ${booking.id}`);
  return [chip, when, ' ', unpair];
}

// The "+" after the link: a menu with a filter of what the booking can be
// linked to -- the active projects, then the newest entities and threads
// in the vault panel's filter (`linkedResults`), the search finding older
// ones -- but its statement and what it links to. A pick replaces the link.
function addLinkChip(statement, booking) {
  const current = booking['linked-to'] ?? null;
  const link = to => setBookingLinkLocal(statement.doc_id, booking.id, to);
  const projectItems = () => projectsInOrder().filter(p => !p.archived && p.id !== current)
    .map(p => ({ label: p.name, icon: KIND_ICONS.project.icon, run: () => link(p.id) }));
  return linkMenuChip(current ? 'Link to something else' : 'Link to what it paid for', {
    target: statement.doc_id,
    current,
    before: () => [{ head: 'Projects' }, ...projectItems()],
    onPick: link,
  });
}

store.subscribe([
  state => state.selected_lens, state => state.selected_entity, state => state.selected_booking,
  state => state.entities_rev, state => state.projects_rev, state => state.threads_rev,
], (_values, state) => renderBookingCard(state));
