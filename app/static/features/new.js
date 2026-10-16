// New: a menu of the types the app makes -- at the sidebar's New item, or
// Ctrl+N -- then the New page of the type picked, rendered into the expanded vault area while
// its op is "new"; and the Task tab of the panel's context column, a new
// task for what the panel shows (a card's Task button opens it). Create
// sends `create_entity` (api.md#structure): the host writes the
// file into its type's folder and pushes the entity. The fields follow the
// vault's roles (vault.md#well-known-roles).
// A draft's form is built once, so its inputs keep focus; only its chips,
// the list's items, the notes and the buttons are built again.

import { $, accEl } from '../core/dom.js';
import { rpcCall, rpcOn } from '../core/rpc.js';
import { isoInstant, localDay, parseDate, relativeDay } from '../core/format.js';
import { registerAction, shortcutText } from '../core/actions.js';
import { t } from '../core/text.js';
import { store, vaultOpen } from '../model/state.js';
import { projectScope, projectsFor } from '../model/entities.js';
import { getDocIcon, iconEl } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import {
  closeVaultArea, openVaultOp, registerVaultPage, replaceVaultPage,
  setPageFocus,
} from '../ui/page.js';
import { accSeg } from '../ui/segmented.js';
import { selectDocument } from './selection.js';
import {
  contextTarget, createdNote, registerContextTab,
  shownContextTab,
} from './context.js';
import { TASK_STATUSES } from './panel/details.js';
import {
  addChip, linkChip, linkTarget, projectChips, labelChips,
} from './panel/panel.js';
import { watchFormGrid } from './workspace/folders.js';

// The types New makes: their derived kind, what Go to calls it (`command`,
// `also`), the folder the host writes them into (shown only; av-vault's
// `schema::layout` decides), and how their title is asked for.
const NEW_TYPES = [
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
const NEW_CONTACT_TYPES = [['person', t('new.contact.person')], ['org', t('new.contact.org')]];

function newTypeIcon(type) {
  return getDocIcon(type.kind, type.kind === 'document' ? 'note' : null);
}

// A new draft of `kind` (one of NEW_TYPES). `page`: it is the New page's,
// which shows the folder and can be cancelled (else the Task tab's);
// `linked`: the entity a task is made for; `projects`: the projects it
// starts in; `scope`: the project selected in the sidebar, noted as such
// while it is the only one; `onCreated(id)`: what its host does once it is
// created.
function newDraft(kind, { page = false, linked = null, projects = [], scope = null, onCreated }) {
  const today = localDay(new Date());
  return {
    type: NEW_TYPES.find(t => t.kind === kind),
    page,
    title: '',
    day: today, // a document's date
    status: 'todo', // a task's
    due: '',
    start: today, // an event's
    startTime: '09:00',
    end: today,
    endTime: '10:00',
    location: '',
    contactType: 'person', // a contact's
    email: '',
    phone: '',
    address: '',
    items: [''], // a list's
    projects: [...projects],
    scope,
    labels: [],
    linked, // the entity it is `linked-to` (a task's, in the Task tab)
    body: '',
    created: null, // { id, title } of the one made before, in the Task tab
    error: null, // why Create failed
    busy: false, // Create is under way
    onCreated,
    els: {}, // the parts built again: chips, linked, items, error, foot
  };
}

// ----------------------------------------------------------------------------
// the menu and the New page

let newPage = null; // the New page's draft

registerAction({
  id: 'new', group: 'new', title: t('new.menu'), icon: '#i-add', shortcut: 'Ctrl+N',
  when: vaultOpen,
  run: anchor => showNewMenu(anchor ?? $('new-item')),
});
for (const type of NEW_TYPES) {
  registerAction({
    id: `new.a_${type.kind}`, group: 'new', title: type.command, icon: newTypeIcon(type), also: type.also,
    when: vaultOpen,
    run: () => openNew(type.kind),
  });
}

// The types, at `anchor`.
function showNewMenu(anchor) {
  const items = NEW_TYPES.map(type => ({
    label: type.name,
    icon: newTypeIcon(type),
    hint: type.folder.split('\\')[0],
    run: () => openNew(type.kind),
  }));
  showMenu(anchor, items, { head: t('new.head') });
}

// Open the New page for `kind`, in the project selected in the sidebar.
// Once created, the page closes and the new entity is selected.
function openNew(kind) {
  openVaultOp('new');
  const scope = projectScope();
  const d = newDraft(kind, {
    page: true,
    projects: scope ? [scope] : [],
    scope,
    onCreated: id => {
      if (newPage === d) closeVaultArea();
      selectDocument(id);
    },
  });
  newPage = d;

  const sheet = accEl('section', 'new__sheet');
  const kindEl = accEl('h2', 'pane-title new__kind');
  kindEl.append(iconEl(newTypeIcon(d.type)), d.type.name);
  sheet.append(kindEl, ...renderNewForm(d));
  const page = accEl('div', 'page');
  page.append(sheet);
  setPageFocus('new:title');
  replaceVaultPage(page);
  renderNewChips(d);
  renderNewFoot(d);
}

// Leaving the page (another op, or the area closed) drops its draft.
store.subscribe([state => state.vault_op, state => state.vault_expanded], ([vault_op, vault_expanded]) => {
  if (newPage && !(vault_op === 'new' && vault_expanded)) newPage = null;
});

registerVaultPage('new', {
  close: closeVaultArea,
  submit: () => {
    if (newPage) createNew(newPage);
  },
});

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
  if (!d.title.trim()) return d.type.missing;
  if (d.type.kind === 'event' && !d.start) return t('new.pick_start');
  if (d.type.kind === 'document' && !d.day) return t('new.pick_date');
  return '';
}

// The draft's role blocks, as the frontmatter has them (vault.md#well-known-roles):
// what is left empty is left out.
function newRoles(d) {
  switch (d.type.kind) {
    case 'document':
      return { document: { type: 'note' } };
    case 'task': {
      const task = { status: d.status };
      if (d.due) task.due = d.due;
      return { task };
    }
    case 'event': {
      const event = { start: newInstant(d.start, d.startTime) };
      if (d.end) event.end = newInstant(d.end, d.endTime);
      if (d.location.trim()) event.location = d.location.trim();
      return { event };
    }
    case 'contact': {
      const contact = { type: d.contactType };
      for (const key of ['email', 'phone', 'address']) {
        if (d[key].trim()) contact[key] = [d[key].trim()];
      }
      return { contact };
    }
    case 'list':
      return { list: { items: d.items.map(item => item.trim()).filter(Boolean) } };
    default:
      return { record: {} };
  }
}

// Create the draft `d` (Create, Ctrl+Enter); the host pushes the new
// entity, and its host goes on (`onCreated`). A failure stays in the form,
// as it was.
function createNew(d) {
  if (d.busy || newBlocked(d)) return;
  const args = { roles: newRoles(d), title: d.title.trim(), projects: d.projects, labels: d.labels };
  if (d.linked) args.linked_to = d.linked;
  if (d.type.kind === 'document') args.date = newInstant(d.day, newLocalTime(new Date()));
  if (!['list', 'record'].includes(d.type.kind) && d.body.trim()) args.content = d.body;
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
// dates: the form's are local days (YYYY-MM-DD) and times (hh:mm)

function newPad(n) {
  return String(n).padStart(2, '0');
}

function newLocalTime(date) {
  return `${newPad(date.getHours())}:${newPad(date.getMinutes())}`;
}

// `day` at `time` here (midnight without a time), as a UTC instant in ISO
// 8601, to the second.
function newInstant(day, time) {
  return isoInstant(new Date(`${day}T${time || '00:00'}`));
}

// An hour after `time`, at most 23:59.
function newHourLater(time) {
  const [h, m] = time.split(':').map(Number);
  return h >= 23 ? '23:59' : `${newPad(h + 1)}:${newPad(m)}`;
}

// ----------------------------------------------------------------------------
// rendering

// The form of `d`: its title, the folder (on the New page), its rows, its
// body, the note of the one made before, the error and the buttons.
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

  if (d.page) {
    const path = accEl('span', 'mono mono--small truncate-start new__path', `${d.type.folder}\\`);
    path.title = t('new.path_title');
    parts.push(path);
  }

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

// The rows of the type's own fields.
function renderNewTypeRows(d, row) {
  switch (d.type.kind) {
    case 'document': {
      row(t('props.type'), accEl('span', '', 'note'));
      const note = accEl('span', 'meta', relativeDay(parseDate(d.day)));
      const day = newDateInput(d.day, value => {
        d.day = value;
        note.textContent = relativeDay(parseDate(value));
        renderNewFoot(d);
      });
      row(t('docs.date'), day, note);
      break;
    }
    case 'task': {
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
      break;
    }
    case 'event': {
      const note = accEl('span', 'meta', relativeDay(parseDate(d.start)));
      // An end before the start moves with it: the same day, an hour later.
      const keepEnd = () => {
        if (!d.start) return;
        if (!d.end || d.end < d.start) d.end = d.start;
        if (d.end === d.start && d.startTime && d.endTime <= d.startTime) d.endTime = newHourLater(d.startTime);
        endDay.value = d.end;
        endTime.value = d.endTime;
      };
      const startDay = newDateInput(d.start, value => {
        d.start = value;
        note.textContent = relativeDay(parseDate(value));
        keepEnd();
        renderNewFoot(d);
      });
      const startTime = newTimeInput(d.startTime, value => {
        d.startTime = value;
        keepEnd();
      });
      const endDay = newDateInput(d.end, value => { d.end = value; });
      const endTime = newTimeInput(d.endTime, value => { d.endTime = value; });
      row(t('props.starts'), startDay, startTime, note);
      row(t('props.ends'), endDay, endTime);
      row(t('new.location'), newTextInput(d, 'location', t('new.address_or_place')));
      break;
    }
    case 'contact':
      newSegRow(row(t('new.type')), NEW_CONTACT_TYPES, () => d.contactType, value => { d.contactType = value; });
      row(t('new.email_address'), newTextInput(d, 'email', '', 'email'));
      row(t('new.phone'), newTextInput(d, 'phone', '', 'tel'));
      row(t('accounts.address'), newTextInput(d, 'address', t('new.street_town')));
      break;
    default:
      break;
  }
}

// The list's items, a record's note, or the markdown body of the others.
function renderNewBody(d) {
  if (d.type.kind === 'list') {
    const head = accEl('div', 'section-head new__body-head', t('new.items'));
    head.append(accEl('span', 'section-head__note', t('new.enter_adds')));
    d.els.items = accEl('div', 'new__items');
    const add = accEl('button', 'tool new__add-item', t('new.add_item'));
    add.type = 'button';
    add.addEventListener('click', () => newAddItem(d, d.items.length));
    renderNewItems(d);
    return [head, d.els.items, add];
  }
  if (d.type.kind === 'record') {
    return [accEl('div', 'note new__body-head', t('new.record_note'))];
  }
  const head = accEl('div', 'section-head new__body-head', t('new.content'));
  head.append(accEl('span', 'section-head__note', 'Markdown'));
  const body = accEl('textarea', 'input new__body');
  body.placeholder = 'Markdown';
  body.value = d.body;
  body.addEventListener('input', () => { d.body = body.value; });
  return [head, body];
}

// A list's items, one input each: Enter goes to the next (a new one after
// the last), Backspace in an empty one removes it. `focus`: the index of
// the item to focus.
function renderNewItems(d, focus = null) {
  const rows = d.items.map((text, i) => {
    const item = accEl('div', 'new__item');
    const input = accEl('input', 'form-input');
    input.type = 'text';
    input.placeholder = t('new.item');
    input.value = text;
    input.addEventListener('input', () => { d.items[i] = input.value; });
    input.addEventListener('keydown', e => {
      if (e.key === 'Enter' && !e.ctrlKey) {
        e.preventDefault();
        if (i + 1 < d.items.length) renderNewItems(d, i + 1);
        else newAddItem(d, i + 1);
      } else if (e.key === 'Backspace' && !input.value && d.items.length > 1) {
        e.preventDefault();
        d.items.splice(i, 1);
        renderNewItems(d, Math.max(0, i - 1));
      }
    });
    const remove = accEl('button', 'tool new__item-remove');
    remove.type = 'button';
    remove.append(iconEl('#i-dismiss'));
    remove.title = t('new.remove_item');
    remove.addEventListener('click', () => {
      d.items.splice(i, 1);
      if (!d.items.length) d.items.push('');
      renderNewItems(d);
    });
    item.append(input, remove);
    return item;
  });
  d.els.items.replaceChildren(...rows);
  if (focus !== null) rows[focus]?.querySelector('input').focus();
}

function newAddItem(d, at) {
  d.items.splice(at, 0, '');
  renderNewItems(d, at);
}

// The projects and labels on one row, as on a card, and the link.
function renderNewChips(d) {
  const leave = project_id => {
    d.projects = d.projects.filter(p => p !== project_id);
    renderNewChips(d);
  };
  const setLabels = labels => {
    d.labels = labels;
    renderNewChips(d);
  };
  const add = addChip({ projects: d.projects, labels: d.labels }, {
    onJoin: project_id => {
      d.projects = [...d.projects, project_id];
      renderNewChips(d);
    },
    onAddLabel: label => setLabels([...d.labels, label]),
  });
  d.els.chips.replaceChildren(...projectChips(d.projects, leave), ...labelChips(d.labels, setLabels), add);
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

// Create (and on the New page Cancel), and the error of a failed Create.
function renderNewFoot(d) {
  const blocked = newBlocked(d);
  const create = accEl('button', 'btn btn--primary', d.busy ? t('new.creating') : d.type.create);
  create.type = 'button';
  create.disabled = !!blocked || d.busy;
  create.title = blocked || d.type.createTitle;
  create.addEventListener('click', () => createNew(d));
  d.els.foot.replaceChildren(create, accEl('span', 'meta', shortcutText('Ctrl+Enter')));
  if (d.page) {
    const cancel = accEl('button', 'btn btn--ghost push', t('field.cancel'));
    cancel.type = 'button';
    cancel.addEventListener('click', closeVaultArea);
    d.els.foot.append(cancel, accEl('span', 'meta', shortcutText('Escape')));
  }
  d.els.error.textContent = d.error ?? '';
  d.els.error.hidden = !d.error;
}

// ----------------------------------------------------------------------------
// fields

function newTextInput(d, key, placeholder, type = 'text') {
  const input = accEl('input', 'form-input');
  input.type = type;
  input.placeholder = placeholder;
  input.value = d[key];
  input.addEventListener('input', () => { d[key] = input.value; });
  return input;
}

function newDateInput(value, onChange) {
  const input = accEl('input', 'form-input new__date');
  input.type = 'date';
  input.value = value;
  input.addEventListener('change', () => onChange(input.value));
  return input;
}

function newTimeInput(value, onChange) {
  const input = accEl('input', 'form-input new__time');
  input.type = 'time';
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
