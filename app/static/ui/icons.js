// The icons (symbols of the page's sprite, build.rs): an entity's by its
// kind, a document's by its genre, a trip's event by what part it is
// (`docIcon`), and `iconEl`, an icon made in a script.

import { t } from '../core/text.js';

const SVG_NS = 'http://www.w3.org/2000/svg';

// ----------------------------------------------------------------------------
// Keyed by the derived `DocumentType` (kebab-case, see av-vault envelope.rs).
export const KIND_ICONS = {
  document: { icon: '#i-note' },
  message: { icon: '#i-chat' },
  email: { icon: '#i-mention' },
  conversation: { icon: '#i-chat' },
  thread: { icon: '#i-chat' },
  project: { icon: '#i-folder' },
  statement: { icon: '#i-payment' },
  'bank-account': { icon: '#i-payment' },
  'recurring-payment': { icon: '#i-arrow-sync' },
  calendar: { icon: '#i-calendar' },
  event: { icon: '#i-calendar' },
  task: { icon: '#i-checkmark' },
  contact: { icon: '#i-person' },
  record: { icon: '#i-lock-closed' },
  list: { icon: '#i-task-list-ltr' },
};

// An entity's kind in words (the conflict dialog, a card's head).
const KIND_NAMES = {
  document: t('kind.document'),
  message: t('kind.message'),
  email: t('kind.email'),
  conversation: t('kind.conversation'),
  thread: t('kind.thread'),
  project: t('kind.project'),
  statement: t('kind.statement'),
  'bank-account': t('kind.bank_account'),
  'recurring-payment': t('kind.recurring_payment'),
  calendar: t('kind.calendar'),
  event: t('kind.event'),
  task: t('kind.task'),
  contact: t('kind.contact'),
  record: t('kind.record'),
  list: t('kind.list'),
};
export const kindName = kind => KIND_NAMES[kind] ?? kind;

// ----------------------------------------------------------------------------
const DOC_SUB_ICONS = {
  note: { icon: '#i-note' },
  letter: { icon: '#i-mail' },
  contract: { icon: '#i-lock-closed' },
  bill: { icon: '#i-task-list-ltr' },
  memo: { icon: '#i-payment' },
  report: { icon: '#i-contact-card' },
  invoice: { icon: '#i-contact-card' },
  statement: { icon: '#i-document-table' },
};

// ----------------------------------------------------------------------------
// A trip's events (vault.md#event): a transport's by its way, a stay's, a
// layover's, an activity's.
export const WAY_ICONS = {
  flight: '#i-airplane',
  train: '#i-vehicle-subway',
  bus: '#i-vehicle-bus',
  ferry: '#i-vehicle-ship',
  car: '#i-vehicle-car',
};
const TRIP_ICONS = { stay: '#i-bed', activity: '#i-ticket-diagonal', layover: '#i-hourglass-half' };

// ----------------------------------------------------------------------------
// The icon of an entity `doc` (its `kind`, `genre`, `event`): a trip's event
// by what part it is, a document by its genre if it has one, else its
// kind's.
export function docIcon(doc) {
  const kind = doc?.kind;
  const part = kind === 'event' ? doc.event?.kind : null;
  if (part === 'transport') return WAY_ICONS[doc.event.way] ?? KIND_ICONS.event.icon;
  if (part) return TRIP_ICONS[part] ?? KIND_ICONS.event.icon;
  if (kind === 'document' && doc.genre) return DOC_SUB_ICONS[doc.genre]?.icon || KIND_ICONS.document.icon;
  return KIND_ICONS[kind]?.icon || '';
}

export const IMPORT_ICON = '#i-arrow-import';

// A booking that is a transfer between two of the vault's accounts
// (`pairOf`, model/bank.js): its row and card.
export const TRANSFER_ICON = '#i-arrow-swap';

// A booking's icon, its row's and card's: a transfer's (`pair`), a
// recurring payment's (`recurringPayment`), else a payment's.
export function bookingIcon(pair, recurringPayment) {
  return pair ? TRANSFER_ICON : recurringPayment ? KIND_ICONS['recurring-payment'].icon : KIND_ICONS.statement.icon;
}

// ----------------------------------------------------------------------------
const IMAGE_TYPES = ['jpg', 'jpeg', 'png', 'gif', 'webp', 'heic', 'bmp', 'tif', 'tiff'];

// The icon of a file by its name's extension: a PDF's, an image's, else a
// paper clip (a document's original, vault.md#document).
export function fileIcon(name) {
  const dot = name.lastIndexOf('.');
  const ext = dot < 0 ? '' : name.slice(dot + 1).toLowerCase();
  if (ext === 'pdf') return '#i-document-pdf';
  if (IMAGE_TYPES.includes(ext)) return '#i-image';
  return '#i-attach';
}

// ----------------------------------------------------------------------------
// An `<svg class="icon">` showing the symbol `href` (`#i-<name>`): the
// markup's icons are written as such, a script's are made here, since an
// element made with `accEl` is no SVG element.
export function iconEl(href, className) {
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('class', className ? `icon ${className}` : 'icon');
  const use = document.createElementNS(SVG_NS, 'use');
  use.setAttribute('href', href);
  svg.append(use);
  return svg;
}
