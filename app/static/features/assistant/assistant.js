// The assistant (assistant.md): conversations with the model picked for them
// (the vault's settings, picked in the head: accounts.md#model-choice),
// in the panel's context column (expanded or half-open), its Conversation tab
// (features/context.js). A conversation is an entity in
// the vault, linked to the document or thread the panel showed when it
// started; the switcher lists the conversations linked to what is shown. The host
// checks the picked model (`aiModelOf` mirrors it), sends the
// model what is typed and the document or thread the conversation is linked to,
// writes the conversation and streams the reply
// (`chat_open`, `chat_send`, `chat_stop`, `chat_select`; the events
// `conversation` and `ai_delta`); the page shows the path from the first
// message to `active`, and lets the user edit a message and send it again,
// which starts a branch. A card's chat button opens the chat beside the
// card (`toggleCardChat`). A written reply can be saved as a document, or
// start a task (`assistantSaveReply`; the `task.with` action,
// features/new.js). A reply that ends the path may offer questions to ask
// next, as chips (the email analysis's conversation, `renderAssistantSuggestions`).

import { $, accEl, oncePerFrame } from '../../core/dom.js';
import { titled } from '../../core/dom.js';
import { DAY_SHORT_MONTH, TIME_FORMAT } from '../../core/format.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import { runAction } from '../../core/actions.js';
import { t, textOf } from '../../core/text.js';
import { createEditor } from '../../editor/editor.js';
import { serializeMarkdown } from '../../editor/markdown.js';
import { store } from '../../model/state.js';
import { showError } from '../../model/state.js';
import {
  assistantConversations, entities, isAiConversation, threads, vaultSettings,
} from '../../model/entities.js';
import { projectsFor } from '../../model/entities.js';
import { iconEl } from '../../ui/icons.js';
import { showMenu } from '../../ui/menu.js';
import { dayShows, openLink, panelMode } from '../selection.js';
import { registerContextTab } from '../context.js';
import { createdNote } from '../context.js';
import { linkTarget } from '../panel/panel.js';
import {
  aiAnyOffered, aiModelOf, aiModelPicker, aiOpenAccounts, aiProvider,
  aiShowModels,
} from '../workspace/models.js';
import { aiUsd } from '../workspace/models.js';
import {
  childrenByParent, firstLeafFrom, pathToActive, siblingsOf,
} from './tree.js';

const assistantMessagesEl = $('assistant-messages');
const assistantModelEl = $('assistant-model');
const assistantSwitchEl = $('assistant-switch');
const assistantTitleEl = $('assistant-title');
const assistantInputEl = $('assistant-input');
const assistantSendEl = $('assistant-send');

const ASSISTANT_EMPTY_HTML = '<p><br></p>';

registerContextTab({
  id: 'conversation', title: t('kind.conversation'), el: $('assistant'),
  show: focus => { if (focus) assistantInput.focus({ preventScroll: true }); },
});

let assistantItem = null; // the id of the document or thread shown, or null
let assistantOpen = null; // the id of the conversation shown, or null for a new one
let assistantConversation = null; // what the host sent for `assistantOpen`
let assistantSend = null; // { draft } of a chat_send not answered yet
let assistantEditing = null; // { id, markdown } of the user message being edited
let assistantNotice = null; // what went wrong with the last send, stop or switch
// the item's id ('' for none) -> the conversation last shown for it
const assistantLastOpen = new Map();
// message id -> { el, label, body, role, editor?, actions, html, ... }
const assistantNodes = new Map();
// a reply's message id -> the document it was saved as, { id, title }, or
// { busy: true } while it is saved; this session's
const assistantSaved = new Map();

function assistantItemTitle(item) {
  if (!item) return '';
  return threads.get(item)?.title ?? entities.get(item)?.title ?? '';
}

// ----------------------------------------------------------------------------
// what is shown

// The item the panel shows: the thread in the thread view, the entity in
// the document view, the focused card's in the leg and the day view. An AI
// conversation shown in the panel opens in the assistant, with the item it is
// linked to.
function assistantShow(state) {
  let item = null;
  const mode = panelMode(state);
  if (mode === 'thread') item = state.selected_thread;
  else if (mode === 'document' || mode === 'leg') item = state.selected_entity;
  else if (mode === 'day' && dayShows(state.selected_entity)) item = state.selected_entity;

  let open;
  const doc = item ? entities.get(item) : null;
  if (isAiConversation(doc)) {
    open = item;
    item = doc.linked_to ?? null;
  }
  if (item === assistantItem && (open === undefined || open === assistantOpen)) return;
  assistantItem = item;
  if (open === undefined) {
    const conversations = assistantConversations(item);
    const last = assistantLastOpen.get(item ?? '');
    open = conversations.some(conversation => conversation.doc_id === last) ? last : conversations[0]?.doc_id ?? null;
  }
  assistantOpenChat(open);
}

// Show the conversation `id`, or a new one (null).
function assistantOpenChat(id) {
  assistantEndEdit();
  assistantOpen = id;
  assistantConversation = null;
  assistantSend = null;
  assistantNotice = null;
  if (id) {
    assistantLastOpen.set(assistantItem ?? '', id);
    rpcCall('chat_open', { id }, { channel: 'chat-open' }).then(
      ({ conversation }) => onConversation(id, conversation),
      e => {
        if (id !== assistantOpen) return;
        if (e.code === 'not_found') onConversation(id, null);
        else assistantFailed(e);
      });
  }
  renderAssistant();
}

// The messages shown: from the first one to `active`, then on down to the
// newest leaf (a reply being written, or one that failed, isn't `active`).
function assistantPath(conversation) {
  const byId = new Map(conversation.messages.map(m => [m.id, m]));
  const children = childrenByParent(conversation.messages);
  const roots = children.get(undefined) ?? [];
  const start = byId.get(conversation.active) ?? roots.reduce((a, b) => (b.id > a.id ? b : a), roots[0]);
  if (!start) return [];
  return pathToActive(byId, firstLeafFrom(start, children).id);
}

function assistantWriting() {
  return !!assistantOpen && !!assistantConversation?.messages.some(m => m.status === 'writing');
}

// ----------------------------------------------------------------------------
// what the host sends

// The conversation `id` as it is now (null: gone): an answer, or the
// `conversation` event. Ignored unless it is the one shown.
function onConversation(id, conversation) {
  if (id !== assistantOpen) return;
  const first = !assistantConversation;
  assistantConversation = conversation;
  if (!conversation) assistantOpen = null; // gone
  renderAssistant(first);
}

function onAiDelta(conversation_id, message_id, html) {
  if (conversation_id !== assistantOpen || !assistantConversation) return;
  const message = assistantConversation.messages.find(m => m.id === message_id);
  if (message) message.html = html;
  const node = assistantNodes.get(message_id);
  if (!node) return;
  const stick = assistantAtBottom();
  node.body.innerHTML = html;
  node.html = html;
  if (stick) assistantScrollDown();
}

// A call about the conversation shown failed: say why (`superseded` and `cancelled`
// need no word).
function assistantFailed(e) {
  if (e.code === 'superseded' || e.code === 'cancelled') return;
  assistantNotice = e.message;
  renderAssistant();
}

// ----------------------------------------------------------------------------
// actions

// The model conversations are answered with, or what keeps it from answering
// (`aiModelOf`).
function assistantSetup() {
  return aiModelOf(vaultSettings().chat_model);
}

// Pick the model for conversations; it is kept in the vault's settings, and the
// `vault_settings` event shows it.
function assistantPickModel(choice) {
  rpcCall('set_vault_settings', { chat_model: choice }).catch(assistantFailed);
}

// Send `payload` ({ text, parent?, linked_to? } or { retry }) with the chat
// model; `draft` is the input's content, restored if the host refuses. The
// answer shows the reply being written, in a new conversation if there was
// none; an answer after the user moved on is dropped.
function assistantPost(payload, draft) {
  const setup = assistantSetup();
  if (setup.problem) return;
  const send = { draft };
  assistantSend = send;
  assistantNotice = null;
  rpcCall('chat_send', { conversation: assistantOpen, ...payload }).then(({ id, conversation }) => {
    if (assistantSend !== send) return;
    assistantSend = null;
    if (id !== assistantOpen) {
      assistantOpen = id;
      assistantLastOpen.set(assistantItem ?? '', id);
    }
    onConversation(id, conversation);
  }, e => {
    if (assistantSend !== send) return;
    assistantSend = null;
    // A message from the input goes back there.
    if (send.draft && !assistantDraftText()) assistantInput.setHTML(send.draft);
    assistantFailed(e);
  });
  renderAssistant();
}

// The input's message: after the shown path's last written message, or as a
// new conversation linked to the shown item.
function assistantSendInput() {
  const text = assistantDraftText();
  if (!text || assistantSend || assistantWriting() || assistantSetup().problem) return;
  if (assistantOpen && !assistantConversation) return; // still loading
  let payload;
  if (assistantConversation) {
    const written = assistantPath(assistantConversation).filter(m => m.status !== 'failed' && m.status !== 'writing');
    payload = { text, parent: written[written.length - 1]?.id ?? null };
  } else {
    payload = { text, linked_to: assistantItem };
  }
  const draft = assistantInput.getHTML();
  assistantInput.setHTML(ASSISTANT_EMPTY_HTML);
  assistantPost(payload, draft);
  assistantScrollDown();
}

function assistantStop() {
  if (assistantOpen) rpcCall('chat_stop', { conversation: assistantOpen }).catch(assistantFailed);
}

function assistantStartEdit(id) {
  if (assistantEditing?.id === id) return;
  assistantEndEdit();
  const node = assistantNodes.get(id);
  if (!node?.editor) return;
  assistantEditing = { id, markdown: serializeMarkdown(node.body).trim() };
  node.el.classList.add('chat__msg--editing');
  node.actions.hidden = false;
  node.editor.setEditable(true);
  updateAssistantButtons();
}

// Leave editing; the message shows its sent text again.
function assistantEndEdit() {
  if (!assistantEditing) return;
  const node = assistantNodes.get(assistantEditing.id);
  assistantEditing = null;
  if (!node) return;
  node.editor.setEditable(false);
  node.editor.setHTML(node.html);
  node.el.classList.remove('chat__msg--editing');
  node.actions.hidden = true;
}

// Send the edited message: unchanged, it is answered again; changed, it
// becomes a new message beside it (a branch).
function assistantResend() {
  const editing = assistantEditing;
  const node = editing && assistantNodes.get(editing.id);
  const message = editing && assistantConversation?.messages.find(m => m.id === editing.id);
  if (!node || !message || assistantSend || assistantWriting() || assistantSetup().problem) return;
  const text = serializeMarkdown(node.body).trim();
  if (!text) return;
  const payload = text === editing.markdown ? { retry: message.id } : { text, parent: message.parent ?? null };
  assistantEndEdit();
  assistantPost(payload, null);
}

function assistantSelect(message) {
  if (!assistantOpen) return;
  rpcCall('chat_select', { conversation: assistantOpen, message }).catch(assistantFailed);
}

// The switcher: the conversations linked to the shown item, and a new one.
function assistantSwitch() {
  const conversations = assistantConversations(assistantItem);
  const items = conversations.map(conversation => ({
    label: conversation.title || t('untitled'),
    hint: conversation.date ? DAY_SHORT_MONTH.format(conversation.date) : '',
    checked: conversation.doc_id === assistantOpen,
    run: () => assistantOpenChat(conversation.doc_id),
  }));
  items.push({
    label: t('chat.new_conversation'),
    checked: !assistantOpen,
    separator: conversations.length > 0,
    run: () => {
      assistantOpenChat(null);
      assistantInput.focus();
    },
  });
  showMenu(assistantSwitchEl, items, { matchWidth: false });
}

// Save as document: the reply of `node` becomes a document -- its markdown
// the body, titled by its first line, linked to what the conversation is about and
// in its projects (`projectsFor`). The conversation stays; the reply notes the
// document.
function assistantSaveReply(node) {
  const content = node.content.trim();
  const saved = assistantSaved.get(node.id);
  if (!content || saved?.busy || (saved && entities.has(saved.id))) return;
  const item = assistantConversation?.linked_to ?? null;
  const linked = item && linkTarget(item) ? item : null;
  const title = assistantTitleOf(content) || assistantConversation?.title || t('chat.assistant_reply');
  const args = { roles: { document: { genre: 'note' } }, title, content, projects: projectsFor(linked).projects };
  if (linked) args.linked_to = linked;
  if (node.personal) args.classification = 'pii';
  assistantSaved.set(node.id, { busy: true });
  renderAssistantReplyActions(node);
  rpcCall('create_entity', args).then(({ id }) => {
    assistantSaved.set(node.id, { id, title });
    renderAssistantReplyActions(node);
  }, e => {
    assistantSaved.delete(node.id);
    renderAssistantReplyActions(node);
    const message = t('chat.save_reply_failed', { error: e.message });
    if (assistantNodes.get(node.id) !== node) return showError(message);
    assistantNotice = message;
    renderAssistant();
  });
}

// The title of a document made of `markdown`: its first line that gives
// one, by the host's rule for a conversation's title (`markdown::plain_title`) --
// without its block marker and inline marks, cut at a word to at most 80
// characters and an ellipsis (as long as a scanned document's). '' if no
// line gives one.
function assistantTitleOf(markdown, max = 80) {
  for (const raw of markdown.split('\n')) {
    const line = raw.trim().replace(/^[#>*+\- ]+/, '').replace(/^\d+\. /, '').replace(/[*_`]/g, '').trim();
    if (!line) continue;
    if (line.length <= max) return line;
    let cut = line.slice(0, max);
    const space = cut.lastIndexOf(' ');
    if (space > max / 2) cut = cut.slice(0, space);
    return `${cut.replace(/[ ,.;:]+$/, '')}\u2026`;
  }
  return '';
}

// ----------------------------------------------------------------------------
// rendering

function assistantAtBottom() {
  const el = assistantMessagesEl;
  return el.scrollHeight - el.scrollTop - el.clientHeight < 24;
}

function assistantScrollDown() {
  assistantMessagesEl.scrollTop = assistantMessagesEl.scrollHeight;
}

function assistantTime(iso, withDay) {
  if (!iso) return '';
  const date = new Date(iso);
  const time = TIME_FORMAT.format(date);
  if (!withDay) return time;
  if (date.toDateString() === new Date().toDateString()) return t('chat.today_at', { time });
  return `${DAY_SHORT_MONTH.format(date)} ${time}`;
}

function assistantDraftText() {
  return serializeMarkdown(assistantInputEl).trim();
}

// `scrollDown`: the conversation was just opened.
function renderAssistant(scrollDown) {
  const setup = assistantSetup();
  renderAssistantHead();
  const stick = scrollDown || assistantAtBottom();
  renderAssistantMessages(setup);
  updateAssistantButtons(setup);
  if (stick) assistantScrollDown();
}

function renderAssistantHead() {
  aiModelPicker(vaultSettings().chat_model, assistantPickModel, assistantModelEl);

  const conversations = assistantConversations(assistantItem);
  const index = conversations.findIndex(conversation => conversation.doc_id === assistantOpen);
  assistantSwitchEl.textContent = index >= 0
    ? `${t('count.of_total', { at: index + 1, n: conversations.length })} \u2304`
    : conversations.length ? `${t('chat.new_more', { n: conversations.length })} \u2304` : `${t('new.head')} \u2304`;

  const title = assistantConversation?.title ?? (assistantOpen ? entities.get(assistantOpen)?.title : '') ?? '';
  assistantTitleEl.textContent = title;
  assistantTitleEl.hidden = !title;
}

const scheduleRenderAssistantHead = oncePerFrame(() => renderAssistantHead());

function renderAssistantMessages(setup) {
  for (const el of assistantMessagesEl.querySelectorAll('.chat__hint, .chat__suggestions, :scope>.error-note')) el.remove();
  const conversation = assistantOpen ? assistantConversation : null;
  const path = conversation ? assistantPath(conversation) : [];

  const shown = new Set(path.map(m => m.id));
  if (assistantEditing && !shown.has(assistantEditing.id)) assistantEndEdit();
  for (const [id, node] of assistantNodes) {
    if (shown.has(id)) continue;
    node.editor?.destroy();
    node.el.remove();
    assistantNodes.delete(id);
  }

  const children = conversation ? childrenByParent(conversation.messages) : new Map();
  path.forEach((message, i) => {
    let node = assistantNodes.get(message.id);
    if (!node) {
      node = message.role === 'user' ? createAssistantUserNode(message) : createAssistantReplyNode(message);
      assistantNodes.set(message.id, node);
    }
    updateAssistantNode(node, message, children);
    // Only a node out of place moves: moving one would blur its editor.
    if (assistantMessagesEl.children[i] !== node.el) {
      assistantMessagesEl.insertBefore(node.el, assistantMessagesEl.children[i] ?? null);
    }
  });

  const last = path[path.length - 1];
  if (last?.role === 'assistant' && last.status === 'done' && last.suggestions?.length) {
    assistantMessagesEl.append(renderAssistantSuggestions(last, setup));
  }

  if (!assistantOpen) {
    if (!setup.problem) {
      const hint = accEl('div', 'chat__hint');
      const item = assistantItemTitle(assistantItem);
      hint.append(accEl('div', '', item
        ? t('chat.sent_with_item', { item, model: setup.model })
        : t('chat.sent_only', { model: setup.model })));
      hint.append(accEl('div', '', item
        ? t('chat.saved_linked', { item })
        : t('chat.saved')));
      assistantMessagesEl.append(hint);
    }
  }
  if (setup.problem) assistantMessagesEl.append(renderAssistantGuide(setup));
  if (assistantNotice) {
    assistantMessagesEl.append(accEl('div', 'error-note', assistantNotice));
  }
}

// The questions the last reply offers to ask next (the email analysis's
// conversation), as chips: a click sends one as if it were typed. They are gone
// once the conversation moves on.
function renderAssistantSuggestions(message, setup) {
  const row = accEl('div', 'chat__suggestions');
  const busy = !!assistantSend || assistantWriting() || !!setup.problem;
  for (const text of message.suggestions) {
    const chip = accEl('button', 'chip chip--action', text);
    chip.type = 'button';
    chip.title = t('chat.ask_this');
    chip.disabled = busy;
    chip.addEventListener('click', () => {
      if (assistantSend || assistantWriting() || assistantSetup().problem) return;
      assistantPost({ text, parent: message.id }, null);
      assistantScrollDown();
    });
    row.append(chip);
  }
  return row;
}

// What keeps the picked model from answering, with the way out: the menu
// of models, or Accounts.
function renderAssistantGuide(setup) {
  const guide = accEl('div', 'chat__hint chat__guide');
  const name = setup.account ? aiProvider(setup.account.kind).name : '';
  const pickable = aiAnyOffered();
  let text;
  let label = t('models.pick');
  let pick = pickable;
  if (setup.problem === 'provider') {
    text = t('chat.guide.provider');
    label = t('chat.guide.set_up');
    pick = false;
  } else if (setup.problem === 'key') {
    text = t('chat.guide.key', { name, model: setup.model });
    label = t('open.named', { name });
    pick = false;
  } else if (setup.problem === 'limit') {
    text = t('chat.guide.limit', { name, limit: aiUsd(setup.account.spend_limit) });
    label = t('chat.guide.raise');
    pick = false;
  } else if (setup.problem === 'gone') {
    text = t('chat.guide.gone', { model: setup.model });
  } else if (setup.problem === 'offered') {
    text = t('chat.guide.offered', { name, model: setup.model });
  } else if (pickable) {
    text = t('chat.guide.pick');
  } else {
    text = t('chat.guide.tick');
    label = t('chat.guide.choose');
  }
  const button = accEl('button', 'btn', label);
  button.type = 'button';
  button.addEventListener('click', ev => {
    if (!pick) return aiOpenAccounts(setup.account?.id);
    ev.stopPropagation();
    aiShowModels(button, vaultSettings().chat_model, assistantPickModel);
  });
  guide.append(accEl('div', '', text), button);
  return guide;
}

// The label row: who and when, and the arrows through the siblings.
function renderAssistantLabel(node, message, children) {
  node.label.replaceChildren();
  let who;
  if (message.role === 'user') {
    who = t('chat.you_at', { time: assistantTime(message.date, true) });
  } else if (message.status === 'writing') {
    who = t('chat.writing_label');
  } else if (message.status === 'failed') {
    who = t('chat.failed_label');
  } else {
    const time = assistantTime(message.date);
    who = message.status === 'stopped' ? t('chat.stopped_at', { time }) : t('chat.assistant_at', { time });
  }
  const whoEl = accEl('span', '', who);
  if (message.model) whoEl.title = message.model;
  node.label.append(whoEl);

  const siblings = [...siblingsOf(message, children).list].sort((a, b) => (a.id < b.id ? -1 : 1));
  if (siblings.length < 2) return;
  const index = siblings.findIndex(m => m.id === message.id);
  const nav = accEl('span', 'chat__nav');
  const step = (text, title, target) => {
    const button = accEl('button', 'tool', text);
    button.type = 'button';
    titled(button, title);
    button.disabled = !target;
    if (target) button.addEventListener('click', () => assistantSelect(target.id));
    return button;
  };
  nav.append(
    step('\u2039', t('chat.previous_version'), siblings[index - 1]),
    accEl('span', 'chat__nav-count', `${index + 1} / ${siblings.length}`),
    step('\u203A', t('chat.next_version'), siblings[index + 1]));
  node.label.append(nav);
}

function createAssistantUserNode(message) {
  const el = accEl('div', 'chat__msg chat__msg--user');
  const label = accEl('div', 'chat__label label');
  const body = accEl('div', 'editor prose chat__bubble');
  body.title = t('chat.click_to_edit');
  const actions = accEl('div', 'chat__actions');
  actions.hidden = true;
  const cancel = accEl('button', 'btn btn--ghost', t('field.cancel'));
  cancel.type = 'button';
  cancel.addEventListener('click', () => assistantEndEdit());
  const send = accEl('button', 'chat__send chat__send--small');
  send.type = 'button';
  send.append(iconEl('#i-send'));
  send.title = t('chat.send_again');
  send.addEventListener('click', () => assistantResend());
  actions.append(cancel, send);
  el.append(label, body, actions);

  const editor = createEditor(body, {
    onEscape: () => assistantEndEdit(),
    onSubmit: () => assistantResend(),
    onChange: () => updateAssistantButtons(),
  });
  body.addEventListener('click', () => {
    if (!editor.isEditable()) assistantStartEdit(message.id);
  });
  return { el, label, body, role: 'user', editor, actions, send, html: null };
}

// A reply, and below it once written: "+ Task from this" and Save as
// document (`renderAssistantReplyActions`).
function createAssistantReplyNode(message) {
  const el = accEl('div', 'chat__msg');
  const label = accEl('div', 'chat__label label');
  const body = accEl('div', 'editor prose chat__reply');
  const actions = accEl('div', 'chat__actions');
  actions.hidden = true;
  const task = accEl('button', 'btn', t('chat.task_from_this'));
  task.type = 'button';
  task.title = t('chat.task_from_this_title');
  el.append(label, body, actions);
  const node = { id: message.id, el, label, body, role: 'assistant', actions, task, saved: null, content: '', html: null, personal: false };
  task.addEventListener('click', () => runAction('task.with', node.content, node.personal));
  return node;
}

// The reply's Save as document -- "Saving..." while it is saved -- or, once
// saved, the note of the document, while it is in the vault.
function renderAssistantReplyActions(node) {
  const saved = assistantSaved.get(node.id);
  const key = saved?.busy ? 'busy' : saved && entities.has(saved.id) ? saved.id : '';
  if (node.saved === key) return;
  node.saved = key;
  if (key && key !== 'busy') {
    node.actions.replaceChildren(node.task, createdNote(t('chat.saved_as', { title: saved.title }), saved.id, t('chat.show_document')));
    return;
  }
  const save = accEl('button', 'btn', key ? t('chat.saving') : t('chat.save_as_document'));
  save.type = 'button';
  save.disabled = !!key;
  save.title = t('chat.save_as_document_title');
  save.addEventListener('click', () => assistantSaveReply(node));
  node.actions.replaceChildren(node.task, save);
}

function updateAssistantNode(node, message, children) {
  renderAssistantLabel(node, message, children);
  if (node.role === 'user') {
    if (node.html !== message.html && !node.editor.isEditable()) {
      node.editor.setHTML(message.html);
      node.html = message.html;
    }
    return;
  }
  node.body.classList.toggle('chat__reply--writing', message.status === 'writing');
  node.body.classList.toggle('error-note', message.status === 'failed');
  const html = message.status === 'failed' ? null : message.html;
  if (message.status === 'failed') {
    node.body.textContent = message.error ? textOf(message.error) : t('chat.reply_failed');
  } else if (node.html !== html) {
    node.body.innerHTML = html;
  }
  node.html = html;
  node.content = message.content ?? '';
  node.personal = !!message.personal;
  renderAssistantLeftOut(node, message);
  node.actions.hidden = !['done', 'stopped'].includes(message.status) || !node.content.trim();
  if (!node.actions.hidden) renderAssistantReplyActions(node);
}

// What the reply's model wasn't given, by their classification
// (assistant.md#context), under the reply.
function renderAssistantLeftOut(node, message) {
  const left = message.left_out ?? [];
  let note = node.el.querySelector(':scope > .chat__left-out');
  if (!left.length) {
    note?.remove();
    return;
  }
  if (!note) {
    note = accEl('div', 'meta chat__left-out');
    node.body.after(note);
  }
  const kept = left.join(' \u00b7 ');
  note.textContent = message.model ? t('chat.kept_from', { model: message.model, kept }) : t('chat.kept_from_model', { kept });
}

// The input's button: send (dimmed without text), or stop while the reply
// is written; the edited message's send button follows the same rules.
function updateAssistantButtons(setup = assistantSetup()) {
  const writing = assistantWriting();
  const busy = !!assistantSend || writing || !!setup.problem;
  const empty = !assistantDraftText();
  assistantInputEl.toggleAttribute('data-empty', empty);
  const enabled = !setup.problem;
  if (assistantInput.isEditable() !== enabled) {
    assistantInput.setEditable(enabled, false);
    if (enabled && !assistantInputEl.firstElementChild) assistantInput.setHTML(ASSISTANT_EMPTY_HTML);
  }
  assistantInputEl.dataset.placeholder = enabled ? t('chat.ask_anything') : t('chat.pick_to_chat');

  if (writing) {
    assistantSendEl.querySelector('use').setAttribute('href', '#i-stop');
    assistantSendEl.title = t('chat.stop_reply');
    assistantSendEl.dataset.mode = 'stop';
    assistantSendEl.disabled = false;
  } else {
    assistantSendEl.querySelector('use').setAttribute('href', '#i-send');
    assistantSendEl.title = t('chat.send');
    assistantSendEl.dataset.mode = 'send';
    assistantSendEl.disabled = empty || busy || (!!assistantOpen && !assistantConversation);
  }

  const node = assistantEditing && assistantNodes.get(assistantEditing.id);
  if (node) node.send.disabled = busy || !serializeMarkdown(node.body).trim();
}

// ----------------------------------------------------------------------------
// setup

const assistantInput = createEditor(assistantInputEl, {
  onSubmit: () => assistantSendInput(),
  onChange: () => updateAssistantButtons(),
});
assistantInput.setHTML(ASSISTANT_EMPTY_HTML);

assistantSendEl.addEventListener('click', () => {
  if (assistantSendEl.dataset.mode === 'stop') assistantStop();
  else assistantSendInput();
});

assistantSwitchEl.addEventListener('click', ev => {
  ev.stopPropagation();
  assistantSwitch();
});

// Links in the chat never navigate the app away; one in a reply opens in
// its default app (`openLink`, features/selection.js). A click in a
// message of the user
// edits it instead.
assistantMessagesEl.addEventListener('click', ev => {
  const link = ev.target.closest('a');
  if (!link) return;
  ev.preventDefault();
  if (link.closest('.chat__reply')) openLink(link.getAttribute('href'));
});

store.subscribe([
  state => state.selected_lens, state => state.selected_day?.getTime(), state => state.selected_entity,
  state => state.selected_thread, state => state.threads_rev,
], (_values, state) => assistantShow(state));

store.subscribe([state => state.accounts_rev, state => state.settings_rev], () => renderAssistant());

// The cards' chat buttons are pressed while the chat is open beside them.
store.subscribe(state => state.panel_size, panel_size => {
  for (const btn of document.querySelectorAll('#panel-body [data-cmd="chat"]')) {
    btn.setAttribute('aria-pressed', String(panel_size === 'chat'));
  }
});

// A conversation written (or deleted) changes the switcher's count; the conversation shown
// leaving the vault is gone; a reply's saved document leaving it brings
// back Save as document.
store.subscribe(state => state.entities_rev, () => {
  if (assistantConversation && !entities.has(assistantOpen)) onConversation(assistantOpen, null);
  scheduleRenderAssistantHead();
  for (const node of assistantNodes.values()) {
    if (node.role === 'assistant' && !node.actions.hidden) renderAssistantReplyActions(node);
  }
});

rpcOn('conversation', conversation => onConversation(conversation.id, conversation));
rpcOn('ai_delta', ({ conversation, message, html }) => onAiDelta(conversation, message, html));
