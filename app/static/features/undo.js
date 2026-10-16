// Undo and redo (saving.md#undo): Ctrl+Z takes the last edit back,
// Ctrl+Y or Ctrl+Shift+Z does it again -- outside a field or an editor,
// which keep the keys for their own undo (`typing`). The host does it and
// says so in the status bar (the `notice` event). Each is sent as a
// structural edit (`structureCall`) and ends as one does, the selection
// kept.

import { registerAction } from '../core/actions.js';
import { typing } from '../core/dom.js';
import { t } from '../core/text.js';
import { store } from '../model/state.js';
import { structureCall } from '../model/structure.js';

function replay(cmd) {
  structureCall(cmd, {}, () => store.getState());
}

registerAction({
  id: 'undo', title: t('undo.undo'), shortcut: 'Ctrl+Z',
  when: () => !typing(),
  run: () => replay('undo'),
});
for (const shortcut of ['Ctrl+Y', 'Ctrl+Shift+Z']) {
  registerAction({
    id: 'redo', title: t('undo.redo'), shortcut,
    when: () => !typing(),
    run: () => replay('redo'),
  });
}
