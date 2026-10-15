// The host's events into the model (api.md#events): the vault being
// opened.

import { rpcOn } from '../core/rpc.js';
import { store } from './state.js';

// The vault being opened: its scan starts (the vault's name).
rpcOn('vault', ({ state, name }) => {
  if (state !== 'scanning') return;
  store.setState({ vault_name: name });
});
