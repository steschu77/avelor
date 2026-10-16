// `switchVault` and `revealVault`, which the sidebar's vault menu offers.

import { registerAction } from '../../core/actions.js';
import { rpcCall } from '../../core/rpc.js';
import { t } from '../../core/text.js';
import { showError } from '../../model/state.js';
import { store, vaultOpen } from '../../model/state.js';
import { trySave } from '../../model/documents.js';
import { leaveEditModes } from '../header.js';

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

registerAction({
  id: 'vault.switch', group: 'vault', icon: '#i-library', also: t('vault.switch_also'),
  title: () => (vaultOpen() ? t('sidebar.switch_vault') : t('vault.open_dots')),
  run: switchVault,
});
registerAction({
  id: 'vault.reveal', group: 'vault', title: t('vault.reveal'), icon: '#i-folder', also: t('vault.reveal_also'),
  when: vaultOpen, run: revealVault,
});

// Show the open vault's folder in Explorer (the vault menu); on Linux in
// the file manager.
export function revealVault() {
  rpcCall('reveal_vault').catch(e => {
    if (e.code !== 'cancelled') showError(e.message);
  });
}
