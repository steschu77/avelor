// The side panel: `renderPanel` is the one place that decides what it shows --
// the day view (the events of `selected_day` as cards in #day-cards, then its
// tasks due as rows that open into cards, kept by id while entities change), a
// thread (all members of `selected_thread` as a flat, date-ordered card list
// in #panel-thread-view, vault.md#thread), or a standalone entity (one card in
// #doc-content) -- driven only by store state (`selected_lens`,
// `selected_day`, `selected_entity`, `selected_thread`, `threads_rev`).
// Structural edits (drops, the remove-from-thread and delete buttons) are host
// round trips (`structureCall`, model/structure.js) that end in a published
// selection, never in a direct render call; the requests (`joinThreadLocal`,
// ...) are model/structure.js's. What a card shows of an event, a task or a
// mail is features/panel/details.js's (`renderCardDetails`, `taskTag`,
// `parseContact`). app.js reaches into `msgControllers` for the cards' dirty
// dots, and features/header.js leaves edit mode; autosave lives in
// model/documents.js and covers every editor in edit mode.

import { $, accEl, oncePerFrame } from '../../core/dom.js';
import { placeChildren } from '../../core/dom.js';
import { DATE_FORMAT } from '../../core/format.js';
import { parseDate } from '../../core/format.js';
import { DAY_LONG_MONTH_YEAR, WEEKDAY_LONG, dayStart, relativeDay } from '../../core/format.js';
import { plural } from '../../core/format.js';
import { createEditor } from '../../editor/editor.js';
import { store } from '../../model/state.js';
import { entities, knownThreads, threadMembers, threads } from '../../model/entities.js';
import { projects, projectsInOrder, threadProjects } from '../../model/entities.js';
import { getMeta } from '../../model/entities.js';
import { assistantChats, isAiConversation } from '../../model/entities.js';
import { dayOccurrences, dayTasks } from '../../model/calendar.js';
import { bindDocContent } from '../../model/documents.js';
import { editorRoots, isDirty, onEdited, trySave } from '../../model/documents.js';
import { reportEditing } from '../../model/documents.js';
import { structureCall } from '../../model/structure.js';
import {
  joinProjectLocal, leaveProjectLocal, setLinkLocal, setTagsLocal,
} from '../../model/structure.js';
import { createThreadLocal, joinThreadLocal, leaveThreadLocal } from '../../model/structure.js';
import { selectionWithout } from '../../model/structure.js';
import { getDocIcon, iconEl } from '../../ui/icons.js';
import { KIND_ICONS } from '../../ui/icons.js';
import { showMenu } from '../../ui/menu.js';
import { armedButton } from '../../ui/confirm.js';
import {
  createDocPreviewToolbar, createEditorToolbar,
} from '../../ui/toolbar.js';
import { editTextInPlace } from '../../ui/field.js';
import {
  MEMBER_DRAG, attachDropZone, endDragPreview, startDragPreview,
} from '../../ui/dropzone.js';
import { openLink, panelMode, selectDocument } from '../selection.js';
import { selectById } from '../selection.js';
import { documentsIfCalendar, setDayListed } from '../selection.js';
import { openLinkedTab } from '../context.js';
import { openTaskTab } from '../context.js';
import { toggleCardChat } from '../context.js';
import { parseContact, taskTag } from './details.js';
import { renderCardDetails } from './details.js';

export const msgControllers = new Map(); // doc_id -> controller
let activeEditId = null; // the one card currently in edit mode, or null

let renderedThreadId = null;
let renderedMembers = []; // entities entries, in thread order, as currently rendered
let renderedDocId = null; // the standalone entity shown in #doc-content, or null
let renderedDay = null; // the day shown in the day view, or null
let renderedDayIds = []; // the entities it lists (cards and rows), in order
const dayRows = new Map(); // doc_id -> the row of a task it lists, not opened
let renderedRev = -1; // the `threads_rev` the current cards were built for
let focusedDocId = null; // the card currently marked aria-selected

const threadIconEl = $('thread-icon');
const threadPathEl = $('thread-path');
const threadTitleEl = $('thread-title');
const threadRenameBtn = $('thread-rename-btn');
const threadProjectEl = $('thread-project');
const threadMessagesEl = $('thread-messages');
const docContentEl = $('doc-content');
const dayWeekdayEl = $('day-weekday');
const dayTitleEl = $('day-title');
const dayCardsEl = $('day-cards');
const panelDocViewEl = $('panel-doc-view');
const panelDayViewEl = $('panel-day-view');
const panelThreadViewEl = $('panel-thread-view');
let dayEmptyEl = null; // the day view's note for a day without anything, built once
let dayDueHeadEl = null; // the heads of its tasks due, and of today's overdue ones
let dayOverdueHeadEl = null;

// ----------------------------------------------------------------------------
// Drag-and-drop (ui/dropzone.js): a row dragged out of the table lens
// (features/lenses/documents.js) can be
// dropped on either card panel:
// - on the thread panel (#thread-messages) it becomes a member of the open
//   thread (`joinThreadLocal`, model/structure.js),
// - on a standalone entity's card (#doc-content) it starts a new thread of
//   the two (`createThreadLocal`).
// Members are ordered by date, so there is no drop position to pick; the
// panel's last card is highlighted as the drop target. Dropping anywhere
// outside these two containers is a no-op via the usual HTML5 DnD rule (no
// preventDefault means no valid drop target). A card dragged out of its
// thread by its head (`MEMBER_DRAG`) is no drop for either: it goes to the
// Documents lens, which takes it out of the thread.
attachDropZone(threadMessagesEl, doc_id => {
  if (renderedThreadId) joinThreadLocal(doc_id, renderedThreadId);
});

attachDropZone(docContentEl, doc_id => {
  if (renderedDocId) createThreadLocal(renderedDocId, doc_id);
});

// The thread header's pen renames the thread shown (`renameEntity`).
threadRenameBtn.addEventListener('click', () => {
  if (renderedThreadId) renameEntity(renderedThreadId, threadTitleEl);
});

// ----------------------------------------------------------------------------
// Deletes `doc_id` (the card's delete button), shown in `thread_id` or
// standalone (null): the host moves its file to `.trash` and dissolves any
// thread it leaves with fewer than two members. A card being edited is
// saved first -- the save reaches the host before the delete, and if the
// delete fails, no edit is lost.
function deleteEntityLocal(doc_id, thread_id) {
  if (!entities.has(doc_id)) return;
  if (activeEditId === doc_id) setMessageEditMode(doc_id, false);

  structureCall('delete_entity', { id: doc_id }, selectionWithout(doc_id, thread_id));
}

// ----------------------------------------------------------------------------
// A card's projects and labels on one line, as `projectChips` and
// `tagChips`, then the "+" that adds either (`addChip`); no row for an
// entity in no project and without labels that can't get any.
function renderCardChips(doc_id, msg) {
  const projectIds = msg.projects ?? [];
  const tags = msg.tags ?? [];
  const leave = msg.writable ? project_id => leaveProjectLocal(doc_id, project_id) : null;
  const chips = [
    ...projectChips(projectIds, leave),
    ...tagChips(tags, msg.writable ? next => setTagsLocal(doc_id, next) : null),
  ];
  if (msg.writable) {
    chips.push(addChip({ projects: projectIds, tags }, {
      onJoin: project_id => joinProjectLocal(doc_id, project_id),
      onAddTag: label => setTagsLocal(doc_id, [...tags, label]),
    }));
  }
  if (!chips.length) return null;
  const row = accEl('div', 'pane-section msg__chips');
  row.append(...chips);
  return row;
}

// The projects `project_ids` as chips, each with an x that calls
// `onRemove(project_id)` -- no x without it (an entity the host can't
// write). Projects not in the vault are left out. `renameProjectChips`
// keeps the names in the panel current. The card's, the thread's and the
// New page's (features/new.js).
export function projectChips(project_ids, onRemove) {
  return project_ids.filter(project_id => projects.has(project_id)).map(project_id => {
    const chip = accEl('span', 'chip');
    chip.dataset.project = project_id;
    const name = accEl('span', '', projects.get(project_id).name);
    chip.append(name);
    if (onRemove) {
      const remove = accEl('button', 'chip__remove', '\u00d7');
      remove.type = 'button';
      remove.title = 'Remove from the project';
      remove.addEventListener('click', () => onRemove(project_id));
      chip.append(remove);
    }
    return chip;
  });
}

// A project was renamed: its chips in the panel take the new name, without
// rebuilding the cards (and their editors).
function renameProjectChips() {
  for (const chip of document.querySelectorAll('#panel .chip[data-project]')) {
    const name = projects.get(chip.dataset.project)?.name;
    if (name !== undefined) chip.firstElementChild.textContent = name;
  }
}

// ----------------------------------------------------------------------------
// Labels (`tags`) as chips, each with an x that calls `onChange(tags)` with
// the label removed -- no x without it; `removeTitle(tag)` titles the x.
// The card's, the New page's and the lens header's (the active label
// filter, features/header.js).
export function tagChips(tags, onChange, removeTitle = tag => `Remove the label ${tag}`) {
  return tags.map(tag => {
    const chip = accEl('span', 'chip chip--tag', tag);
    if (onChange) {
      const remove = accEl('button', 'chip__remove', '\u00d7');
      remove.type = 'button';
      remove.title = removeTitle(tag);
      remove.addEventListener('click', () => onChange(tags.filter(t => t !== tag)));
      chip.append(remove);
    }
    return chip;
  });
}

const STAR_ICON = '#i-star-filled';

// The "+" chip after a row of project and label chips. It opens a menu with
// a filter: the active projects, starred first, and the labels in use --
// but those in `has` ({ projects, tags }) -- and "New label" for a text
// that names none. A pick calls `onJoin(project_id)` or `onAddTag(label)`.
// The card's and the New page's.
export function addChip(has, { onJoin, onAddTag }) {
  const add = accEl('button', 'chip chip--action', '+');
  add.type = 'button';
  add.title = 'Add to a project or a label';
  add.addEventListener('click', () => {
    const projectItems = projectsInOrder()
      .filter(p => !p.archived && !has.projects.includes(p.id))
      .map(p => ({ label: p.name, icon: p.starred ? STAR_ICON : '', run: () => onJoin(p.id) }));
    const used = new Set();
    for (const doc of entities.values()) for (const tag of doc.tags ?? []) used.add(tag);
    const labelItems = [...used].filter(tag => !has.tags.includes(tag)).sort((a, b) => a.localeCompare(b))
      .map(tag => ({ label: tag, run: () => onAddTag(tag) }));
    const known = new Set([...used, ...has.tags].map(tag => tag.toLowerCase()));
    const newLabel = text => known.has(text.toLowerCase())
      ? []
      : [{ label: `New label \u201c${text}\u201d`, run: () => onAddTag(text) }];
    showMenu(add, [{ head: 'Projects' }, ...projectItems, { head: 'Labels' }, ...labelItems],
      { filter: { placeholder: 'Filter, or a new label', extra: newLabel } });
  });
  return add;
}

// ----------------------------------------------------------------------------
// Links (vault.md#metadata): an entity's one
// `linked-to`, and the other side, derived -- the entities and threads
// whose `linked-to` names it, and the transaction items that do. Shown on
// the card (`renderCardLinks`) and in the Linked tab (features/linked.js).

// The entity, thread or project `id` as a link shows it: its title and
// icon; null if it isn't in the vault.
export function linkTarget(id) {
  const doc = entities.get(id);
  if (doc) return { title: doc.title, icon: getDocIcon(doc.kind, doc.genre), date: doc.date };
  const thread = threads.get(id);
  if (thread) return { title: thread.title, icon: KIND_ICONS.thread.icon, date: parseDate(thread.date) };
  const project = projects.get(id);
  if (project) return { title: project.name, icon: KIND_ICONS.project.icon, date: undefined };
  return null;
}

// The links of `id`: `to`, the one it links to (null if none, or not in the
// vault); `from`, the ids of the entities and threads that link to it,
// newest first; `items`, the transaction items that do, as `{ doc_id,
// title }` of their transactions entity and the item.
export function linksOf(id) {
  const holder = entities.get(id) ?? threads.get(id);
  const to = holder?.linked_to && linkTarget(holder.linked_to) ? holder.linked_to : null;
  const from = [];
  const items = [];
  for (const doc of [...entities.values(), ...threads.values()]) {
    if (doc.linked_to === id) from.push(doc.id);
    for (const item of doc.transactions?.transactions ?? []) {
      if (item['linked-to'] === id) items.push({ doc_id: doc.id, title: item.title ?? '' });
    }
  }
  const time = id => entities.get(id)?.date?.getTime() ?? 0;
  from.sort((a, b) => time(b) - time(a));
  return { to, from, items };
}

// A link to `id` as a chip: an arrow for its direction (`incoming`: `id`
// links here), its icon and title -- `label` instead of the title, for a
// transaction item -- which select it, and an x that calls `onRemove` (none
// without it). The card's and the New form's (features/new.js).
export function linkChip(id, { incoming = false, label = null, onRemove = null } = {}) {
  const target = linkTarget(id);
  const title = label ?? (target?.title || 'Untitled');
  const chip = accEl('span', 'chip chip--link');
  chip.title = `${incoming ? 'Linked from' : 'Links to'} \u201c${title}\u201d`;
  const name = accEl('button', 'chip__name', title);
  name.type = 'button';
  name.addEventListener('click', () => selectById(id));
  const icon = iconEl(target?.icon ?? '');
  chip.append(incoming ? '\u2190' : '\u2192', icon, name);
  if (onRemove) {
    const remove = accEl('button', 'chip__remove', '\u00d7');
    remove.type = 'button';
    remove.title = 'Remove the link';
    remove.addEventListener('click', onRemove);
    chip.append(remove);
  }
  return chip;
}

// Fill `row` with the links of the card `doc_id`, as `linkChip`s: the one
// it links to, then those linking to it -- but the chats about it, which
// the Conversation tab lists. An x where the entity holding the link can
// be written; none on a transaction item. The row is hidden while empty.
function renderCardLinks(doc_id, row) {
  const { to, from, items } = linksOf(doc_id);
  const writable = id => (entities.get(id) ?? threads.get(id))?.writable;
  const chips = [];
  if (to) {
    chips.push(linkChip(to, { onRemove: writable(doc_id) ? () => setLinkLocal(doc_id, null) : null }));
  }
  for (const id of from) {
    if (isAiConversation(entities.get(id))) continue;
    chips.push(linkChip(id, { incoming: true, onRemove: writable(id) ? () => setLinkLocal(id, null) : null }));
  }
  for (const item of items) {
    chips.push(linkChip(item.doc_id, { incoming: true, label: item.title || linkTarget(item.doc_id)?.title }));
  }
  row.replaceChildren(...chips);
  row.hidden = !chips.length;
}

// The cards' heads, event details, links and threads lines follow every
// entity change -- a task made for the card, a title changed, an event
// synced, a thread restored -- without rebuilding the cards (and their
// editors); subscribed below.
const refreshCardChips = oncePerFrame(() => {
  renderThreadProjects();
  for (const controller of msgControllers.values()) {
    renderCardHead(controller);
    renderCardDetails(controller.detailsEl, entities.get(controller.doc_id), controller.occurrence);
    renderCardLinks(controller.doc_id, controller.linksEl);
    renderCardThreads(controller.doc_id, controller.threadsEl);
    renderCardChat(controller);
  }
});

// The card's chat button, marked with a dot while chats are linked to its
// entity (`assistantChats`, model/entities.js; the mail analysis writes
// one).
function renderCardChat(controller) {
  const count = assistantChats(controller.doc_id).length;
  controller.chatBtn.toggleAttribute('data-chats', count > 0);
  controller.chatBtn.title = count
    ? `Chat about document \u00b7 ${plural(count, 'chat')}`
    : 'Chat about document';
}

// A card's icon, title and date as its entity has them now (a document's
// icon is its genre's); a day card's date is its occurrence's, and so is
// its title if Google gave it another. A
// title being renamed keeps what is typed. The pen is hidden for an entity
// the host can't write, and for an occurrence's own title, which isn't
// the series' to rename.
function renderCardHead(controller) {
  const doc = entities.get(controller.doc_id);
  if (!doc) return;
  const date = (controller.occurrence ?? doc).date;
  const ownTitle = controller.occurrence?.exception?.title;
  if (!controller.titleEl.hasAttribute('contenteditable')) {
    controller.titleEl.textContent = ownTitle ?? doc.title ?? '';
  }
  controller.dateEl.textContent = date ? DATE_FORMAT.format(date) : '';
  controller.iconEl.firstChild.setAttribute('href', getDocIcon(doc.kind, doc.genre));
  controller.renameBtn.hidden = !doc.writable || ownTitle !== undefined;
}

// Rename `id` -- a card's entity, or the thread shown -- in place in
// `titleEl` (`editTextInPlace`); the host writes the title (an empty one
// removes it) and pushes the entity back. The selection stays.
function renameEntity(id, titleEl) {
  const current = () => (entities.get(id) ?? threads.get(id))?.title ?? '';
  editTextInPlace(titleEl, current,
    title => structureCall('set_title', { id, title }, () => store.getState()));
}

// ----------------------------------------------------------------------------
// Threads (panel.md#thread-switch): an entity in more than one thread of
// the vault has a line of them in its card's head. The one shown is marked;
// a click on another shows that one, with the entity still selected.

// Fill `line` with the threads of the card `doc_id` as chips, in the order
// of its `threads`; hidden unless there are two or more.
function renderCardThreads(doc_id, line) {
  const ids = knownThreads(entities.get(doc_id));
  line.hidden = ids.length < 2;
  if (line.hidden) {
    line.replaceChildren();
    return;
  }
  const label = accEl('span', 'label', 'Threads');
  line.replaceChildren(label, ...ids.map(thread_id => threadChip(doc_id, thread_id)));
}

// The thread `thread_id` of the card `doc_id` as a chip: its icon and
// title, marked if it is the one shown, else a button that shows it.
function threadChip(doc_id, thread_id) {
  const chip = accEl('span', 'chip');
  const icon = iconEl(KIND_ICONS.thread.icon);
  const title = threads.get(thread_id)?.title || 'Untitled';
  if (thread_id === renderedThreadId) {
    chip.setAttribute('aria-current', 'true');
    chip.title = 'The thread shown';
    chip.append(icon, title);
    return chip;
  }
  const name = accEl('button', 'chip__name', title);
  name.type = 'button';
  name.title = 'Show this thread';
  name.addEventListener('click', () => switchThread(doc_id, thread_id));
  chip.append(icon, name);
  return chip;
}

// Show `doc_id` in its thread `thread_id`: `renderPanel` rebuilds the panel
// for that thread, with the card focused and scrolled to -- in the
// Documents lens, if the calendar showed it in its day.
function switchThread(doc_id, thread_id) {
  store.setState({ ...documentsIfCalendar(), selected_entity: doc_id, selected_thread: thread_id });
}

// ----------------------------------------------------------------------------
// The one panel renderer (subscribed below). Visibility is resolved
// first, so a panel is never still `hidden` (display:none) when a scroll
// below runs against it -- scrolling inside a hidden ancestor is a silent
// no-op. Cards are only rebuilt when the shown day/thread/entity or
// `threads_rev` changed; otherwise a new `selected_entity` within the shown
// thread or day just moves the focus, keeping editors and scroll position.
// The day's cards are re-keyed rather than rebuilt (`refreshDay`).
function renderPanel(state) {
  const mode = panelMode(state);
  panelDayViewEl.hidden = mode !== 'day';
  panelThreadViewEl.hidden = mode !== 'thread';
  panelDocViewEl.hidden = mode === 'day' || mode === 'thread';

  const stale = state.threads_rev !== renderedRev;

  if (mode === 'day') {
    const day = state.selected_day ?? dayStart(new Date());
    if (renderedDay?.getTime() !== day.getTime()) {
      renderedRev = state.threads_rev;
      renderDay(day);
    } else if (stale) {
      renderedRev = state.threads_rev;
      refreshDay();
    } else if (dayRows.has(state.selected_entity)) {
      refreshDay(); // a task's row selected: it opens into its card
    } else if (state.selected_entity !== focusedDocId) {
      focusMessage(state.selected_entity);
    }
  } else if (mode === 'thread') {
    if (stale || state.selected_thread !== renderedThreadId) {
      renderedRev = state.threads_rev;
      renderThread(state.selected_thread, state.selected_entity);
    } else if (state.selected_entity !== focusedDocId) {
      focusMessage(state.selected_entity);
    }
  } else if (mode === 'document') {
    if (stale || state.selected_entity !== renderedDocId) {
      renderedRev = state.threads_rev;
      renderDocument(entities.get(state.selected_entity));
    }
  } else if (mode === 'none') {
    if (renderedDocId || renderedThreadId || !docContentEl.querySelector('.empty')) {
      renderedRev = state.threads_rev;
      renderEmpty();
    }
  }
}

// ----------------------------------------------------------------------------
// Tears down every rendered card -- its editor, content subscription, and
// save state registration -- and empties the card containers.
function destroyRenderedCards() {
  for (const doc of renderedMembers) destroyMessageController(doc.doc_id);
  if (renderedDocId) destroyMessageController(renderedDocId);
  for (const doc_id of renderedDayIds) destroyMessageController(doc_id);
  docContentEl.replaceChildren();
  threadMessagesEl.replaceChildren();
  dayCardsEl.replaceChildren();
  focusedDocId = null;

  renderedThreadId = null;
  renderedMembers = [];
  renderedDocId = null;
  renderedDay = null;
  renderedDayIds = [];
  setDayListed([]);
  dayRows.clear();
}

// ----------------------------------------------------------------------------
// The empty preview: nothing selected, e.g. right after the shown document
// was deleted. The panel stays open; no card (and no controller) is left.
function renderEmpty() {
  destroyRenderedCards();
  docContentEl.appendChild(emptyNote(KIND_ICONS.document.icon, 'No document selected', 'Select a document from the list.'));
}

// A note in place of cards (`.empty`): an icon, a title and a hint.
function emptyNote(icon, title, hint) {
  const empty = accEl('div', 'empty');

  const iconBox = accEl('div', 'empty__icon');
  iconBox.append(iconEl(icon));
  empty.appendChild(iconBox);

  const titleEl = accEl('div', 'empty__title', title);
  empty.appendChild(titleEl);

  const hintEl = accEl('div', '', hint);
  empty.appendChild(hintEl);

  return empty;
}

// ----------------------------------------------------------------------------
// Rebuilds the thread panel for `threadId` and focuses (and scrolls to)
// `focusDocId`'s card. Only called by renderPanel.
function renderThread(threadId, focusDocId) {
  const thread = threads.get(threadId);
  if (!thread) return;

  threadIconEl.firstChild.setAttribute('href', KIND_ICONS.thread.icon);
  threadPathEl.textContent = thread.path ?? '';
  threadPathEl.title = thread.path ?? '';
  if (!threadTitleEl.hasAttribute('contenteditable')) threadTitleEl.textContent = thread.title ?? '';
  threadRenameBtn.hidden = !thread.writable;

  renderMembers(threadId, threadMembers(threadId));
  renderThreadProjects();

  focusMessage(focusDocId);
  scrollIntoView(focusDocId);
}

// The thread view's Project row: its members' projects (vault.md#thread),
// without an x -- a member leaves a project on its own card.
function renderThreadProjects() {
  if (renderedThreadId) threadProjectEl.replaceChildren(...projectChips(threadProjects(renderedThreadId), null));
}

// ----------------------------------------------------------------------------
function renderMembers(thread_id, members) {
  destroyRenderedCards();
  renderedThreadId = thread_id;
  renderedMembers = members;

  for (const doc of members) {
    const controller = createMessageController(doc, { selectable: true, thread: true });
    threadMessagesEl.appendChild(controller.card);
  }
}

// ----------------------------------------------------------------------------
// Rebuilds the day view for `day`: its head, then its events' cards and
// its tasks (`refreshDay`), the selected one focused and scrolled to. Only
// called by renderPanel.
function renderDay(day) {
  destroyRenderedCards();
  renderedDay = day;

  dayWeekdayEl.textContent = WEEKDAY_LONG.format(day);
  dayTitleEl.textContent = DAY_LONG_MONTH_YEAR.format(day);

  refreshDay();
  scrollIntoView(focusedDocId);
}

// ----------------------------------------------------------------------------
// The day view follows the entities and the filter: a card per event on
// the day, by start, then the tasks due on it under "Due" and, on today,
// those still open from earlier days under "Overdue" -- all keyed by id. A
// task is a row until it is selected; then it opens into its card, which
// stays while the day is shown. A card that stays keeps its editor (and
// its place in the DOM) and takes its occurrence's head and details; a new
// one is built; one gone is torn down, an edit in it saved first. A day
// without anything shows a note. Called once per frame while the calendar
// lens shows (`renderCalendarLens`), by renderPanel after a structural
// edit, and when a row is selected.
export function refreshDay() {
  if (!renderedDay) return;
  const occurrences = dayOccurrences(renderedDay);
  const { due, overdue } = dayTasks(renderedDay);
  const ids = [...occurrences, ...due, ...overdue].map(entry => entry.doc_id);
  for (const doc_id of renderedDayIds) {
    if (ids.includes(doc_id)) continue;
    destroyMessageController(doc_id);
    dayRows.delete(doc_id);
  }
  renderedDayIds = ids;
  setDayListed(ids);

  const cards = occurrences.map(ev => {
    const controller = msgControllers.get(ev.doc_id);
    if (!controller) {
      return createMessageController(entities.get(ev.doc_id), { selectable: true, occurrence: ev }).card;
    }
    controller.occurrence = ev;
    renderCardHead(controller);
    renderCardDetails(controller.detailsEl, entities.get(ev.doc_id), ev);
    return controller.card;
  });

  const { selected_entity } = store.getState();
  const taskEntry = (doc, inOverdue) => {
    if (msgControllers.has(doc.doc_id)) return msgControllers.get(doc.doc_id).card;
    if (doc.doc_id !== selected_entity) return dueRow(doc, inOverdue);
    dayRows.delete(doc.doc_id);
    return createMessageController(doc, { selectable: true }).card;
  };
  dayDueHeadEl ??= dayHead('Due');
  dayOverdueHeadEl ??= dayHead('Overdue', 'from earlier days');
  const entries = [
    ...cards,
    ...(due.length ? [dayDueHeadEl, ...due.map(doc => taskEntry(doc, false))] : []),
    ...(overdue.length ? [dayOverdueHeadEl, ...overdue.map(doc => taskEntry(doc, true))] : []),
  ];
  dayEmptyEl ??= emptyNote(KIND_ICONS.event.icon, 'No events', 'Nothing in the calendar on this day.');
  placeChildren(dayCardsEl, entries.length ? entries : [dayEmptyEl]);
  focusMessage(selected_entity);
}

// A head over the day's tasks, with an optional note.
function dayHead(title, note) {
  const head = accEl('div', 'section-head', title);
  if (note) head.append(accEl('span', 'section-head__note', note));
  return head;
}

// The row of a task the day lists: its tag, title and, as meta, how long
// ago it was due (under "Overdue") and its assignee. Kept by id, filled
// again on every refresh; a click selects the task, which opens it.
function dueRow(doc, inOverdue) {
  let row = dayRows.get(doc.doc_id);
  if (!row) {
    row = accEl('button', 'tile due-row');
    row.type = 'button';
    const doc_id = doc.doc_id;
    row.addEventListener('click', () => selectDocument(doc_id));
    dayRows.set(doc_id, row);
  }
  const meta = [
    inOverdue ? relativeDay(doc.due) : '',
    doc.task.assignee ? parseContact(doc.task.assignee).name : '',
  ].filter(Boolean).join(' \u00b7 ');
  row.replaceChildren(taskTag(doc), accEl('span', 'row-title', doc.title || 'Untitled'));
  if (meta) row.append(accEl('span', 'meta', meta));
  return row;
}

// Scrolls the day view to today's overdue tasks (the grid's count).
export function scrollToOverdue() {
  if (dayOverdueHeadEl?.isConnected) dayOverdueHeadEl.scrollIntoView({ block: 'start', behavior: 'instant' });
}

// ----------------------------------------------------------------------------
// Marks `doc_id`'s card as the selected one, mirroring the table row's own
// [aria-selected]. Scrolling is a separate, explicit action (see call
// sites) -- not every focus should also move the viewport. One-shot/
// non-persistent: a later full re-render (a different thread, or a new
// member) clears it rather than trying to carry it over.
function focusMessage(doc_id) {
  if (focusedDocId && focusedDocId !== doc_id) {
    msgControllers.get(focusedDocId)?.card.setAttribute('aria-selected', 'false');
  }

  const controller = msgControllers.get(doc_id);
  if (!controller) {
    focusedDocId = null;
    return;
  }

  controller.card.setAttribute('aria-selected', 'true');
  focusedDocId = doc_id;
}

// ----------------------------------------------------------------------------
export function scrollIntoView(doc_id) {
  const card = msgControllers.get(doc_id)?.card;
  card?.scrollIntoView({ block: 'center', behavior: 'instant' });
}

// ----------------------------------------------------------------------------
function renderDocument(doc) {
  destroyRenderedCards();
  renderedDocId = doc.doc_id;

  const controller = createMessageController(doc);
  docContentEl.appendChild(controller.card);
}

// ----------------------------------------------------------------------------
// One card: header, event details, chips, toolbars, and its own editor.
// `thread`: a member of the thread shown -- its head drags it out of the
// thread, and remove-from-thread applies; `selectable`: a click selects its
// entity, which only makes sense with other cards on screen (a thread, a
// day); `occurrence`: a day card's occurrence of its event (`date`, `end`).
function createMessageController(msg, { selectable = false, thread = false, occurrence = null } = {}) {
  const doc_id = msg.doc_id;

  const card = accEl('div', 'msg');

  const head = accEl('div', 'pane-section pane-section--ruled doc-head');

  const headTitle = accEl('div', 'doc-head__title');
  const kindIconEl = iconEl('');
  const titleEl = accEl('span', 'doc-title');
  const dirtyDot = accEl('span', 'dirty-dot');
  dirtyDot.hidden = !isDirty(doc_id);
  // The pen renames the entity in place (`renameEntity`), shown where
  // `renderCardHead` says so -- like the thread header's.
  const renameBtn = accEl('button', 'icon-btn icon-btn--quiet doc-head__rename');
  renameBtn.type = 'button';
  renameBtn.title = 'Rename';
  renameBtn.append(iconEl('#i-edit'));
  renameBtn.addEventListener('click', () => renameEntity(doc_id, titleEl));
  headTitle.appendChild(kindIconEl);
  headTitle.appendChild(titleEl);
  headTitle.appendChild(dirtyDot);
  headTitle.appendChild(renameBtn);

  const headSub = accEl('div', 'doc-head__sub');

  const dateEl = accEl('span', 'label');

  const pathEl = accEl('span', 'meta truncate-start', msg.path ?? '');
  pathEl.title = msg.path ?? '';

  headSub.appendChild(dateEl);
  headSub.appendChild(pathEl);

  head.appendChild(headTitle);
  head.appendChild(headSub);

  const threadsEl = accEl('div', 'doc-head__threads');
  renderCardThreads(doc_id, threadsEl);
  head.appendChild(threadsEl);
  card.appendChild(head);

  // An event's details (features/panel/details.js), refreshed with the
  // chips.
  const detailsEl = accEl('div', 'pane-section props');
  renderCardDetails(detailsEl, msg, occurrence);
  card.appendChild(detailsEl);

  // In a thread, the head drags the card: dropped on the Documents lens it
  // leaves the thread (features/lenses/documents.js); like a row there, on
  // a project pill or a project's or thread's row it joins that.
  if (thread) {
    head.draggable = true;
    head.addEventListener('dragstart', ev => {
      ev.dataTransfer.setData('text/plain', doc_id);
      ev.dataTransfer.setData(MEMBER_DRAG, renderedThreadId);
      startDragPreview(ev, kindIconEl.firstChild.getAttribute('href'), titleEl.textContent, 'copyMove');
    });
    head.addEventListener('dragend', endDragPreview);
  }

  const chipsEl = renderCardChips(doc_id, msg);
  if (chipsEl) card.appendChild(chipsEl);

  const linksEl = accEl('div', 'pane-section msg__chips');
  renderCardLinks(doc_id, linksEl);
  card.appendChild(linksEl);

  const previewToolbar = createDocPreviewToolbar();
  card.appendChild(previewToolbar);

  const editorToolbar = createEditorToolbar();
  card.appendChild(editorToolbar);

  const editorRoot = accEl('div', 'pane-section editor prose');
  card.appendChild(editorRoot);

  const editor = createEditor(editorRoot, {
    toolbar: editorToolbar,
    onEscape: () => setMessageEditMode(doc_id, false),
    onChange: () => onEdited(doc_id),
    onLinkClick: openLink,
  });

  editorRoots.set(doc_id, editorRoot);

  const editable = Boolean(getMeta(doc_id)?.editable);
  const editBtn = previewToolbar.querySelector('button[data-cmd="edit"]');
  editBtn.setAttribute('aria-disabled', String(!editable));
  editBtn.title = editable ? 'Edit message' : 'Read-only';

  const unsubscribeContent = bindDocContent(doc_id, editor, editorRoot);

  editBtn.addEventListener('click', () => {
    if (editBtn.getAttribute('aria-disabled') === 'true') return;
    setMessageEditMode(doc_id, !editor.isEditable());
  });

  // Remove-from-thread: only meaningful on a card shown as a member of the
  // open thread, and always acts on that thread (an entity may be in
  // several).
  const threadBtn = previewToolbar.querySelector('button[data-cmd="thread"]');
  threadBtn.setAttribute('aria-disabled', String(!thread));
  threadBtn.addEventListener('click', () => {
    if (threadBtn.getAttribute('aria-disabled') === 'true') return;
    leaveThreadLocal(doc_id, renderedThreadId);
  });

  // New task for this: the context column's Task tab, for this card
  // (features/new.js); its links: the Linked tab (features/linked.js);
  // the chat beside it, pressed while open (features/assistant/assistant.js).
  previewToolbar.querySelector('button[data-cmd="task"]').addEventListener('click', () => openTaskTab(doc_id));
  previewToolbar.querySelector('button[data-cmd="link"]').addEventListener('click', () => openLinkedTab(doc_id));
  const chatBtn = previewToolbar.querySelector('button[data-cmd="chat"]');
  chatBtn.setAttribute('aria-pressed', String(store.getState().panel_size === 'chat'));
  chatBtn.addEventListener('click', () => toggleCardChat(doc_id));

  // Delete: two clicks, no dialog (`armedButton`).
  const deleteBtn = previewToolbar.querySelector('button[data-cmd="delete"]');
  deleteBtn.setAttribute('aria-disabled', String(!getMeta(doc_id)?.deletable));
  armedButton(deleteBtn, 'Click again to delete',
    () => deleteEntityLocal(doc_id, thread ? renderedThreadId : null));

  const controller = {
    doc_id, card, editor, editBtn, renameBtn, dirtyDot, editorRoot, iconEl: kindIconEl, titleEl,
    dateEl, detailsEl, linksEl, threadsEl, chatBtn, occurrence, unsubscribeContent,
  };
  msgControllers.set(doc_id, controller);
  renderCardHead(controller);
  renderCardChat(controller);

  if (selectable) {
    // Clicking a card selects that entity, except when the click landed on
    // one of its own toolbar buttons (those already have their own
    // behavior). Clicking inside the editor body itself -- read-only text,
    // or placing a cursor while editing -- still selects the card; it's
    // not exempted.
    card.addEventListener('click', ev => {
      if (ev.target.closest('button')) return;
      selectDocument(doc_id);
    });
  }

  return controller;
}

// ----------------------------------------------------------------------------
function setMessageEditMode(doc_id, on) {
  if (on && activeEditId && activeEditId !== doc_id) {
    setMessageEditMode(activeEditId, false);
  }

  const controller = msgControllers.get(doc_id);
  if (!controller) return;

  if (!on && controller.editor.isEditable()) {
    trySave(doc_id);
  }
  controller.editor.setEditable(on);
  controller.editBtn.setAttribute('aria-pressed', String(on));

  if (on) activeEditId = doc_id;
  else if (activeEditId === doc_id) activeEditId = null;
  reportEditing();
}

// ----------------------------------------------------------------------------
function destroyMessageController(doc_id) {
  const controller = msgControllers.get(doc_id);
  if (!controller) return;

  const editing = controller.editor.isEditable();
  if (editing) trySave(doc_id);
  controller.unsubscribeContent();
  controller.editor.destroy();
  controller.card.remove();
  editorRoots.delete(doc_id);
  msgControllers.delete(doc_id);
  if (activeEditId === doc_id) activeEditId = null;
  if (editing) reportEditing();
}

// ----------------------------------------------------------------------------
// The card in edit mode, if any, leaves it, saving as it does (closing the
// app, switching the vault: `leaveEditModes`, features/header.js).
export function leaveCardEditMode() {
  if (activeEditId) setMessageEditMode(activeEditId, false);
}

// ----------------------------------------------------------------------------
// The side panel (day view, thread, or standalone entity) is rendered in
// exactly one place, `renderPanel` (above), from a single
// subscription over everything it depends on.
store.subscribe([
  state => state.selected_lens, state => state.selected_day?.getTime(), state => state.selected_entity,
  state => state.selected_thread, state => state.threads_rev,
], (_values, state) => renderPanel(state));

// The cards' heads, event details, links and threads lines show the
// entities and threads as they are now.
store.subscribe(state => state.entities_rev, refreshCardChips);

// A project was added or renamed: only its chips show its name, so they
// alone are refreshed rather than rebuilding the panel (and its editors).
store.subscribe(state => state.projects_rev, renameProjectChips);
