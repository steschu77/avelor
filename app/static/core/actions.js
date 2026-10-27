// The page's actions (architecture.md#actions): what a key, a menu, a
// button or Go to (ui.md#go-to) starts, registered once by id. One keydown
// handler runs them by their shortcut: the first action with the key's
// shortcut whose `when` holds. A key something else already handled
// (`defaultPrevented`, e.g. Ctrl+I as italic in an editor) is left alone,
// and none acts behind an open dialog.
// The menu registers its Esc (ui/menu.js) before the vault pages theirs
// and their Ctrl+Enter (ui/page.js): app.js imports the menu first.

import { typing } from './dom.js';
import { t } from './text.js';

const actions = []; // { id, title, shortcut, outsideFields, host, when, run, group, place, icon, also, page, end, archived, where }, in registration order
const sources = []; // functions, each giving the actions of what the vault has

// `shortcut`: the key with its modifiers, like 'Ctrl+N', 'Ctrl+Enter' or
// 'Escape' (see `shortcutOf`), or several that do the same;
// `outsideFields`: the key is a field's or an editor's own while one has
// the focus; `host`: the host takes the key, before the page; `when()`:
// whether the action applies now (always, if not given). `group`: where Go
// to lists it (features/palette.js), by its `title`, a text or a function
// giving it as it is now, with its `icon`; `also`: other words it is found
// by, with commas between; `page`: it opens a page. `place`: it goes to a
// place Go to finds by name, a lens, a page, a project, a source; `end()`:
// what stands at its row's end then; `archived`: a place after the others.
// Without a group or a place only a key or a caller runs it. `where`: the
// part of the app its key works in, 'panel', anywhere if not given.
export function registerAction(action) {
  actions.push(action);
}

// The actions of what the vault has, a source's, a project's: `list()` gives
// them as they are when Go to asks; none has a shortcut.
export function registerActions(list) {
  sources.push(list);
}

// Run the action `id` with `args` (e.g. the button a menu opens at), if it
// applies now; a key runs it without any.
export function runAction(id, ...args) {
  const action = actions.find(a => a.id === id);
  if (action && applies(action)) action.run(...args);
}

// The actions with a group or a place that apply now, what the vault has
// first: their title a text, `also` and their shortcuts lists.
export function listedActions() {
  return [...sources.flatMap(list => list()), ...actions].filter(a => (a.group || a.place) && applies(a)).map(listed);
}

// Of the actions with a group those with a shortcut, one that doesn't apply
// now without its `run`.
export function keyedActions() {
  return actions.filter(a => a.group && a.shortcut).map(a => ({ ...listed(a), run: applies(a) ? a.run : null }));
}

function listed(action) {
  return {
    ...action,
    title: typeof action.title === 'function' ? action.title() : action.title,
    also: (action.also ?? '').split(',').map(word => word.trim()).filter(Boolean),
    shortcuts: shortcutsOf(action),
  };
}

function applies(action) {
  return !action.when || action.when();
}

function shortcutsOf(action) {
  return action.shortcut ? [action.shortcut].flat() : [];
}

// ----------------------------------------------------------------------------
// The shortcut a key event stands for: Ctrl, Alt and Shift in this order,
// then the key, a letter in upper case.
export function shortcutOf(e) {
  const key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
  return [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', key]
    .filter(Boolean).join('+');
}

// A shortcut's keys as the run's language names them, and together as a
// text: "Ctrl+Enter".
const KEY_NAMES = {
  Ctrl: t('key.ctrl'), Shift: t('key.shift'), Enter: t('key.enter'), Escape: t('key.esc'), Tab: t('key.tab'),
  Space: t('key.space'), Backspace: t('key.backspace'), Plus: '+', Minus: '\u2212', Up: '\u2191', Down: '\u2193',
  Left: '\u2190', Right: '\u2192',
};
export function shortcutKeys(shortcut) {
  return shortcut.split('+').map(key => KEY_NAMES[key] ?? key);
}
export function shortcutText(shortcut) {
  return shortcutKeys(shortcut).join('+');
}

// The shortcut of the action `id`, as a hint says it: "CTRL+N".
export function shortcutHint(id) {
  const [shortcut] = shortcutsOf(actions.find(a => a.id === id));
  return shortcutText(shortcut).toUpperCase();
}

document.addEventListener('keydown', e => {
  if (e.defaultPrevented || e.isComposing || document.querySelector('dialog[open]')) return;
  const shortcut = shortcutOf(e);
  const action = actions.find(a => !a.host && shortcutsOf(a).includes(shortcut) && !(a.outsideFields && typing())
    && applies(a));
  if (!action) return;
  e.preventDefault();
  action.run();
});
