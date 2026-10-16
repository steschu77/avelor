// The keys that are no command, by where they work: Go to lists them with the
// commands' (ui.md#go-to). The editor's are its own, named as its toolbar's
// buttons.

import { $ } from '../core/dom.js';
import { t } from '../core/text.js';
import { EDITOR_KEYS } from '../editor/editor.js';

export const GO_TO = 'Ctrl+Shift+P';

const key = (name, shortcuts, icon = '#i-keyboard') => ({ name, shortcuts, icon });

const toolbar = $('editor-toolbar-template').content;
function editorKey({ command, shortcut }) {
  const btn = toolbar.querySelector(`[data-cmd="${command}"]`);
  return key(btn.title, [shortcut], btn.querySelector('use')?.getAttribute('href'));
}

// Each group's `head`, what the foot says of a key of it (`works`), its keys
// (`first` before the commands', `keys` after them) and whether it has a
// command's (`commands`, of an action); `inGoTo`: those of Go to itself,
// which it doesn't look through with everything else.
export const KEY_GROUPS = [
  {
    head: t('keys.anywhere'), works: t('keys.works_anywhere'), commands: () => true,
    first: [key(t('palette.name'), [GO_TO], '#i-arrow-right')],
    keys: [
      key(t('keys.next_control'), ['Tab']),
      key(t('keys.previous_control'), ['Shift+Tab']),
      key(t('keys.press'), ['Enter', 'Space']),
      key(t('keys.create'), ['Ctrl+Enter'], '#i-checkmark'),
      key(t('keys.discard'), ['Escape'], '#i-dismiss'),
    ],
  },
  {
    head: t('keys.text'), works: t('keys.works_in_text'),
    keys: [
      ...EDITOR_KEYS.map(editorKey),
      key(t('keys.further_in'), ['Tab']),
      key(t('keys.further_out'), ['Shift+Tab']),
      key(t('keys.leave_text'), ['Escape'], '#i-dismiss'),
    ],
  },
  {
    head: t('keys.menu'), works: t('keys.works_there'),
    keys: [
      key(t('keys.in_menu'), ['Up', 'Down']),
      key(t('keys.divider'), ['Left', 'Right']),
    ],
  },
  {
    head: t('keys.go_to'), works: t('keys.works_here'), inGoTo: true,
    keys: [
      key(t('keys.rows'), ['Up', 'Down']),
      key(t('keys.act'), ['Enter']),
      key(t('keys.narrow'), ['Tab']),
      key(t('keys.back'), ['Backspace']),
      key(t('keys.close'), ['Escape']),
    ],
  },
];
