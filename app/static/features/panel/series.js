// A recurring payment's card (panel.md#a-series-card), two blocks of the
// entity's card (`registerCardBlock`): above its details the amount each
// time, how often, a year's and the category (`seriesSum`); under them its
// occurrences newest first, as far as its account's statements go
// (`occurrences`, model/bank.js) -- paid with the booking's chip, off with
// an offer to take the booking's amount from that day on, missed, and the
// series' other bookings as extras (`seriesOccurrences`). Neither shows on
// another entity's card. Its fields are the details' (`seriesFields`,
// features/panel/details.js).

import { accEl } from '../../core/dom.js';
import { DAY_MONTH, DAY_SHORT_MONTH, MONTH_YEAR, localDay, parseDate, signedAmount, signedMoney } from '../../core/format.js';
import { entities } from '../../model/entities.js';
import { amountOn, howOften, occurrences, perYear, statementsEnd, withAmount } from '../../model/bank.js';
import { setRecurringLocal } from '../../model/structure.js';
import { selectBooking } from '../selection.js';
import { linkChip, registerCardBlock } from './panel.js';

const STATES = { paid: 'Paid', off: 'Off', missed: 'Missed', extra: 'Extra' };

// The amount each time (as of today), how often, a year's and the category.
function seriesSum(doc) {
  if (!doc?.recurring) return null;
  const currency = entities.get(doc.recurring.account)?.['bank-account']?.currency;
  const amount = amountOn(doc, new Date()) ?? 0;
  const times = perYear(doc);
  const facts = [howOften(doc), times != null ? `${signedAmount(amount * times)} a year` : null, doc.recurring.category];
  return [accEl('span', 'figure figure--lg', signedMoney(amount, currency)),
    accEl('span', 'meta', facts.filter(Boolean).join(' \u00b7 '))];
}

// The occurrences, newest first, under a head counting them.
function seriesOccurrences(doc) {
  if (!doc?.recurring) return null;
  const list = occurrences(doc);
  const end = statementsEnd(doc.recurring.account);
  const counts = Object.keys(STATES).map(state => {
    const n = list.filter(occurrence => occurrence.state === state).length;
    return n ? `${n} ${STATES[state].toLowerCase()}` : null;
  });
  const head = accEl('div', 'occurrences__head');
  head.append(accEl('span', 'label', 'Occurrences'),
    accEl('span', 'meta', [end ? `to ${DAY_SHORT_MONTH.format(end)}` : null, ...counts].filter(Boolean).join(' \u00b7 ')));
  if (!list.length) return [head, accEl('span', 'meta', 'None as far as the statements go.')];
  return [head, ...list.map(occurrence => renderOccurrence(doc, occurrence))];
}

// One occurrence: its day, its state, the booking that paid it (a chip
// that selects it), and an off one's offer while the series can be written.
function renderOccurrence(doc, { day, state, booking, statement }) {
  const row = accEl('div', 'list-row occurrence');
  const flagged = state === 'off' || state === 'missed';
  const stateEl = accEl('span', flagged ? 'chip chip--danger' : 'meta', STATES[state]);
  if (state === 'off') stateEl.title = 'Off the amount by more than it may vary';
  row.append(accEl('span', 'meta', DAY_MONTH.format(day)), stateEl);
  if (!booking) {
    row.append(accEl('span', 'meta', 'no booking'), accEl('span'));
    return row;
  }
  const paidOn = parseDate(booking.date);
  const label = [paidOn ? DAY_SHORT_MONTH.format(paidOn) : null, signedAmount(booking.amount ?? 0)].filter(Boolean).join(' \u00b7 ');
  row.append(linkChip(statement.doc_id, { label, onSelect: () => selectBooking(statement.doc_id, booking.id) }), accEl('span'));
  if (state === 'off' && doc.writable) row.append(renderOffer(doc, day, booking.amount ?? 0));
  return row;
}

// "-72.15 since November 2026: take it as the amount?" and Take it, which
// adds the amount from the occurrence's day on (`withAmount`).
function renderOffer(doc, day, amount) {
  const offer = accEl('span', 'occurrence__offer');
  const take = accEl('button', 'tool label', 'Take it');
  take.type = 'button';
  take.title = `The amount from ${DAY_SHORT_MONTH.format(day)} on`;
  take.addEventListener('click', () => setRecurringLocal(doc.doc_id, withAmount(doc.recurring, localDay(day), amount)));
  offer.append(accEl('span', 'note', `${signedAmount(amount)} since ${MONTH_YEAR.format(day)}: take it as the amount?`), take);
  return offer;
}

registerCardBlock('head', 'pane-section pane-section--ruled card-amount', seriesSum);
registerCardBlock('details', 'pane-section occurrences', seriesOccurrences);
