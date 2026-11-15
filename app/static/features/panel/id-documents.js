// A contact's ID documents (panel.md#id-documents): a part of its card
// after its toolbar (`registerCardBlock`), its head saying what models read
// of them and + ID document; a row each by its type -- the number (Copy),
// the type's own fields, the issuer and country, what it is linked to, the
// expiry at the end (marked when it runs out soon or has); one that runs
// out before a trip its holder travels on ends, the trip and the task to
// renew it, with Open (`tripsShortOf`, model/trip.js). While the host can
// write the contact a click on a row opens its fields under it, with
// Link... and Remove. Each writes the contact's own fields by path
// (`setFieldsLocal`), an item's as `id-documents.<its id>.<key>`.

import { accEl } from '../../core/dom.js';
import { pressable } from '../../core/dom.js';
import { titled } from '../../core/dom.js';
import { DATE_FORMAT, parseDate, relativeDay } from '../../core/format.js';
import { t } from '../../core/text.js';
import { entities, vaultSettings } from '../../model/entities.js';
import { ID_DOCUMENT_TYPES, idDocumentExtras, idDocumentIcon, idDocumentType, idDocumentState, newItemId } from '../../model/id-documents.js';
import { setFieldsLocal } from '../../model/structure.js';
import { tripsShortOf } from '../../model/trip.js';
import { armedButton } from '../../ui/confirm.js';
import { editField, fieldInput } from '../../ui/field.js';
import { docIcon, iconEl } from '../../ui/icons.js';
import { showMenu } from '../../ui/menu.js';
import { toolButton } from '../../ui/toolbar.js';
import { selectById } from '../selection.js';
import { registerCardBlock } from './panel.js';

// The item whose fields are open: `{ contact, key }`, null for none.
let editing = null;

// How long Copy says it copied.
const COPIED_MS = 1500;

// What models read of the ID documents, by the contact's classification
// (assistant.md#context): the head's words and their tooltip.
const REACH = {
  open: [t('id_documents.reach_open'), t('id_documents.reach_open_why')],
  local: [t('id_documents.reach_local'), t('id_documents.reach_local_why')],
  sealed: [t('id_documents.reach_none'), t('id_documents.reach_none_why')],
};
REACH.masked = REACH.open;

// The part on a contact's card: its head, then a row per document, the
// open one's fields under it; none on a contact the host can't write that
// holds none.
function idDocumentsPart(doc, redraw) {
  if (!doc?.contact && !doc?.['id-documents']) return null;
  const items = doc['id-documents'] ?? [];
  if (!items.length && !doc.writable) return null;
  const [note, why] = REACH[doc.classification ?? vaultSettings().classification ?? 'open'] ?? REACH.sealed;
  const privacy = accEl('span', 'meta private', t('id_documents.private', { note }));
  privacy.prepend(iconEl('#i-lock-closed'));
  privacy.title = why;
  const head = accEl('div', 'card-part__head');
  head.append(accEl('span', 'label', t('id_documents.head')), privacy);
  if (doc.writable) head.append(addButton(doc));
  const short = tripsShortOf(doc.doc_id);
  return [head, ...items.flatMap((item, i) => idDocumentEls(doc, item, item.id ?? String(i), short.get(String(item.id)) ?? [], redraw))];
}

registerCardBlock('parts', 'pane-section pane-section--ruled card-part', idDocumentsPart);

const linksOf = item => [].concat(item['linked-to'] ?? []).map(String);
const pathOf = (key, field) => (field ? `id-documents.${key}.${field}` : `id-documents.${key}`);

// ----------------------------------------------------------------------------
// The item's row: its type's icon, the type over its number and the rest,
// its links, the `trips` it runs out before (the warning for its icon);
// its dates at the end. A click on it opens its fields, or closes them.
function idDocumentEls(doc, item, key, trips, redraw) {
  const open = doc.writable && editing?.contact === doc.doc_id && editing.key === key;
  const row = accEl('div', ['id-document', trips.length && 'id-document--gap', doc.writable && 'is-writable'].filter(Boolean).join(' '));
  const main = accEl('span', 'id-document__main', idDocumentType(item.type));
  const line = accEl('span', 'id-document__line');
  if (item.number) line.append(...numberEls(String(item.number)));
  const about = [...idDocumentExtras(item), item['issued-by'], item.country].filter(Boolean).join(' \u00b7 ');
  if (about) line.append(accEl('span', 'meta id-document__about', about));
  if (line.childNodes.length) main.append(line);
  const links = linksOf(item);
  if (links.length) {
    const chips = accEl('span', 'id-document__links');
    chips.append(...links.map(id => linkEl(doc, item, key, id)));
    main.append(chips);
  }
  if (trips.length) main.append(gapEl(trips));
  row.append(iconEl(trips.length ? '#i-warning' : idDocumentIcon(item.type)), main, endEl(item, trips[0]));
  if (!doc.writable) return [row];
  row.title = open ? t('id_documents.close_fields') : t('id_documents.edit_fields');
  pressable(row).dataset.focusKey = `id-document:${key}`;
  row.addEventListener('click', e => {
    if (e.target.closest('button, input')) return;
    editing = open ? null : { contact: doc.doc_id, key };
    redraw();
  });
  return open ? [row, formEl(doc, item, key, redraw)] : [row];
}

// The number, and Copy, which puts it on the clipboard and says so.
function numberEls(number) {
  const copy = toolButton(t('id_documents.copy'), t('id_documents.copy_title'), async () => {
    await copyText(number);
    copy.textContent = t('id_documents.copied');
    setTimeout(() => {
      copy.textContent = t('id_documents.copy');
    }, COPIED_MS);
  });
  return [accEl('span', 'id-document__number', number), copy];
}

// `text` on the clipboard: by the Clipboard API, else (a page it isn't
// offered to) by copying it selected in a textarea out of sight; the focus
// stays where it was.
async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const active = document.activeElement;
    const area = accEl('textarea', 'id-document__copy');
    area.value = text;
    document.body.append(area);
    area.select();
    document.execCommand('copy');
    area.remove();
    active?.focus?.({ preventScroll: true });
  }
}

// "Runs out before Summer with Oma 2027 ends 07/18/2027 \u00b7 Renew Maya's
// passport, due 03/15/2027", the soonest trip; how many more.
function gapEl([{ trip, end, task }, ...more]) {
  const parts = [t('id_documents.runs_out_before', { trip: trip.name || t('untitled'), end: DATE_FORMAT.format(end) })];
  const title = task?.title || t('untitled');
  if (task) parts.push(task.due && !isNaN(task.due) ? t('id_documents.task_due', { title, due: DATE_FORMAT.format(task.due) }) : title);
  if (more.length) parts.push(t('id_documents.more_trips', { n: more.length }));
  return accEl('span', 'meta id-document__gap', parts.join(' \u00b7 '));
}

// Its expiry ("to 09/02/2027", "ran out 01/02/2026"), then with a trip it
// runs out before Open, the task to renew it or else the trip; else "runs
// out in 30 days" once `SOON_DAYS` are left, else when it was issued.
function endEl(item, gap) {
  const end = accEl('span', 'id-document__end');
  const state = idDocumentState(item);
  if (state.expiry) {
    const day = DATE_FORMAT.format(state.expiry);
    end.append(state.expired ? accEl('span', 'meta is-expired', t('id_documents.ran_out', { day }))
      : accEl('span', 'meta', t('id_documents.valid_to', { day })));
  }
  if (gap) {
    const target = gap.task ? gap.task.doc_id : gap.trip.id;
    const title = gap.task ? t('id_documents.open_task') : t('id_documents.open_trip');
    end.append(toolButton(t('context.open'), title, () => selectById(target)));
    return end;
  }
  const issued = parseDate(item.issued ? String(item.issued).slice(0, 10) : null);
  if (state.soon) end.append(accEl('span', 'meta is-soon', t('id_documents.runs_out', { when: relativeDay(state.expiry) })));
  else if (issued && !isNaN(issued)) end.append(accEl('span', 'meta', t('id_documents.issued_on', { day: DATE_FORMAT.format(issued) })));
  return end;
}

// What the item links to: a chip that selects it, x unlinking it.
function linkEl(doc, item, key, id) {
  const target = entities.get(id);
  const chip = accEl('span', 'chip chip--link');
  const name = accEl('button', 'chip__name', target?.title || t('untitled'));
  name.type = 'button';
  name.addEventListener('click', () => selectById(id));
  chip.append('\u2192', iconEl(target ? docIcon(target) : ''), name);
  if (doc.writable) {
    const remove = accEl('button', 'chip__remove', '\u00d7');
    remove.type = 'button';
    titled(remove, t('id_documents.unlink'));
    remove.addEventListener('click', () => {
      const rest = linksOf(item).filter(other => other !== id);
      setFieldsLocal(doc.doc_id, { [pathOf(key, 'linked-to')]: rest.length ? rest : null });
    });
    chip.append(remove);
  }
  return chip;
}

// ----------------------------------------------------------------------------
// The item's fields under its row: its type (the app's offered as it is
// typed), number, country, issued, expiry and issuer, each written by
// itself on Enter or leaving it; then Link..., Remove and Done.
function formEl(doc, item, key, redraw) {
  const set = (field, value) => setFieldsLocal(doc.doc_id, { [pathOf(key, field)]: value || null });
  const field = (name, label, type) => {
    const value = item[name] == null ? '' : String(item[name]);
    const input = fieldInput(type, `id-document:${key}:${name}`, type === 'date' ? value.slice(0, 10) : value, 'form-input');
    editField(input, text => set(name, type === 'date' ? text : text.trim()));
    const el = accEl('label', 'id-document__field');
    el.append(accEl('span', 'label', label), input);
    return [el, input];
  };
  const [typeEl, type] = field('type', t('props.type'), 'text');
  type.setAttribute('list', typeList.id);
  const fields = [typeEl, field('number', t('id_documents.number'), 'text')[0],
    field('country', t('id_documents.country'), 'text')[0], field('issued', t('id_documents.issued'), 'date')[0],
    field('expiry', t('id_documents.expiry'), 'date')[0], field('issued-by', t('id_documents.issued_by'), 'text')[0]];
  const link = toolButton(t('id_documents.link'), t('id_documents.link_title'), btn => linkMenu(btn, doc, item, key));
  link.dataset.focusKey = `id-document:${key}:link`;
  const remove = toolButton(t('id_documents.remove'), t('id_documents.remove_title'), () => {});
  armedButton(remove, t('id_documents.remove_armed'), () => {
    editing = null;
    setFieldsLocal(doc.doc_id, { [pathOf(key)]: null });
  }, `id-document:${key}`);
  const done = toolButton(t('id_documents.done'), t('id_documents.close_fields'), () => {
    editing = null;
    redraw();
  });
  const acts = accEl('div', 'id-document__acts');
  acts.append(link, remove, done);
  const form = accEl('div', 'id-document__form');
  form.append(...fields, acts);
  return form;
}

// The types the app names, offered as a type is typed.
const typeList = accEl('datalist');
typeList.id = 'id-document-types';
typeList.append(...ID_DOCUMENT_TYPES.map(([value]) => {
  const option = accEl('option');
  option.value = value;
  return option;
}));
document.body.append(typeList);

// The documents and tasks to link the item to, newest first, a filter
// finding any.
function linkMenu(anchor, doc, item, key) {
  const linked = new Set(linksOf(item));
  const candidates = [...entities.values()]
    .filter(other => ['document', 'task', 'email'].includes(other.kind) && !linked.has(other.doc_id))
    .sort((a, b) => (b.date?.getTime?.() ?? 0) - (a.date?.getTime?.() ?? 0));
  const itemOf = other => ({
    label: other.title || t('untitled'), icon: docIcon(other),
    run: () => setFieldsLocal(doc.doc_id, { [pathOf(key, 'linked-to')]: [...linked, other.doc_id] }),
  });
  showMenu(anchor, candidates.slice(0, 20).map(itemOf), {
    head: t('id_documents.link_head', { type: idDocumentType(item.type) }),
    filter: {
      placeholder: t('id_documents.filter_entries'),
      extra: text => candidates.slice(20)
        .filter(other => (other.title ?? '').toLowerCase().includes(text.toLowerCase())).slice(0, 20).map(itemOf),
    },
  });
}

// + ID document: a type picked, or typed, adds an item with a new id and
// opens its fields.
function addButton(doc) {
  const btn = toolButton(t('id_documents.add'), t('id_documents.add_title'), () => showMenu(btn,
    ID_DOCUMENT_TYPES.map(([type, label]) => ({ label, icon: idDocumentIcon(type), run: () => addIdDocument(doc, type) })),
    {
      head: t('id_documents.new_for', { name: doc.title || t('untitled') }),
      filter: { placeholder: t('id_documents.another_type'), extra: text => [{ label: t('id_documents.add_type', { type: text }), run: () => addIdDocument(doc, text) }] },
    }));
  btn.dataset.focusKey = 'id-document-add';
  return btn;
}

function addIdDocument(doc, type) {
  const id = newItemId();
  const items = doc['id-documents'] ?? [];
  setFieldsLocal(doc.doc_id, items.length ? { [pathOf(id)]: { type } } : { 'id-documents': [{ id, type }] });
  editing = { contact: doc.doc_id, key: id };
}
