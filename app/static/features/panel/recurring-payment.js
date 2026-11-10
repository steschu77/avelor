// A recurring payment's card (panel.md#a-recurring-card), two blocks of the
// entity's card (`registerCardBlock`): above its details the amount each
// time, how often, a year's and the category (`recurringPaymentSum`); under them its
// occurrences newest first, as far as its account's statements go
// (`occurrences`, model/bank.js) -- paid with the booking's chip, off with
// an offer to take the booking's amount from that day on, missed, and the
// recurring payment's other bookings as extras (`recurringPaymentOccurrences`). Neither shows on
// another entity's card. Its fields are the details' (`recurringPaymentFields`,
// features/panel/details.js).

import { accEl } from '../../core/dom.js';
import { DAY_MONTH, DAY_SHORT_MONTH, MONTH_YEAR, localDay, parseDate, signedAmount, signedMoney } from '../../core/format.js';
import { t } from '../../core/text.js';
import { entities } from '../../model/entities.js';
import { amountOn, howOften, occurrences, perYear, statementsEnd, withAmount } from '../../model/bank.js';
import { setRecurringPaymentLocal } from '../../model/structure.js';
import { selectBooking } from '../selection.js';
import { linkChip, registerCardBlock } from './panel.js';

const STATES = { paid: t('occurrence.paid'), off: t('occurrence.off'), missed: t('occurrence.missed'), extra: t('occurrence.extra') };
const STATE_COUNTS = {
  paid: n => t('occurrence.count_paid', { n }),
  off: n => t('occurrence.count_off', { n }),
  missed: n => t('occurrence.count_missed', { n }),
  extra: n => t('occurrence.count_extra', { n }),
};

// The amount each time (as of today), how often, a year's and the category.
function recurringPaymentSum(doc) {
  if (!doc?.['recurring-payment']) return null;
  const currency = entities.get(doc['recurring-payment'].account)?.['bank-account']?.currency;
  const amount = amountOn(doc, new Date()) ?? 0;
  const times = perYear(doc);
  const facts = [howOften(doc), times != null ? t('recurring.a_year', { amount: signedAmount(amount * times) }) : null, doc['recurring-payment'].category];
  return [accEl('span', 'figure figure--lg', signedMoney(amount, currency)),
    accEl('span', 'meta', facts.filter(Boolean).join(' \u00b7 '))];
}

// The occurrences, newest first, under a head counting them.
function recurringPaymentOccurrences(doc) {
  if (!doc?.['recurring-payment']) return null;
  const list = occurrences(doc);
  const end = statementsEnd(doc['recurring-payment'].account);
  const counts = Object.keys(STATES).map(state => {
    const n = list.filter(occurrence => occurrence.state === state).length;
    return n ? STATE_COUNTS[state](n) : null;
  });
  const head = accEl('div', 'occurrences__head');
  head.append(accEl('span', 'label', t('occurrence.head')),
    accEl('span', 'meta', [end ? t('occurrence.to', { day: DAY_SHORT_MONTH.format(end) }) : null, ...counts].filter(Boolean).join(' \u00b7 ')));
  if (!list.length) return [head, accEl('span', 'meta', t('occurrence.none'))];
  return [head, ...list.map(occurrence => renderOccurrence(doc, occurrence))];
}

// One occurrence: its day, its state, the booking that paid it (a chip
// that selects it), and an off one's offer while the recurring payment can be written.
function renderOccurrence(doc, { day, state, booking, statement }) {
  const row = accEl('div', 'list-row occurrence');
  const flagged = state === 'off' || state === 'missed';
  const stateEl = accEl('span', flagged ? 'chip chip--danger' : 'meta', STATES[state]);
  if (state === 'off') stateEl.title = t('occurrence.off_title');
  row.append(accEl('span', 'meta', DAY_MONTH.format(day)), stateEl);
  if (!booking) {
    row.append(accEl('span', 'meta', t('occurrence.no_booking')), accEl('span'));
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
  const take = accEl('button', 'tool label', t('occurrence.take'));
  take.type = 'button';
  take.title = t('occurrence.take_title', { day: DAY_SHORT_MONTH.format(day) });
  take.addEventListener('click', () => setRecurringPaymentLocal(doc.doc_id, withAmount(doc['recurring-payment'], localDay(day), amount)));
  const since = MONTH_YEAR.format(day);
  offer.append(accEl('span', 'note', t('occurrence.offer', { amount: signedAmount(amount), since })), take);
  return offer;
}

registerCardBlock('head', 'pane-section pane-section--ruled card-amount', recurringPaymentSum);
registerCardBlock('details', 'pane-section occurrences', recurringPaymentOccurrences);
