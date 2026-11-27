// New: a menu of the types the app makes -- at the vault panel's New, or
// Ctrl+N -- each a draft card in the side panel (model/drafts.js,
// panel.md#draft), a draft kept from before offered first; and the Task
// tab of the panel's context column, a new task for what the panel shows
// (a card's Task button opens it), whose Create sends `create_entity`
// (api.md#structure): the host writes the file into the tasks' folder and
// pushes the entity. The tab's form is built once, so its inputs keep
// focus; only its chips, the notes and the buttons are built again.
// "+ Task from this" of an assistant reply runs the `task.with` action
// (`openTaskWith`).

import { $, accEl } from '../core/dom.js';
import { rpcCall, rpcOn } from '../core/rpc.js';
import { localDay, parseDate, relativeDay } from '../core/format.js';
import { registerAction, shortcutText } from '../core/actions.js';
import { t } from '../core/text.js';
import { store, vaultOpen } from '../model/state.js';
import { currentDraft, openDraft, showDraft } from '../model/drafts.js';
import { projectsFor } from '../model/entities.js';
import { docIcon, iconEl } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import { accSeg } from '../ui/segmented.js';
import {
  contextTarget, createdNote, registerContextTab, selectContextTab,
  shownContextTab,
} from './context.js';
import { TASK_STATUSES } from './panel/details.js';
import {
  addChip, linkChip, linkTarget, projectChips, labelChips,
} from './panel/panel.js';
import { watchFormGrid } from './workspace/folders.js';

// The types New makes: their derived kind, what Go to calls it (`command`,
// `also`), the folder the host writes them into (shown only; av-vault's
// `schema::layout` decides), and how their title is asked for; also the
// draft card's (features/panel/draft.js).
export const NEW_TYPES = [
  {
    kind: 'document', name: t('kind.document'), command: t('new.document'), also: t('new.document_also'), folder: 'Documents', label: t('new.label.title'), hint: t('new.label.title'),
    missing: t('new.enter_title'), create: t('new.create.document'), createTitle: t('new.create_title.document'),
  },
  {
    kind: 'task', name: t('kind.task'), command: t('new.task'), folder: 'Tasks', label: t('new.label.title'), hint: t('new.hint.task'),
    missing: t('new.enter_title'), create: t('new.create.task'), createTitle: t('new.create_title.task'),
  },
  {
    kind: 'event', name: t('kind.event'), command: t('day.new_event'), folder: 'Calendar\\Created Here', label: t('new.label.title'), hint: t('new.label.title'),
    missing: t('new.enter_title'), create: t('new.create.event'), createTitle: t('new.create_title.event'),
  },
  {
    kind: 'contact', name: t('kind.contact'), command: t('new.contact'), also: t('new.contact_also'), folder: 'Contacts', label: t('docs.name'), hint: t('docs.name'),
    missing: t('new.enter_name'), create: t('new.create.contact'), createTitle: t('new.create_title.contact'),
  },
  {
    kind: 'list', name: t('kind.list'), command: t('new.list'), folder: 'Lists', label: t('new.label.title'), hint: t('new.hint.list'),
    missing: t('new.enter_title'), create: t('new.create.list'), createTitle: t('new.create_title.list'),
  },
  {
    kind: 'record', name: t('kind.record'), command: t('new.record'), folder: 'Records', label: t('new.label.title'), hint: t('new.label.title'),
    missing: t('new.enter_title'), create: t('new.create.record'), createTitle: t('new.create_title.record'),
  },
];

function newTypeIcon(type) {
  return docIcon({ kind: type.kind, type: type.kind === 'document' ? 'note' : null });
}

// A new draft of `kind` (one of NEW_TYPES), the Task tab's: `linked`: the
// entity a task is made for; `projects`: the projects it
// starts in; `scope`: the project selected in the sidebar, noted as such
// while it is the only one; `onCreated(id)`: what its host does once it is
// created.
function newDraft(kind, { linked = null, projects = [], scope = null, onCreated }) {
  return {
    type: NEW_TYPES.find(t => t.kind === kind),
    title: '',
    status: 'todo',
    due: '',
    projects: [...projects],
    scope,
    labels: [],
    linked, // the entity it is `linked-to`
    body: '',
    created: null, // { id, title } of the one made before
    error: null, // why Create failed
    busy: false, // Create is under way
    onCreated,
    els: {}, // the parts built again: chips, linked, error, foot
  };
}

// ----------------------------------------------------------------------------
// the menu

registerAction({
  id: 'new', group: 'new', title: t('new.menu'), icon: '#i-add', shortcut: 'Ctrl+N',
  when: vaultOpen,
  run: anchor => showNewMenu(anchor ?? $('new-item')),
});
for (const type of NEW_TYPES) {
  registerAction({
    id: `new.a_${type.kind}`, group: 'new', title: type.command, icon: newTypeIcon(type), also: type.also,
    when: vaultOpen,
    run: () => openDraft(type.kind),
  });
}

// New event on the day `day` (the day view's, features/panel/panel.js).
registerAction({
  id: 'new.event',
  when: vaultOpen,
  run: day => openDraft('event', { day: localDay(day) }),
});

// The types as a menu's items, each a draft card in the side panel
// (model/drafts.js); `prefill`: the draft's (`openDraft`), a project's New
// here its project (features/projects.js).
export function newTypeItems(prefill = {}) {
  return NEW_TYPES.map(type => ({
    label: type.name,
    icon: newTypeIcon(type),
    hint: type.folder.split('\\')[0],
    run: () => openDraft(type.kind, prefill),
  }));
}

// The types at `anchor`; a kept draft first, shown again.
function showNewMenu(anchor) {
  const items = newTypeItems();
  const kept = currentDraft();
  if (kept) items.unshift({ label: t('draft.back', { title: kept.title || t('untitled') }), icon: docIcon(kept), run: showDraft });
  showMenu(anchor, items, { head: t('new.head') });
}

// ----------------------------------------------------------------------------
// the Task tab: a new task for what the panel shows

const taskTabEl = $('context-task');
const taskDrafts = new Map(); // what a task is for ('' for nothing) -> its draft
let taskShown = null; // the draft the tab shows

registerContextTab({ id: 'task', title: t('kind.task'), el: taskTabEl, show: focus => renderTaskTab(focus) });

registerAction({
  id: 'task.create', title: t('new.create_task_action'), shortcut: 'Ctrl+Enter',
  when: () => shownContextTab() === 'task' && !!taskShown && taskTabEl.contains(document.activeElement),
  run: () => createNew(taskShown),
});

// The draft for `target`, kept while the app runs: linked to it, in its
// projects (`projectsFor`). Once created, a fresh one takes its place,
// noting the one made.
function taskDraft(target) {
  let d = taskDrafts.get(target);
  if (d) return d;
  d = newDraft('task', {
    linked: target || null,
    ...projectsFor(target),
    onCreated: id => {
      taskDrafts.delete(target);
      taskDraft(target).created = { id, title: d.title.trim() };
      if (taskShown === d) renderTaskTab(true);
    },
  });
  taskDrafts.set(target, d);
  return d;
}

// Show the form of what the panel shows; the same draft's form stays as it
// is (and keeps the focus). `focus`: the title takes it.
function renderTaskTab(focus = false) {
  const d = taskDraft(contextTarget(store.getState()));
  if (d !== taskShown) {
    taskShown = d;
    taskTabEl.replaceChildren(...renderNewForm(d));
    renderNewChips(d);
    renderNewFoot(d);
  }
  if (focus) taskTabEl.querySelector('.new__title')?.focus({ preventScroll: true });
}

// An assistant reply's "+ Task from this" (features/assistant/assistant.js,
// the `task.with` action): the Task tab for
// what the panel shows, with `markdown` in its body, below what is typed
// there already; `personal` (a reply showing personal values) makes the
// task `masked`. The context column is open beside the chat already.
function openTaskWith(markdown, personal = false) {
  const d = taskDraft(contextTarget(store.getState()));
  d.body = d.body.trim() ? `${d.body.trimEnd()}\n\n${markdown}` : markdown;
  if (personal) d.classification = 'masked';
  taskShown = null; // the form is built again, with the body
  selectContextTab('task', true);
}
registerAction({ id: 'task.with', title: t('new.task_with'), run: openTaskWith });

store.subscribe(contextTarget, () => {
  if (shownContextTab() === 'task') renderTaskTab();
});

// Another vault opened (`forgetVault`): the drafts named the old vault's
// entities and projects.
rpcOn('vault', ({ state }) => {
  if (state !== 'indexing') return;
  taskDrafts.clear();
  if (shownContextTab() === 'task') renderTaskTab();
});

// ----------------------------------------------------------------------------
// creating

// Why the draft can't be created yet, or '' if it can.
function newBlocked(d) {
  return d.title.trim() ? '' : d.type.missing;
}

// The task's block (vault.md#task): what is left empty is left out.
function newRoles(d) {
  const task = { status: d.status };
  if (d.due) task.due = d.due;
  return { task };
}

// Create the draft `d` (Create, Ctrl+Enter); the host pushes the new
// entity, and its host goes on (`onCreated`). A failure stays in the form,
// as it was.
function createNew(d) {
  if (d.busy || newBlocked(d)) return;
  const args = { roles: newRoles(d), title: d.title.trim(), projects: d.projects, labels: d.labels };
  if (d.linked) args.linked_to = d.linked;
  if (d.classification) args.classification = d.classification;
  if (d.body.trim()) args.content = d.body;
  d.busy = true;
  d.error = null;
  renderNewFoot(d);
  rpcCall('create_entity', args).then(({ id }) => d.onCreated(id), e => {
    d.busy = false;
    d.error = e.message;
    renderNewFoot(d);
  });
}

// ----------------------------------------------------------------------------
// rendering

// The form of `d`: its title, its rows, its body, the note of the one made
// before, the error and the buttons.
function renderNewForm(d) {
  const title = accEl('input', 'input new__title');
  title.type = 'text';
  title.placeholder = d.type.hint;
  title.value = d.title;
  title.dataset.focusKey = 'new:title';
  title.addEventListener('input', () => {
    d.title = title.value;
    renderNewFoot(d);
  });
  const parts = [accEl('span', 'label new__title-label', d.type.label), title];

  const [rows, row] = watchFormGrid();
  rows.classList.add('new__rows');
  renderNewTypeRows(d, row);
  d.els.chips = row(t('new.projects_labels'));
  d.els.linked = d.linked ? row(t('new.linked')) : null;
  parts.push(rows, ...renderNewBody(d));

  if (d.created) {
    parts.push(createdNote(t('new.created', { title: d.created.title }), d.created.id, t('new.show_task')));
  }
  d.els.error = accEl('div', 'error-note new__error');
  d.els.foot = accEl('div', 'new__foot');
  parts.push(d.els.error, d.els.foot);
  return parts;
}

// The task's rows: its status and when it is due.
function renderNewTypeRows(d, row) {
  newSegRow(row(t('props.status')), TASK_STATUSES, () => d.status, value => { d.status = value; });
  const note = accEl('span', 'meta');
  const due = newDateInput(d.due, value => {
    d.due = value;
    showDue();
  });
  const clear = newClearButton(t('new.no_due'), () => {
    d.due = '';
    due.value = '';
    showDue();
  });
  const showDue = () => {
    note.textContent = d.due ? relativeDay(parseDate(d.due)) : t('new.none');
    clear.hidden = !d.due;
  };
  row(t('props.due'), due, note, clear);
  showDue();
}

// The list's items, a record's note, or the markdown body of the others.
function renderNewBody(d) {
  const head = accEl('div', 'section-head new__body-head', t('new.content'));
  head.append(accEl('span', 'section-head__note', 'Markdown'));
  const body = accEl('textarea', 'input new__body');
  body.placeholder = 'Markdown';
  body.value = d.body;
  body.addEventListener('input', () => { d.body = body.value; });
  return [head, body];
}

// The projects and labels on one row, as on a card, and the link.
function renderNewChips(d) {
  draftChips(d, d.els.chips, () => renderNewChips(d));
  if (d.scope && d.projects.length === 1 && d.projects[0] === d.scope) {
    d.els.chips.append(accEl('span', 'meta push', t('new.from_scope')));
  }

  if (d.els.linked) {
    const unlink = () => {
      d.linked = null;
      renderNewChips(d);
    };
    d.els.linked.replaceChildren(d.linked && linkTarget(d.linked)
      ? linkChip(d.linked, { onRemove: unlink })
      : accEl('span', 'meta', t('new.none')));
  }
}

// Create, and the error of a failed Create.
function renderNewFoot(d) {
  const blocked = newBlocked(d);
  const create = accEl('button', 'btn btn--primary', d.busy ? t('new.creating') : d.type.create);
  create.type = 'button';
  create.disabled = !!blocked || d.busy;
  create.title = blocked || d.type.createTitle;
  create.addEventListener('click', () => createNew(d));
  d.els.foot.replaceChildren(create, accEl('span', 'meta', shortcutText('Ctrl+Enter')));
  d.els.error.textContent = d.error ?? '';
  d.els.error.hidden = !d.error;
}

// ----------------------------------------------------------------------------
// fields

// An input of the draft `d`'s `key`, of `type`; `onInput` after each change.
// The Task tab's, and New recurring payment's (features/recurring-payment-new.js).
export function newTextInput(d, key, placeholder, { type = 'text', className = 'form-input', onInput = null } = {}) {
  const input = accEl('input', className);
  input.type = type;
  input.placeholder = placeholder;
  input.value = d[key];
  input.addEventListener('input', () => {
    d[key] = input.value;
    onInput?.();
  });
  return input;
}

// The projects and labels of the draft `d` into `el`: a chip each, with x,
// and "+" (`addChip`); `rerender` after a change.
export function draftChips(d, el, rerender) {
  const leave = project_id => {
    d.projects = d.projects.filter(p => p !== project_id);
    rerender();
  };
  const setLabels = labels => {
    d.labels = labels;
    rerender();
  };
  const add = addChip({ projects: d.projects, labels: d.labels }, {
    onJoin: project_id => {
      d.projects = [...d.projects, project_id];
      rerender();
    },
    onAddLabel: label => setLabels([...d.labels, label]),
  });
  el.replaceChildren(...projectChips(d.projects, leave), ...labelChips(d.labels, setLabels), add);
}

export function newDateInput(value, onChange) {
  const input = accEl('input', 'form-input new__date');
  input.type = 'date';
  input.value = value;
  input.addEventListener('change', () => onChange(input.value));
  return input;
}

// A segmented control of `options` in `field`, built again on a pick.
function newSegRow(field, options, current, onPick) {
  const render = () => field.replaceChildren(accSeg(options, current(), value => {
    onPick(value);
    render();
  }));
  render();
}

function newClearButton(title, onClear) {
  const button = accEl('button', 'tool push');
  button.type = 'button';
  button.append(iconEl('#i-dismiss'));
  button.title = title;
  button.addEventListener('click', onClear);
  return button;
}
