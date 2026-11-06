// A booking's card (panel.md#a-bookings-card): a booking of a statement,
// shown in the panel's booking view while it is selected (`selectBooking`,
// `panelMode`). Its head is who paid or was paid, the day and the
// statement's file; then the amount, the account, the other side, the
// purpose and reference, the statement and the balance after it; the
// projects it is in through its link (no x: they are the linked entity's),
// and the link, set or removed with `set_booking_link`. A booking is an
// item of its statement's file: nothing of it is renamed, edited or
// deleted here.

import { $, accEl } from '../../core/dom.js';
import { DATE_FORMAT, amountText, parseDate, signedMoney } from '../../core/format.js';
import { store } from '../../model/state.js';
import { entities, projects, projectsInOrder } from '../../model/entities.js';
import { BOOKING_KINDS, bookingOf, ledger, linkProjects, partyOf, partyRef } from '../../model/bank.js';
import { setBookingLinkLocal } from '../../model/structure.js';
import { KIND_ICONS, iconEl } from '../../ui/icons.js';
import { linkMenuChip } from '../linked.js';
import { panelMode } from '../selection.js';
import { contactChip, entityChip } from './details.js';
import { linkChip, linkTarget, projectChips } from './panel.js';

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
  const card = accEl('div', 'msg');

  const head = accEl('div', 'pane-section pane-section--ruled doc-head');
  const title = accEl('div', 'doc-head__title');
  title.append(iconEl(KIND_ICONS.transactions.icon), accEl('span', 'doc-title', partyOf(booking)));
  const sub = accEl('div', 'doc-head__sub');
  const date = parseDate(booking.date);
  const path = accEl('span', 'meta truncate-start', statement.path ?? '');
  path.title = statement.path ?? '';
  sub.append(accEl('span', 'label', date ? DATE_FORMAT.format(date) : ''), path);
  head.append(title, sub);

  const sum = accEl('div', 'pane-section pane-section--ruled card-amount');
  const status = [pending ? 'pending' : 'booked', BOOKING_KINDS.get(booking.kind)?.toLowerCase()];
  sum.append(accEl('span', 'figure figure--lg', signedMoney(amount, fields.currency)),
    accEl('span', 'meta', status.filter(Boolean).join(' \u00b7 ')));
  card.append(head, sum, renderProps(statement, booking, account));

  const to = booking['linked-to'] ?? null;
  const through = linkProjects(booking);
  if (through.length) {
    const row = accEl('div', 'pane-section msg__chips');
    const how = projects.has(to) ? 'linked to the project' : `through ${linkTarget(to)?.title || 'its link'}`;
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

// The booking's facts: its account, the other side, the purpose and the
// bank's reference, its statement, and the account's balance after it (a
// pending booking has none yet).
function renderProps(statement, booking, account) {
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
  if (booking.purpose) add('Purpose', accEl('span', 'mono mono--small', booking.purpose));
  if (booking.ref) add('Reference', accEl('span', 'mono mono--small', booking.ref));
  add('Statement', linkChip(statement.doc_id));
  if (booking.status === 'pending') {
    add('Balance', accEl('span', 'meta', 'pending: not in the balance yet'));
  } else {
    const balance = account ? ledger(account).entries.find(entry => entry.booking.id === booking.id)?.balance : null;
    add('Balance', balance != null ? amountText(balance) : '\u2014', ' ', accEl('span', 'meta', 'after this booking'));
  }
  return props;
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
