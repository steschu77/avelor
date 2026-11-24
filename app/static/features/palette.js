// Go to (ui.md#go-to): places, entries, commands and keys by name, at
// Ctrl+Shift+P or the vault panel's item under New. The dialog: its box, what
// it is narrowed to, the blocks found (features/palette-found.js) and the
// foot; Enter runs the highlighted row, once the dialog is closed and the
// focus is back where it was.

import { $, accEl } from '../core/dom.js';
import { registerAction, runAction, shortcutHint, shortcutKeys, shortcutOf } from '../core/actions.js';
import { cutAtWords, wordsOf } from '../core/match.js';
import { t } from '../core/text.js';
import { iconEl } from '../ui/icons.js';
import { GO_TO } from './keys.js';
import { collect, commandBlocks, foundBlocks, keyBlocks } from './palette-found.js';

const PAGE = 8; // the rows PageUp and PageDown go over

// What it is narrowed to, as Tab goes round them: everything, the commands,
// the keys; what the box asks for, and what is said when nothing is found.
const NARROW = ['', '>', 'keys'];
const HINTS = { '': t('palette.placeholder'), '>': t('palette.which_command'), keys: t('palette.which_key') };
const NONE = { '': t('palette.none'), '>': t('palette.none'), keys: t('palette.no_key') };

const dialogEl = $('palette');
const inputEl = $('palette-input');
const narrowEl = $('palette-narrow');
const listEl = $('palette-list');
const footEl = $('palette-foot');

let found = null; // what has a name, as it was when it opened (`collect`)
let narrow = ''; // one of NARROW
const unfolded = new Map(); // how many more rows a block shows, by its key
let rows = []; // { row, el }, in the list's order
let at = 0; // the highlighted row

registerAction({ id: 'palette', shortcut: GO_TO, run: () => open() });
registerAction({
  id: 'keys', group: 'help', title: t('palette.shortcuts'), icon: '#i-keyboard', also: t('palette.shortcuts_also'),
  run: () => open('keys'),
});
$('goto-item').addEventListener('click', () => runAction('palette'));
for (const el of document.querySelectorAll('kbd[data-shortcut]')) el.textContent = shortcutHint(el.dataset.shortcut);

function open(to = '') {
  runAction('menu.close');
  found = collect();
  dialogEl.showModal();
  narrowTo(to, '');
}

// Narrowed to `to`, with `text` in the box.
function narrowTo(to, text = inputEl.value) {
  narrow = to;
  type(text);
}

function type(text) {
  if (inputEl.value !== text) inputEl.value = text;
  unfolded.clear();
  render();
  inputEl.focus();
}

function runRow(second = false) {
  const row = rows[at]?.row;
  const act = second ? row?.second : row;
  if (!act?.run) return;
  if (!row.stays) dialogEl.close();
  act.run();
}

// ----------------------------------------------------------------------------
// the list

function render(to = 0) {
  const words = wordsOf(inputEl.value);
  const unfold = (key, n) => {
    unfolded.set(key, n);
    render(at); // the first of those unfolded
  };
  const everything = !narrow && words.length;
  const blocks = everything ? foundBlocks(found, words, { unfolded, unfold })
    : narrow === 'keys' ? keyBlocks(found, words) : commandBlocks(found, words);
  rows = [];
  const none = everything ? t('palette.nothing', { text: inputEl.value.trim() }) : NONE[narrow];
  listEl.replaceChildren(...(blocks.length ? blocks.map(block => blockEl(block, words)) : [accEl('div', 'palette__none', none)]));
  for (const option of narrowEl.children) option.setAttribute('aria-pressed', String(option.dataset.narrow === narrow));
  inputEl.placeholder = HINTS[narrow];
  highlight(to);
}

function blockEl({ head, note, rows: blockRows }, words) {
  const el = accEl('div');
  el.setAttribute('role', 'group');
  el.setAttribute('aria-label', head);
  const headEl = accEl('div', 'palette__head');
  headEl.append(accEl('span', 'label', head));
  if (note) headEl.append(accEl('span', 'note', note));
  el.append(headEl, ...blockRows.map(row => rowEl(row, words)));
  return el;
}

function rowEl(row, words) {
  const el = accEl('div', 'palette__row');
  el.classList.toggle('is-outside', Boolean(row.outside));
  el.classList.toggle('palette__row--more', Boolean(row.more));
  el.id = `palette-row-${rows.length}`;
  el.dataset.at = rows.length;
  el.setAttribute('role', 'option');
  const name = accEl('span', 'palette__name');
  const title = accEl('span', row.serif ? 'row-title' : 'row-title row-title--ui');
  title.append(...(row.more || row.unmarked ? [row.name] : markedNodes(words, row.name)));
  name.append(title);
  if (row.also || row.label) {
    const hit = accEl('span', 'palette__hit');
    if (row.label) hit.append(accEl('span', 'label', t('search.in_label')));
    hit.append(...markedNodes(words, row.label || row.also));
    name.append(hit);
  }
  const end = accEl('span', 'palette__end');
  if (row.end) end.append(accEl('span', 'meta', row.end));
  if (row.shortcuts?.length) end.append(keysEl(row.shortcuts));
  el.append(iconEl(row.icon), name, end);
  rows.push({ row, el });
  return el;
}

function markedNodes(words, text) {
  return cutAtWords(words, text).map(piece => (piece.hit ? accEl('mark', '', piece.text) : piece.text));
}

// Shortcuts as their keys, each drawn as one.
function keysEl(shortcuts) {
  const el = accEl('span', 'keys');
  shortcuts.forEach((shortcut, i) => {
    if (i) el.append(accEl('span', 'meta', t('palette.or')));
    el.append(...shortcutKeys(shortcut).map(key => accEl('kbd', 'key', key)));
  });
  return el;
}

// The row Enter acts on, and what the keys do, in the foot.
function highlight(i) {
  rows[at]?.el.removeAttribute('aria-selected');
  at = Math.max(0, Math.min(i, rows.length - 1));
  const { row, el } = rows[at] ?? {};
  el?.setAttribute('aria-selected', 'true');
  inputEl.setAttribute('aria-activedescendant', el?.id ?? '');
  if (at === 0) listEl.scrollTop = 0;
  else el.scrollIntoView({ block: 'nearest' });
  const does = (shortcut, text) => {
    const doesEl = accEl('span', 'palette__does');
    doesEl.append(keysEl([shortcut]), text);
    return doesEl;
  };
  footEl.replaceChildren(
    ...(row?.run ? [does('Enter', row.verb)] : row ? [accEl('span', '', row.works)] : []),
    ...(row?.second ? [does('Ctrl+Enter', row.second.verb)] : []),
    does('Tab', t('palette.narrows')),
    does('Escape', t('palette.closes')));
}

// ----------------------------------------------------------------------------
// keys and the mouse

// A ">" typed first narrows to the commands, and isn't part of the text.
inputEl.addEventListener('input', () => {
  const text = inputEl.value;
  if (!narrow && text.startsWith('>')) narrowTo('>', text.slice(1));
  else type(text);
});

inputEl.addEventListener('keydown', e => {
  const n = rows.length;
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    if (n) highlight((at + (e.key === 'ArrowDown' ? 1 : n - 1)) % n);
  } else if (e.key === 'PageDown' || e.key === 'PageUp') {
    highlight(at + (e.key === 'PageDown' ? PAGE : -PAGE));
  } else if (e.key === 'Enter') {
    runRow(e.ctrlKey);
  } else if (e.key === 'Tab') {
    narrowTo(NARROW[(NARROW.indexOf(narrow) + (e.shiftKey ? NARROW.length - 1 : 1)) % NARROW.length]);
  } else if (e.key === 'Backspace' && narrow && !inputEl.value) {
    narrowTo('');
  } else if (shortcutOf(e) === GO_TO) {
    narrowTo('', '');
  } else {
    return;
  }
  e.preventDefault();
});

// A press narrows, the focus staying in the box.
narrowEl.addEventListener('mousedown', e => {
  e.preventDefault();
  const option = e.target.closest('[data-narrow]');
  if (option) narrowTo(option.dataset.narrow);
});

// The pointer highlights a row as it moves, not as the list scrolls under
// it; a press runs it, the focus staying in the box.
listEl.addEventListener('mousemove', e => {
  const i = e.target.closest('[data-at]')?.dataset.at;
  if (i === undefined || Number(i) === at || !(e.movementX || e.movementY)) return;
  const top = listEl.scrollTop;
  highlight(Number(i));
  listEl.scrollTop = top;
});
listEl.addEventListener('mousedown', e => {
  e.preventDefault();
  const i = e.target.closest('[data-at]')?.dataset.at;
  if (i === undefined) return;
  highlight(Number(i));
  runRow(e.ctrlKey);
});

// A press beside the dialog closes it.
dialogEl.addEventListener('mousedown', e => {
  const box = dialogEl.getBoundingClientRect();
  const inside = e.clientX >= box.left && e.clientX <= box.right && e.clientY >= box.top && e.clientY <= box.bottom;
  if (e.target === dialogEl && !inside) dialogEl.close();
});
