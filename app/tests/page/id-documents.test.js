import { equal, test } from './harness.js';
import { vault } from './vault.js';
import {
  idDocumentExtras, idDocumentType, idDocumentOf, idDocumentState, idDocumentsOf, newItemId,
} from '../../static/model/id-documents.js';

const TODAY = new Date(2027, 2, 15, 18);
const state = (expiry, until = null) => {
  const { expired, soon, short } = idDocumentState({ expiry }, until, TODAY);
  return [expired, soon, short];
};

test('an ID document runs out the day after its expiry, soon within 90 days of it', () => {
  equal(state('2027-03-14'), [true, false, false]);
  equal(state('2027-03-15'), [false, true, false]);
  equal(state('2027-06-13'), [false, true, false]);
  equal(state('2027-06-14'), [false, false, false]);
  equal(idDocumentState({ expiry: '2027-06-14' }, null, TODAY).expiry, new Date(2027, 5, 14));
});

test('an ID document is short when it runs out before a day', () => {
  equal(state('2028-07-07', new Date(2028, 6, 8, 12)), [false, false, true]);
  equal(state('2028-07-08', new Date(2028, 6, 8, 12)), [false, false, false]);
});

test('without an expiry nothing is said', () => {
  equal(idDocumentState({}, new Date(2028, 6, 8), TODAY), { expiry: null, expired: false, soon: false, short: false });
  equal(idDocumentState({ expiry: 'soon' }, null, TODAY).expiry, null);
});

test('a type in words', () => {
  equal([idDocumentType('passport'), idDocumentType('fishing licence'), idDocumentType(undefined)], ['id_document.passport', 'fishing licence', 'id_document.any']);
});

test("a holder's document in words", () => {
  equal([idDocumentOf('visa', 'Maya'), idDocumentOf('fishing licence', 'Maya'), idDocumentOf(null, 'Maya')], ['id_document.of.visa', 'id_document.of.other', 'id_document.of.any']);
});

test("a type's own fields in words", () => {
  equal(idDocumentExtras({ id: 'x', type: 'passport', number: '1', 'is-biometric': true, 'vehicle-categories': ['B', 'C'], height: 170, pages: [1] }),
    ['id_document.biometric', 'id_document.classes', 'height 170']);
  equal([idDocumentExtras({ 'is-biometric': false }), idDocumentExtras(null)], [[], []]);
});

test('the ID documents of a contact, by its id or its title', () => {
  const items = [{ id: 'x', type: 'passport' }];
  vault({ id: 'maya', kind: 'contact', title: 'Maya Yahnek', 'id-documents': items }, { id: 'alex', kind: 'contact', title: 'Alex' }, { id: 'note', kind: 'document', title: 'Note' });
  equal([idDocumentsOf('@maya').items, idDocumentsOf(' maya yahnek ').items, idDocumentsOf('Alex').items], [items, items, []]);
  equal([idDocumentsOf('Note'), idDocumentsOf('@gone'), idDocumentsOf(null)], [null, null, null]);
});

test("a new item's id is a UUIDv7 of now", () => {
  const before = Date.now();
  const id = newItemId();
  equal(/^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(id), true);
  const at = parseInt(id.slice(0, 8) + id.slice(9, 13), 16);
  equal(at >= before && at <= Date.now(), true);
});
