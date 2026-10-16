// Undo and redo (saving.md#undo): Ctrl+Z takes the last edit back,
// Ctrl+Y or Ctrl+Shift+Z does it again -- outside a field or an editor,
// which keep the keys for their own undo, or from Go to. A card still in
// edit mode leaves it first, saving, so its editing session is the last
// step.
// The host does it and says so in the status bar (the `notice` event).
// Each is sent as a structural edit (`structureCall`) and ends as one
// does, the selection kept.

import { registerAction } from '../core/actions.js';
import { t } from '../core/text.js';
import { store, vaultOpen } from '../model/state.js';
import { structureCall } from '../model/structure.js';
import { leaveEditModes } from './header.js';

function replay(cmd) {
  leaveEditModes();
  structureCall(cmd, {}, () => store.getState());
}

registerAction({
  id: 'undo', group: 'window', title: t('undo.undo'), icon: '#i-arrow-undo',
  shortcut: 'Ctrl+Z', outsideFields: true,
  when: vaultOpen,
  run: () => replay('undo'),
});
registerAction({
  id: 'redo', group: 'window', title: t('undo.redo'), icon: '#i-arrow-undo',
  shortcut: ['Ctrl+Y', 'Ctrl+Shift+Z'], outsideFields: true,
  when: vaultOpen,
  run: () => replay('redo'),
});
