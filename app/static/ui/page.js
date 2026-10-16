// The pages of the expanded vault area (ui.md). `replaceVaultPage` shows a
// page in place of the one shown.

import { $ } from '../core/dom.js';
import { store } from '../model/state.js';

// Show `page` in the vault area in place of the one shown.
export function replaceVaultPage(page) {
  $('vault-area-body').replaceChildren(page);
}

// ----------------------------------------------------------------------------
// The vault area is expanded only while a page is shown in it: a page
// opens it (`openVaultOp`), its x or a filter click closes it
// (`closeVaultArea`). The layout follows `vault_expanded`
// (features/layout.js).

// vault operations: selecting one titles the vault area and expands the panel
const vaultOpTitles = {
  trash: "Trash",
};
const vaultAreaTitle = $("vault-area-title");

// Selecting an op titles the vault area and expands it. Each op renders
// its own body (features/workspace/trash.js).
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
