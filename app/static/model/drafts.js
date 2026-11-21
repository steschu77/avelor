// The draft (panel.md#draft): an entity not made yet, which the side
// panel's draft card shows (features/panel/draft.js) and New starts
// (`openDraft`, features/new.js). One at a time, in the page only, until
// Create makes it (`createDraft`, one `create_entity`) or Discard drops it;
// another selection hides it (features/selection.js), New's menu shows it
// again (`showDraft`), and a vault switch drops it. The card's writes reach
// it through model/structure.js for its id (`isDraft`, `changeDraft`), by
// the host's rules for `edit_entities` (api.md#structure), never refused:
// the host checks the whole at Create.

import { isoInstant, localDay, zonedInstant } from '../core/format.js';
import { rpcCall, rpcOn } from '../core/rpc.js';
import { projectCalendar } from './blueprint.js';
import { writableCalendar } from './calendar.js';
import { entityOf, projectScope } from './entities.js';
import { newItemId } from './id-documents.js';
import { store } from './state.js';
import { LOCAL_ZONE } from './trip.js';

// The role blocks `create_entity` takes of a draft: a document's From and
// To in its message block.
const ROLES = ['document', 'message', 'task', 'event', 'contact', 'list', 'record'];

// { meta: the entity as its `entity` event would carry it, doc: as the
// page keeps it (`entityOf`) }, or none
let draft = null;

// ----------------------------------------------------------------------------
// A draft of `kind` (document, task, event, contact, list, record), shown at
// once, in place of a kept one: as New made them, in the project selected
// in the sidebar, an event today from 9:00 to 10:00 in this PC's zone, in
// that project's calendar if it can take new events. `prefill`: an event's
// `day`, or fields of its block (`event`: a gap's stay or journey,
// features/gaps.js).
export function openDraft(kind, prefill = {}) {
  const scope = projectScope();
  const meta = {
    id: newItemId(), kind, title: '', path: '', source: null, type: null,
    date: isoInstant(new Date()), projects: scope ? [scope] : [], labels: [], linked_to: null,
    starred: false, archived: false, threads: [], editable: true, writable: true, deletable: false,
  };
  if (kind === 'document') {
    meta.type = 'note';
    meta.document = { type: 'note' };
  } else if (kind === 'task') {
    meta.task = { status: 'todo' };
  } else if (kind === 'event') {
    const start = zonedInstant(prefill.day ?? localDay(new Date()), '09:00', LOCAL_ZONE);
    meta.event = { start: isoInstant(start), end: isoInstant(new Date(start.getTime() + 3600e3)), tz: LOCAL_ZONE };
    const calendar = scope ? writableCalendar(projectCalendar(scope)) : null;
    if (calendar) meta.event.calendar = calendar;
    Object.assign(meta.event, prefill.event);
    // a day only has no zone
    if (meta.event.start.length === 10) delete meta.event.tz;
    meta.date = meta.event.start;
  } else if (kind === 'contact') {
    meta.contact = { type: 'person' };
  } else if (kind === 'list') {
    meta.list = { items: [] };
  } else {
    meta.record = {};
    meta.fields = {};
  }
  draft = { meta, doc: entityOf(meta) };
  showDraft();
}

// Show the kept draft in the side panel, opening it if it is closed; an
// open vault page, which covers the panel, closes.
export function showDraft() {
  if (!draft) return;
  const { panel_size, drafts_rev } = store.getState();
  store.setState({
    selected_draft: draft.meta.id, drafts_rev: drafts_rev + 1, vault_expanded: false,
    ...(panel_size === 'closed' && { panel_size: 'normal' }),
  });
}

// Drop the draft.
export function dropDraft() {
  draft = null;
  store.setState({ selected_draft: null, drafts_rev: store.getState().drafts_rev + 1 });
}

// The draft as the page keeps it, if there is one.
export function currentDraft() {
  return draft?.doc ?? null;
}

// Whether `id` is the draft's.
export function isDraft(id) {
  return Boolean(draft) && draft.meta.id === id;
}

// The draft, if `id` is its.
export function draftDoc(id) {
  return isDraft(id) ? draft.doc : null;
}

// ----------------------------------------------------------------------------
// Apply `change` to the draft: what `edit_entities` takes (`{event}`,
// `{task}`, `{type}`, `{date}`, `{fields}` by path, null or empty
// removing a key, an event's start also its date, a task done when it was
// marked), and what the other commands set: `title`, `labels`, `projects`,
// `linked_to`.
export function changeDraft(change) {
  if (!draft) return;
  const { meta } = draft;
  for (const key of ['title', 'labels', 'projects', 'linked_to', 'date']) {
    if (key in change) meta[key] = change[key];
  }
  if ('type' in change) {
    meta.type = change.type || null;
    meta.document = merged(meta.document, { type: change.type });
  }
  if (change.event) {
    meta.event = merged(meta.event, change.event);
    if (change.event.start) meta.date = change.event.start;
  }
  if (change.task) {
    meta.task = merged(meta.task, change.task);
    if (change.task.status === 'done') meta.task.done = isoInstant(new Date());
    else if (change.task.status) delete meta.task.done;
  }
  for (const [path, value] of Object.entries(change.fields ?? {})) setPath(meta, path, value);
  draft.doc = entityOf(meta);
  store.setState({ drafts_rev: store.getState().drafts_rev + 1 });
}

// `block` with `fields` set, each null or empty one removed.
function merged(block, fields) {
  const next = { ...block };
  for (const [key, value] of Object.entries(fields)) {
    if (value === null || value === '') delete next[key];
    else next[key] = value;
  }
  return next;
}

// A path of `fields`: `role.key` into its role's block; a plain `key` the
// entity's own field (a record's), `classification` the entity's.
function setPath(meta, path, value) {
  const [first, ...rest] = path.split('.');
  if (!rest.length && first === 'classification') {
    meta.classification = value ?? undefined;
  } else if (!rest.length) {
    meta.fields = merged(meta.fields, { [first]: value });
  } else {
    meta[first] = merged(meta[first], { [rest.join('.')]: value });
  }
}

// ----------------------------------------------------------------------------
// Make the draft (`create_entity`, api.md#structure), its body `content`
// (markdown; none for a list or a record), and drop it: the new entity's
// id. Refused, the draft stays as it is.
export async function createDraft(content) {
  const { meta } = draft;
  const roles = Object.fromEntries(ROLES.filter(role => meta[role]).map(role => [role, meta[role]]));
  const args = { roles, title: meta.title.trim(), projects: meta.projects, labels: meta.labels };
  if (meta.linked_to) args.linked_to = meta.linked_to;
  if (meta.classification) args.classification = meta.classification;
  if (meta.kind !== 'event') args.date = meta.date;
  if (meta.kind === 'record' && Object.keys(meta.fields ?? {}).length) args.fields = meta.fields;
  if (!['list', 'record'].includes(meta.kind) && content.trim()) args.content = content;
  const { id } = await rpcCall('create_entity', args);
  dropDraft();
  return id;
}

// Another vault: the draft was this one's.
rpcOn('vault', ({ state }) => {
  if (state === 'indexing' && draft) dropDraft();
});
