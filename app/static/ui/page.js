// The pages of the expanded vault area (ui.md). A page registers what Esc and
// Ctrl+Enter do on it (ui.md#keys-and-actions); `replaceVaultPage` shows it in
// place of the one shown, and the input focused before stays focused.

import { $ } from '../core/dom.js';
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

// ----------------------------------------------------------------------------
// The vault area is expanded only while a page is shown in it: a page
// opens it (`openVaultOp`), its x, Esc (`registerVaultPage`) or a filter
// click closes it (`closeVaultArea`). The layout follows `vault_expanded`
// (features/layout.js).

// vault operations: selecting one titles the vault area and expands the panel
const vaultOpTitles = {
  new: "New",
  trash: "Trash",
};
const vaultAreaTitle = $("vault-area-title");

// Selecting an op titles the vault area and expands it. Each op renders
// its own body (features/workspace/trash.js; features/new.js).
export function openVaultOp(op, title) {
  vaultAreaTitle.textContent = vaultOpTitles[op];
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
