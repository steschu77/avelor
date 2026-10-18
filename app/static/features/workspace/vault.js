// Workspace -> Vault: the open vault, and the files its scan couldn't read
// (`vault_error`), rendered into the expanded vault area while its op is
// "settings".
// Also `switchVault` and `revealVault`, which the sidebar's vault menu
// offers too.

import { accEl, oncePerFrame } from '../../core/dom.js';
import { plural } from '../../core/format.js';
import { rpcCall } from '../../core/rpc.js';
import { showError } from '../../model/state.js';
import { store } from '../../model/state.js';
import { trySave } from '../../model/documents.js';
import { replaceVaultPage } from '../../ui/page.js';
import { leaveEditModes } from '../header.js';

const scheduleRenderVault = oncePerFrame(renderVault);

function renderVault() {
  const state = store.getState();
  if (state.vault_op !== 'settings') return;
  const page = accEl('div', 'page vault');

  if (state.vault_state === 'none') {
    page.append(accEl('div', 'page-title', 'Vault'),
      accEl('div', 'page-text', 'No vault is open. A vault is a folder: every markdown and YAML file in it is one entity.'),
      renderVaultActions(false));
    replaceVaultPage(page);
    return;
  }

  const head = accEl('div', 'page-head');
  head.append(accEl('div', 'page-title', state.vault_name),
    accEl('span', 'meta', vaultSummary(state)));
  page.append(head,
    accEl('div', 'mono truncate-start', state.vault_path),
    renderVaultActions(true));
  if (state.vault_errors.length) page.append(renderVaultErrors(state.vault_errors));
  replaceVaultPage(page);
}

// The file count, marked as scanning until the scan is done.
function vaultSummary({ vault_state, files_total }) {
  const files = plural(files_total, 'file');
  return vault_state === 'scanning' ? `Scanning \u00b7 ${files}` : files;
}

// Show in folder (only for an open vault) and Switch vault.
function renderVaultActions(open) {
  const actions = accEl('div', 'vault__actions');
  if (open) {
    const reveal = accEl('button', 'btn', 'Show in folder');
    reveal.type = 'button';
    reveal.addEventListener('click', revealVault);
    actions.append(reveal);
  }
  const change = accEl('button', 'btn', open ? 'Switch vault\u2026' : 'Open a vault\u2026');
  change.type = 'button';
  change.addEventListener('click', switchVault);
  actions.append(change);
  return actions;
}

// The files the scan couldn't read, each with the reason.
function renderVaultErrors(errors) {
  const part = accEl('div', 'vault__errors');
  const head = accEl('div', 'label section-head', 'Files that couldn\'t be read');
  head.append(accEl('span', 'section-head__note', String(errors.length)));
  part.append(head);
  for (const { path, error } of errors) {
    const row = accEl('div', 'list-row vault__file');
    const name = accEl('span', 'mono truncate-start', path);
    name.title = path;
    row.append(name, accEl('span', 'meta', error));
    part.append(row);
  }
  return part;
}

// Re-render while visible when the op or the vault changes.
store.subscribe([
  state => state.vault_op, state => state.vault_state, state => state.vault_errors,
  state => state.files_total, state => state.vault_name, state => state.vault_path,
], () => scheduleRenderVault());

// ----------------------------------------------------------------------------
// Open another vault, picked on the host. Every editor leaves edit mode
// first and unsaved edits are sent, so they reach the vault they belong to
// and nothing typed while the old vault closes is lost (`forgetVault`); a
// cancelled picker loses nothing.
export function switchVault() {
  leaveEditModes();
  for (const doc_id of store.getState().dirty_docs) trySave(doc_id);
  rpcCall('switch_vault');
}

// Show the open vault's folder in Explorer (the vault menu, the Vault
// page); on Linux in the file manager.
export function revealVault() {
  rpcCall('reveal_vault').catch(e => {
    if (e.code !== 'cancelled') showError(e.message);
  });
}
