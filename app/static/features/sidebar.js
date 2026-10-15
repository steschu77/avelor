// The vault panel: the vault's name and menu.

import { $ } from '../core/dom.js';
import { t } from '../core/text.js';
import { store } from '../model/state.js';
import { showMenu } from '../ui/menu.js';
import { revealVault, switchVault } from './workspace/vault.js';

// ----------------------------------------------------------------------------
store.subscribe(state => state.vault_name,
  vault_name => { $('vault-name').textContent = vault_name || t('sidebar.no_vault'); });

$('vault-menu-btn').addEventListener('click', ev => {
  ev.stopPropagation();
  const items = [
    { label: t('sidebar.switch_vault'), run: switchVault },
    { label: t('sidebar.show_in_folder'), run: revealVault },
  ];
  showMenu(ev.currentTarget, items);
});
