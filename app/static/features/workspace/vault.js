// Workspace -> Vault: the open vault, and the files its indexing couldn't read
// (`vault_error`), rendered into the expanded vault area while its op is
// "vault".
// Also `switchVault` and `revealVault`, which the sidebar's vault menu
// offers too.

import { accEl, oncePerFrame } from '../../core/dom.js';
import { registerAction } from '../../core/actions.js';

import { rpcCall } from '../../core/rpc.js';
import { t } from '../../core/text.js';
import { vaultSettings } from '../../model/entities.js';
import { showError } from '../../model/state.js';
import { store, vaultOpen } from '../../model/state.js';
import { accSeg } from '../../ui/segmented.js';
import { CLASSIFICATIONS } from './models.js';
import { trySave } from '../../model/documents.js';
import { replaceVaultPage } from '../../ui/page.js';
import { leaveEditModes } from '../header.js';

const scheduleRenderVault = oncePerFrame(renderVault);

function renderVault() {
  const state = store.getState();
  if (state.vault_op !== 'vault') return;
  const page = accEl('div', 'page vault');

  if (!vaultOpen()) {
    page.append(accEl('div', 'page-title', t('workspace.vault')),
      accEl('div', 'page-text', t('vault.none_text')),
      renderVaultActions(false));
    replaceVaultPage(page);
    return;
  }

  const head = accEl('div', 'page-head');
  head.append(accEl('div', 'page-title', state.vault_name),
    accEl('span', 'meta', vaultSummary(state)));
  page.append(head,
    accEl('div', 'mono truncate-start', state.vault_path),
    renderVaultActions(true),
    renderVaultClassification());
  if (state.vault_errors.length) page.append(renderVaultErrors(state.vault_errors));
  replaceVaultPage(page);
}

// The file count, marked as indexing until the indexing is done.
function vaultSummary({ vault_state, files_total }) {
  const files = t('count.files', { n: files_total });
  return vault_state === 'indexing' ? t('vault.indexing', { files }) : files;
}

// Show in folder and Rebuild index (only for an open vault), and Switch
// vault.
function renderVaultActions(open) {
  const actions = accEl('div', 'vault__actions');
  if (open) {
    const reveal = accEl('button', 'btn', t('sidebar.show_in_folder'));
    reveal.type = 'button';
    reveal.addEventListener('click', revealVault);
    const rebuild = accEl('button', 'btn', t('vault.rebuild'));
    rebuild.type = 'button';
    rebuild.title = t('vault.rebuild_title');
    rebuild.addEventListener('click', rebuildIndex);
    actions.append(reveal, rebuild);
  }
  const change = accEl('button', 'btn', open ? t('sidebar.switch_vault') : t('vault.open_dots'));
  change.type = 'button';
  change.addEventListener('click', switchVault);
  actions.append(change);
  return actions;
}

// What reaches a model of what has no classification of its own
// (vault.md#vault-config, `set_vault_settings`).
function renderVaultClassification() {
  const part = accEl('div', 'vault__class');
  part.append(accEl('div', 'label section-head', t('vault.classification')));
  const current = vaultSettings().classification ?? 'open';
  const options = CLASSIFICATIONS.map(([value, label, means]) => [value, label, { title: means }]);
  const seg = accSeg(options, current, classification => {
    rpcCall('set_vault_settings', { classification }).catch(e => showError(e.message));
  });
  const means = CLASSIFICATIONS.find(([value]) => value === current)?.[2] ?? '';
  part.append(seg, accEl('div', 'meta', t('vault.classification_note', { means })));
  return part;
}

// The files the indexing couldn't read, each with the reason.
function renderVaultErrors(errors) {
  const part = accEl('div', 'vault__errors');
  const head = accEl('div', 'label section-head', t('vault.unreadable_files'));
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
  state => state.settings_rev,
], () => scheduleRenderVault());

// ----------------------------------------------------------------------------
// Before the vault opens again (a switch, Rebuild index), every editor
// leaves edit mode and unsaved edits are sent, so they reach the vault they
// belong to and nothing typed while it closes is lost (`forgetVault`).
function sendEdits() {
  leaveEditModes();
  for (const doc_id of store.getState().dirty_docs) trySave(doc_id);
}

// Open another vault, picked on the host; a cancelled picker loses nothing.
export function switchVault() {
  sendEdits();
  rpcCall('switch_vault');
}

// Read every file of the open vault again: its catalog starts over
// (vault.md#catalog), and the vault opens again as a switch to it does.
function rebuildIndex() {
  sendEdits();
  rpcCall('rebuild_index').catch(e => showError(e.message));
}

registerAction({
  id: 'vault.switch', group: 'vault', icon: '#i-library', also: t('vault.switch_also'),
  title: () => (vaultOpen() ? t('sidebar.switch_vault') : t('vault.open_dots')),
  run: switchVault,
});
registerAction({
  id: 'vault.reveal', group: 'vault', title: t('vault.reveal'), icon: '#i-folder', also: t('vault.reveal_also'),
  when: vaultOpen, run: revealVault,
});
registerAction({
  id: 'vault.rebuild', group: 'vault', title: t('vault.rebuild'), icon: '#i-arrow-sync', also: t('vault.rebuild_also'),
  when: vaultOpen, run: rebuildIndex,
});

// Show the open vault's folder in Explorer (the vault menu, the Vault
// page); on Linux in the file manager.
export function revealVault() {
  rpcCall('reveal_vault').catch(e => {
    if (e.code !== 'cancelled') showError(e.message);
  });
}
