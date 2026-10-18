// The pages of the expanded vault area (ui.md). A page registers what Esc and
// Ctrl+Enter do on it (ui.md#keys-and-actions); `replaceVaultPage` shows it in
// place of the one shown, and the input focused before stays focused.

import { $ } from '../core/dom.js';
import { accEl } from '../core/dom.js';
import { registerAction } from '../core/actions.js';
import { store } from '../model/state.js';

// The pages of the expanded vault area: op -> { close, submit? }. While one
// is open, Esc closes it and Ctrl+Enter runs its main action (`submit`);
// an open menu takes Esc first.
const vaultPages = new Map();

export function registerVaultPage(op, page) {
  vaultPages.set(op, page);
}

// The registered page shown in the expanded vault area, if any.
export function shownVaultPage() {
  const { vault_op, vault_expanded } = store.getState();
  return vault_expanded ? vaultPages.get(vault_op) : undefined;
}

registerAction({
  id: 'page.close', title: 'Close the page', shortcut: 'Escape',
  when: () => !!shownVaultPage(),
  run: () => shownVaultPage().close(),
});
registerAction({
  id: 'page.submit', title: 'Run the page\u2019s main action', shortcut: 'Ctrl+Enter',
  when: () => !!shownVaultPage()?.submit,
  run: () => shownVaultPage().submit(),
});

// ----------------------------------------------------------------------------
let pageFocus = null; // data-focus-key of the input to focus after the next render (see replaceVaultPage)

// Focus the input whose data-focus-key is `key` once the page is replaced
// next, instead of the one focused now.
export function setPageFocus(key) {
  pageFocus = key;
}

// Show `page` in the vault area in place of the one shown. Pages are
// rebuilt on every change, so the focused input keeps its focus and caret
// (found again by its data-focus-key), unless `pageFocus` names another.
export function replaceVaultPage(page) {
  const body = $('vault-area-body');
  const active = document.activeElement;
  const focusKey = pageFocus ?? (body.contains(active) ? active.dataset.focusKey : undefined);
  const caret = !pageFocus && focusKey ? [active.selectionStart, active.selectionEnd] : null;
  pageFocus = null;

  body.replaceChildren(page);

  const input = focusKey && [...body.querySelectorAll('[data-focus-key]')]
    .find(el => el.dataset.focusKey === focusKey);
  if (input && !input.disabled) {
    input.focus();
    if (caret?.[0] != null) input.setSelectionRange(...caret);
  }
}

// The kind of a new item's pane ('new:openai' for an AI account), else
// null.
export function draftKind(selected) {
  return typeof selected === 'string' && selected.startsWith('new:') ? selected.slice(4) : null;
}

// ----------------------------------------------------------------------------
// The vault area is expanded only while a page is shown in it: a page
// opens it (`openVaultOp`), its x, Esc (`registerVaultPage`) or a filter
// click closes it (`closeVaultArea`). The layout follows `vault_expanded`
// (features/layout.js).

// vault operations: selecting one titles the vault area and expands the panel
const vaultOpTitles = {
  new: "New",
  accounts: "Account Setup",
  trash: "Trash",
};
const vaultAreaTitle = $("vault-area-title");

// Selecting an op titles the vault area and expands it. Each op renders
// its own body (features/workspace/: accounts.js, trash.js;
// features/import/: mail.js; features/new.js). Ops without a sidebar entry
// (Fetch mail) pass their title.
export function openVaultOp(op, title) {
  vaultAreaTitle.textContent = title ?? vaultOpTitles[op];
  if (store.getState().vault_op !== op) {
    $("vault-area-body").replaceChildren();
    store.setState({ vault_op: op });
  }
  store.setState({ vault_expanded: true });
}

export function closeVaultArea() {
  store.setState({ vault_expanded: false });
}

$("vault-area-close-btn").addEventListener('click', closeVaultArea);

// ----------------------------------------------------------------------------
// A list + detail page (the Accounts page): the list of items
// with the tiles that add one, then the selected item's detail; each part
// is built from the component classes (css/components.css).

export function listDetailPage(list, detail) {
  const page = accEl('div', 'list-detail');
  page.append(list, detail);
  return page;
}

// The list, under the page's title and what the page is about.
export function pageList(title, text) {
  const list = accEl('div', 'page list-detail__list');
  list.append(accEl('div', 'page-title', title), accEl('div', 'page-text', text));
  return list;
}

// A heading over a part of the list.
export function listSection(label) {
  return accEl('div', 'label list-detail__section', label);
}

// An item of the list: its kind and `status`, its `title`, and a line
// about it (`sub`); marked while `current`, `onSelect` on a click.
export function itemCard({ icon, kind, status, title, sub, cut = false, current, onSelect }) {
  const card = accEl('button', 'tile item-card');
  card.type = 'button';
  if (current) card.setAttribute('aria-current', 'true');
  const head = accEl('div', 'label item-card__head');
  head.append(accEl('span', 'item-card__kind', kind), accEl('span', 'item-card__status', status));
  card.append(head, accEl('div', 'item-card__title', title), accEl('div', 'meta', sub));
  card.addEventListener('click', onSelect);
  return card;
}

// A tile that adds an item: `label`, then `hint` at its right (a menu's
// caret); `onClick(tile)` on a click, with the tile to anchor a menu to.
export function addTile({ label, hint, current, onClick }) {
  const tile = accEl('button', 'tile add-tile');
  tile.type = 'button';
  if (current) tile.setAttribute('aria-current', 'true');
  tile.append(label, accEl('span', 'label', hint));
  tile.addEventListener('click', () => onClick(tile));
  return tile;
}

// The detail of the selected item, or of a new one.
export function detailPane() {
  return accEl('div', 'page list-detail__detail');
}

// The detail's head: its kind and state (`label`), and an action
// (`button`) at its right.
export function detailHead(label, { icon, button } = {}) {
  const head = accEl('div', 'detail-head');
  head.append(accEl('span', 'label', label));
  if (button) head.append(button);
  return head;
}

// The row of facts under the detail's title; a fact is a label, a value
// (text or an element) and an optional note (text or an element).
export function stats(...cells) {
  const row = accEl('div', 'stats');
  row.append(...cells);
  return row;
}

export function stat(label, value, note) {
  const cell = accEl('div', 'stat');
  cell.append(accEl('span', 'label', label));
  cell.append(typeof value === 'string' ? accEl('span', 'subtitle', value) : value);
  if (note) cell.append(typeof note === 'string' ? accEl('span', 'meta', note) : note);
  return cell;
}

// A notice in the detail: news ('info') or what failed ('error'), of text
// and elements, a button after the text.
const NOTICE_TYPES = { info: 'prose', error: 'error-note' };

export function notice(kind, ...content) {
  const el = accEl('div', `notice ${NOTICE_TYPES[kind]}`);
  el.append(...content);
  return el;
}

// What failed (`message`), with Retry, which calls `retry`.
export function failedRetry(message, retry) {
  const button = accEl('button', 'btn', 'Retry');
  button.type = 'button';
  button.addEventListener('click', retry);
  return notice('error', message, button);
}

// The detail's foot: what its action does (`text`), then the action.
export function detailFooter(text, button) {
  const footer = accEl('div', 'detail-footer');
  footer.append(accEl('span', 'page-text', text), button);
  return footer;
}

// ----------------------------------------------------------------------------
// Changes sent to the host for an item, shown until the last call for it
// has settled (the host's event, which comes before each answer, has the
// item then): `get(id)` the changes still shown; `send(id, change, call)`
// shows `change` and returns `call()`'s promise; `forget(id)` for an item
// that is gone.
export function sentDrafts() {
  const drafts = new Map(); // id -> the changes sent
  const pending = new Map(); // id -> how many of its calls are unsettled
  return {
    get: id => drafts.get(id),
    send(id, change, call) {
      drafts.set(id, { ...drafts.get(id), ...change });
      pending.set(id, (pending.get(id) ?? 0) + 1);
      return call().finally(() => {
        const left = (pending.get(id) ?? 1) - 1;
        if (left > 0) {
          pending.set(id, left);
        } else {
          pending.delete(id);
          drafts.delete(id);
        }
      });
    },
    forget(id) {
      drafts.delete(id);
      pending.delete(id);
    },
  };
}
