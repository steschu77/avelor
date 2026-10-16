// The page's actions (architecture.md#actions): what a key, a
// menu or a button starts, registered once by id. One keydown handler runs
// them by their shortcut: the first action with the key's shortcut whose
// `when` holds. A key something else already handled (`defaultPrevented`,
// e.g. Ctrl+I as italic in an editor) is left alone.
// The menu registers its Esc (ui/menu.js) before the vault pages theirs
// and their Ctrl+Enter (ui/page.js): app.js imports the menu first.

const actions = []; // { id, title, shortcut, when, run }, in registration order

// `shortcut`: the key with its modifiers, like 'Ctrl+N', 'Ctrl+Enter' or
// 'Escape' (see `shortcutOf`); `when()`: whether the action applies now
// (always, if not given).
export function registerAction(action) {
  actions.push(action);
}

// Run the action `id` with `args` (e.g. the button a menu opens at), if it
// applies now; a key runs it without any.
export function runAction(id, ...args) {
  const action = actions.find(a => a.id === id);
  if (action && (!action.when || action.when())) action.run(...args);
}

// The shortcut a key event stands for: Ctrl, Alt and Shift in this order,
// then the key, a letter in upper case.
function shortcutOf(e) {
  const key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
  return [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', key]
    .filter(Boolean).join('+');
}

document.addEventListener('keydown', e => {
  if (e.defaultPrevented || e.isComposing) return;
  const shortcut = shortcutOf(e);
  const action = actions.find(a => a.shortcut === shortcut && (!a.when || a.when()));
  if (!action) return;
  e.preventDefault();
  action.run();
});
