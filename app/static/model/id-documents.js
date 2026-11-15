// A person's ID documents (vault.md#id-documents): the `id-documents` list
// of their contact, each item's type in words and its icon, its type's own
// fields, and how its expiry stands -- what a contact's card shows
// (features/panel/id-documents.js), what a trip checks of its travelers
// (`tripGaps`, model/trip.js), and later People's household tiles.

import { dayStart, daysBetween, parseDate } from '../core/format.js';
import { t } from '../core/text.js';
import { entities, parseContact } from './entities.js';

// The types the app names, as the vault keeps them and in words; any other
// is kept as written.
export const ID_DOCUMENT_TYPES = [
  ['id-card', t('id_document.id_card')],
  ['passport', t('id_document.passport')],
  ['driving-license', t('id_document.driving_license')],
  ['residence-permit', t('id_document.residence_permit')],
  ['visa', t('id_document.visa')],
];

// How many days before its expiry a document is said to run out soon.
const SOON_DAYS = 90;

export function idDocumentType(type) {
  return ID_DOCUMENT_TYPES.find(([value]) => value === type)?.[1] ?? (type || t('id_document.any'));
}

// `holder`'s document of `type` in a phrase: "Anna's passport".
const ID_DOCUMENT_OF = {
  'id-card': holder => t('id_document.of.id_card', { holder }),
  passport: holder => t('id_document.of.passport', { holder }),
  'driving-license': holder => t('id_document.of.driving_license', { holder }),
  'residence-permit': holder => t('id_document.of.residence_permit', { holder }),
  visa: holder => t('id_document.of.visa', { holder }),
};

export function idDocumentOf(type, holder) {
  if (!type) return t('id_document.of.any', { holder });
  return ID_DOCUMENT_OF[type]?.(holder) ?? t('id_document.of.other', { holder, type });
}

export function idDocumentIcon(type) {
  return type === 'driving-license' ? '#i-vehicle-car' : '#i-contact-card';
}

// The fields every item may have; any other is a type's own.
const ITEM_FIELDS = new Set(['id', 'type', 'number', 'country', 'issued', 'expiry', 'issued-by', 'linked-to']);

// The type's own fields of `item` in words: a licence's classes ("class C"),
// "biometric", any other as its name and value; a list or map left out.
export function idDocumentExtras(item) {
  return Object.entries(item ?? {}).filter(([key]) => !ITEM_FIELDS.has(key)).map(([key, value]) => {
    if (key === 'vehicle-categories') return t('id_document.classes', { classes: [].concat(value).join(', ') });
    if (key === 'is-biometric') return value === true ? t('id_document.biometric') : null;
    return value != null && typeof value !== 'object' ? `${key} ${value}` : null;
  }).filter(Boolean);
}

// The contact a ContactRef names (`@<id>`, or a contact's title), if the
// vault has one.
function contactOf(ref) {
  const { id, name } = parseContact(String(ref ?? ''));
  if (id) return entities.get(id) ?? null;
  const wanted = name.trim().toLowerCase();
  return [...entities.values()].find(doc => doc.kind === 'contact' && (doc.title ?? '').trim().toLowerCase() === wanted) ?? null;
}

// The ID documents of the contact `ref` names: `{ holder, items }`, none
// if no contact.
export function idDocumentsOf(ref) {
  const holder = contactOf(ref);
  return holder ? { holder, items: holder['id-documents'] ?? [] } : null;
}

// How the document `item` stands: its expiry (a Date, null without one),
// whether it has run out by `today`, runs out within `SOON_DAYS`, and
// runs out before `until` (a trip's last day; none, no such question).
export function idDocumentState(item, until = null, today = new Date()) {
  const expiry = parseDate(item?.expiry ? String(item.expiry).slice(0, 10) : null) ?? null;
  if (!expiry || isNaN(expiry)) return { expiry: null, expired: false, soon: false, short: false };
  const left = daysBetween(dayStart(today), expiry);
  return {
    expiry,
    expired: left < 0,
    soon: left >= 0 && left <= SOON_DAYS,
    short: !!until && expiry < dayStart(until),
  };
}

// A new item's id: a UUIDv7, its time now.
export function newItemId() {
  const hex = Date.now().toString(16).padStart(12, '0');
  const rand = n => Array.from({ length: n }, () => Math.floor(Math.random() * 16).toString(16)).join('');
  const variant = (8 + Math.floor(Math.random() * 4)).toString(16);
  return `${hex.slice(0, 8)}-${hex.slice(8)}-7${rand(3)}-${variant}${rand(3)}-${rand(12)}`;
}
