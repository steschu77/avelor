// The side panel: `renderPanel` is the one place that decides what it shows --
// a thread (all members of `selected_thread` as a flat, date-ordered card list
// in #panel-thread-view, vault.md#thread), or a standalone entity (one card in
// #doc-content) -- driven only by store state (`selected_entity`,
// `selected_thread`, `threads_rev`).

import { $, accEl, oncePerFrame } from '../../core/dom.js';
import { DATE_FORMAT } from '../../core/format.js';
import { store } from '../../model/state.js';
import { entities, knownThreads, threadMembers, threads } from '../../model/entities.js';
import { bindDocContent } from '../../model/documents.js';
import { structureCall } from '../../model/structure.js';
import { getDocIcon, iconEl } from '../../ui/icons.js';
import { KIND_ICONS } from '../../ui/icons.js';
import { editTextInPlace } from '../../ui/field.js';
import { openLink, panelMode, selectDocument } from '../selection.js';

const msgControllers = new Map(); // doc_id -> controller

let renderedThreadId = null;
let renderedMembers = []; // entities entries, in thread order, as currently rendered
let renderedDocId = null; // the standalone entity shown in #doc-content, or null
let renderedRev = -1; // the `threads_rev` the current cards were built for
let focusedDocId = null; // the card currently marked aria-selected

const threadIconEl = $('thread-icon');
const threadPathEl = $('thread-path');
const threadTitleEl = $('thread-title');
const threadRenameBtn = $('thread-rename-btn');
const threadMessagesEl = $('thread-messages');
const docContentEl = $('doc-content');
const panelDocViewEl = $('panel-doc-view');
const panelThreadViewEl = $('panel-thread-view');

// The thread header's pen renames the thread shown (`renameEntity`).
threadRenameBtn.addEventListener('click', () => {
  if (renderedThreadId) renameEntity(renderedThreadId, threadTitleEl);
});

// ----------------------------------------------------------------------------
// The cards' heads and threads lines follow every entity change -- a title
// changed -- without rebuilding the cards; subscribed below.
const refreshCardChips = oncePerFrame(() => {
  for (const controller of msgControllers.values()) {
    renderCardHead(controller);
    renderCardThreads(controller.doc_id, controller.threadsEl);
  }
});

// A card's icon, title and date as its entity has them now. A title being
// renamed keeps what is typed. The pen is hidden for an entity the host
// can't write.
function renderCardHead(controller) {
  const doc = entities.get(controller.doc_id);
  if (!doc) return;
  if (!controller.titleEl.hasAttribute('contenteditable')) {
    controller.titleEl.textContent = doc.title ?? '';
  }
  controller.dateEl.textContent = doc.date ? DATE_FORMAT.format(doc.date) : '';
  controller.iconEl.firstChild.setAttribute('href', getDocIcon(doc.kind, doc.genre));
  controller.renameBtn.hidden = !doc.writable;
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
// for that thread, with the card focused and scrolled to.
function switchThread(doc_id, thread_id) {
  store.setState({ selected_entity: doc_id, selected_thread: thread_id });
}

// ----------------------------------------------------------------------------
// The one panel renderer (subscribed below). Visibility is resolved
// first, so a panel is never still `hidden` (display:none) when a scroll
// below runs against it -- scrolling inside a hidden ancestor is a silent
// no-op. Cards are only rebuilt when the shown thread/entity or
// `threads_rev` changed; otherwise a new `selected_entity` within the shown
// thread just moves the focus, keeping the scroll position.
function renderPanel(state) {
  const mode = panelMode(state);
  panelThreadViewEl.hidden = mode !== 'thread';
  panelDocViewEl.hidden = mode === 'thread';

  const stale = state.threads_rev !== renderedRev;

  if (mode === 'thread') {
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
// Tears down every rendered card -- its content subscription -- and
// empties the card containers.
function destroyRenderedCards() {
  for (const doc of renderedMembers) destroyMessageController(doc.doc_id);
  if (renderedDocId) destroyMessageController(renderedDocId);
  docContentEl.replaceChildren();
  threadMessagesEl.replaceChildren();
  focusedDocId = null;

  renderedThreadId = null;
  renderedMembers = [];
  renderedDocId = null;
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

  focusMessage(focusDocId);
  scrollIntoView(focusDocId);
}

// ----------------------------------------------------------------------------
function renderMembers(thread_id, members) {
  destroyRenderedCards();
  renderedThreadId = thread_id;
  renderedMembers = members;

  for (const doc of members) {
    const controller = createMessageController(doc, { selectable: true });
    threadMessagesEl.appendChild(controller.card);
  }
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
function scrollIntoView(doc_id) {
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
// One card: header and body. `selectable`: a click selects its entity,
// which only makes sense with other cards on screen (a thread).
function createMessageController(msg, { selectable = false, thread = false } = {}) {
  const doc_id = msg.doc_id;

  const card = accEl('div', 'msg');

  const head = accEl('div', 'pane-section pane-section--ruled doc-head');

  const headTitle = accEl('div', 'doc-head__title');
  const kindIconEl = iconEl('');
  const titleEl = accEl('span', 'doc-title');
  // The pen renames the entity in place (`renameEntity`), shown where
  // `renderCardHead` says so -- like the thread header's.
  const renameBtn = accEl('button', 'icon-btn icon-btn--quiet doc-head__rename');
  renameBtn.type = 'button';
  renameBtn.title = 'Rename';
  renameBtn.append(iconEl('#i-edit'));
  renameBtn.addEventListener('click', () => renameEntity(doc_id, titleEl));
  headTitle.appendChild(kindIconEl);
  headTitle.appendChild(titleEl);
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

  const editorRoot = accEl('div', 'pane-section editor prose');
  card.appendChild(editorRoot);

  // A link in the body never navigates the app away: it opens as a link
  // does (`openLink`).
  editorRoot.addEventListener('click', ev => {
    const a = ev.target.closest('a');
    if (!a) return;
    ev.preventDefault();
    openLink(a.getAttribute('href'));
  });

  const unsubscribeContent = bindDocContent(doc_id, editorRoot);

  const controller = {
    doc_id, card, renameBtn, editorRoot, iconEl: kindIconEl, titleEl, dateEl, threadsEl,
    unsubscribeContent,
  };
  msgControllers.set(doc_id, controller);
  renderCardHead(controller);

  if (selectable) {
    // Clicking a card selects that entity, except when the click landed on
    // one of its own buttons (those already have their own behavior).
    // Clicking inside the body itself still selects the card; it's not
    // exempted.
    card.addEventListener('click', ev => {
      if (ev.target.closest('button')) return;
      selectDocument(doc_id);
    });
  }

  return controller;
}

// ----------------------------------------------------------------------------
function destroyMessageController(doc_id) {
  const controller = msgControllers.get(doc_id);
  if (!controller) return;

  controller.unsubscribeContent();
  controller.card.remove();
  msgControllers.delete(doc_id);
}

// ----------------------------------------------------------------------------
// The side panel (thread, or standalone entity) is rendered in exactly one
// place, `renderPanel` (above), from a single subscription over everything
// it depends on.
store.subscribe([
  state => state.selected_entity, state => state.selected_thread, state => state.threads_rev,
], (_values, state) => renderPanel(state));

// The cards' heads and threads lines show the entities and threads as they
// are now.
store.subscribe(state => state.entities_rev, refreshCardChips);
