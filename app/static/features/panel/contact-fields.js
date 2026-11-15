// A contact's fields on its card (vault.md#contact, panel.md#field-edits):
// its type, its emails, phones and addresses (each a list,
// ui/text-list.js), its birthday and IBAN. While the host can write it,
// each writes by itself through `edit_entities`' `fields`
// (`setFieldsLocal`), a list written whole, an empty value removing its
// key; else as text.

import { accEl } from '../../core/dom.js';
import { DATE_FORMAT, localDay, parseDate } from '../../core/format.js';
import { t } from '../../core/text.js';
import { setFieldsLocal } from '../../model/structure.js';
import { editField, fieldInput } from '../../ui/field.js';
import { accSeg, keyedSeg } from '../../ui/segmented.js';
import { textListField } from '../../ui/text-list.js';

const CONTACT_TYPES = [['person', t('contact_type.person')], ['org', t('contact_type.org')]];
const LISTS = [
  ['email', t('contact.email'), 'email'],
  ['phone', t('contact.phone'), 'tel'],
  ['address', t('contact.address'), 'text'],
];

// ----------------------------------------------------------------------------
// The contact `doc`'s rows into the card's grid (`add(label, ...content)`,
// features/panel/details.js).
export function contactFields(doc, add) {
  const contact = doc.contact ?? {};
  const typeWord = Object.fromEntries(CONTACT_TYPES)[contact.type];
  if (!doc.writable) {
    if (typeWord) add(t('props.type'), typeWord);
    for (const [key, label] of LISTS) if (contact[key]?.length) add(label, contact[key].join(', '));
    if (contact.birthday) add(t('contact.birthday'), DATE_FORMAT.format(parseDate(contact.birthday)));
    if (contact.iban) add(t('contact.iban'), accEl('span', 'mono', contact.iban));
    return;
  }
  const set = (key, value) => setFieldsLocal(doc.doc_id, { [`contact.${key}`]: value });
  add(t('props.type'), keyedSeg(accSeg(CONTACT_TYPES, contact.type ?? 'person', type => set('type', type)), CONTACT_TYPES, 'contact-type'));
  for (const [key, label, type] of LISTS) {
    add(label, textListField(contact[key] ?? [], `contact-${key}`, values => set(key, values.length ? values : null), { type }));
  }
  const birthday = fieldInput('date', 'contact-birthday', contact.birthday ? localDay(parseDate(contact.birthday)) : '', 'form-input props__date');
  editField(birthday, day => set('birthday', day || null));
  add(t('contact.birthday'), birthday);
  const iban = fieldInput('text', 'contact-iban', contact.iban ?? '', 'form-input mono');
  iban.placeholder = t('list.add');
  editField(iban, value => set('iban', value.replace(/\s+/g, '').toUpperCase() || null));
  add(t('contact.iban'), iban);
}
