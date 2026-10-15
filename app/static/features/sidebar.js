// The vault panel: the vault's name and menu.

import { $ } from '../core/dom.js';
import { store } from '../model/state.js';
import { showMenu } from '../ui/menu.js';
import { revealVault, switchVault } from './workspace/vault.js';

// ----------------------------------------------------------------------------
store.subscribe(state => state.vault_name,
  vault_name => { $('vault-name').textContent = vault_name || 'No vault'; });

$('vault-menu-btn').addEventListener('click', ev => {
  ev.stopPropagation();
  const items = [
    { label: 'Switch vault\u2026', run: switchVault },
    { label: 'Show in folder', run: revealVault },
  ];
  showMenu(ev.currentTarget, items);
});
