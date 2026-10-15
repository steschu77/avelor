// `switchVault` and `revealVault`, which the sidebar's vault menu offers.

import { rpcCall } from '../../core/rpc.js';
import { showError } from '../../model/state.js';

// ----------------------------------------------------------------------------
// Open another vault, picked on the host.
export function switchVault() {
  rpcCall('switch_vault');
}

// Show the open vault's folder in Explorer (the vault menu); on Linux in
// the file manager.
export function revealVault() {
  rpcCall('reveal_vault').catch(e => {
    if (e.code !== 'cancelled') showError(e.message);
  });
}
