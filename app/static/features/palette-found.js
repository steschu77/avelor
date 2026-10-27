// What Go to finds (ui.md#go-to): the places, the commands and the keys, each
// prepared for matching as the dialog opens, and the blocks of rows for the
// words typed. The dialog draws them (features/palette.js).

import { keyedActions, listedActions, shortcutKeys } from '../core/actions.js';
import { fit, foundBy, prepare } from '../core/match.js';
import { t } from '../core/text.js';
import { KEY_GROUPS } from './keys.js';

// The commands' groups in the list's order.
const GROUPS = [
  ['new', t('new.head')],
  ['assistant', t('palette.assistant')],
  ['get', t('palette.get')],
  ['go', t('palette.name')],
  ['filter', t('palette.filter')],
  ['window', t('palette.window')],
  ['vault', t('workspace.vault')],
  ['settings', t('settings.name')],
  ['help', t('palette.help')],
];

// How many rows a block shows.
const PLACES = 3;
const COMMANDS = 4;
const KEYS = 2;

// Everything that has a name, as it is now: the places and the commands, the
// actions with one (core/actions.js); the keys, in their groups.
export function collect() {
  const actions = listedActions().map(action => ({ ...action, ...prepare(action.title, action.also) }));
  const order = GROUPS.map(([group]) => group);
  const keyed = keyedActions().map(action => ({ ...action, name: action.title }))
    .sort((a, b) => order.indexOf(a.group) - order.indexOf(b.group));
  const key = entry => ({
    ...entry, ...prepare(entry.name, entry.also),
    pressed: entry.shortcuts.map(shortcut => [...shortcut.split('+'), ...shortcutKeys(shortcut)].map(name => prepare(name).low)),
  });
  return {
    places: actions.filter(action => action.place),
    commands: actions.filter(action => action.group),
    keys: KEY_GROUPS.map(({ first = [], keys, commands = () => false, ...group }) => (
      { ...group, keys: [...first, ...keyed.filter(commands), ...keys].map(key) })),
  };
}

// ----------------------------------------------------------------------------
// the rows

// A row: its `icon`, its `name`, the other words it was found by (`also`),
// what stands at its `end`, its `shortcuts`; what Enter does (`verb`, `run`);
// `works`, without a `run`: where its key works; `unmarked`: the words are its
// key's, not its name's.
function commandRow(action, words) {
  return {
    icon: action.icon, name: action.name, also: foundBy(words, action).join(', '), shortcuts: action.shortcuts,
    verb: action.page ? t('palette.opens_page') : t('palette.does_it'), run: action.run,
  };
}

function keyRow(entry, words, works) {
  const row = { ...commandRow(entry, words), unmarked: !fit(words, entry) };
  return row.run ? row : { ...row, works };
}

function placeRow(action, words) {
  return {
    icon: action.icon, name: action.name, also: foundBy(words, action).join(', '), shortcuts: action.shortcuts,
    end: action.end?.(), verb: t('palette.goes_there'), run: action.run,
  };
}

// ----------------------------------------------------------------------------
// the blocks: `{ head, rows }`

// The commands by their groups, narrowed by the words, the best fit first.
export function commandBlocks({ commands }, words) {
  return GROUPS.map(([group, head]) => ({
    head,
    rows: fitting(commands.filter(command => command.group === group), words).map(({ item }) => commandRow(item, words)),
  })).filter(block => block.rows.length);
}

// The keys by where they work, narrowed by the words: what one does, or the
// keys typed ("ctrl e").
export function keyBlocks({ keys }, words) {
  const typed = words.flatMap(({ word }) => word.split('+')).filter(Boolean);
  return keys.map(group => ({
    head: group.head,
    rows: group.keys
      .filter(entry => fit(words, entry) || entry.pressed.some(names => hasKeys(names, typed)))
      .map(entry => keyRow(entry, words, group.works)),
  })).filter(block => block.rows.length);
}

// Whether the keys typed are among a shortcut's, the last one perhaps begun.
function hasKeys(names, typed) {
  return typed.length > 0 && typed.every((word, i) => (
    names.some(name => name === word || (i === typed.length - 1 && word.length > 1 && name.startsWith(word)))));
}

// What the words find, the block with the best match first: the places, the
// commands, the keys that are none (by a word they start with).
export function foundBlocks({ places, commands, keys }, words) {
  const parts = [];

  const place = fitting(places, words).sort((a, b) => Boolean(a.item.archived) - Boolean(b.item.archived));
  if (place.length) {
    parts.push({ fits: place[0].fits, blocks: [{ head: t('palette.name'), rows: place.slice(0, PLACES).map(({ item }) => placeRow(item, words)) }] });
  }

  const command = fitting(commands.filter(action => !action.place), words);
  if (command.length) {
    parts.push({ fits: command[0].fits, blocks: [{ head: t('palette.commands'), rows: command.slice(0, COMMANDS).map(({ item }) => commandRow(item, words)) }] });
  }

  const others = keys.filter(group => !group.inGoTo).flatMap(group => group.keys.filter(entry => !entry.group).map(entry => ({ ...entry, works: group.works })));
  const key = fitting(others, words).filter(({ fits }) => fits > 2);
  if (key.length) {
    const rows = key.slice(0, KEYS).map(({ item }) => keyRow(item, words, item.works));
    parts.push({ fits: key[0].fits - 0.1, blocks: [{ head: t('palette.keys'), rows }] });
  }
  return parts.sort((a, b) => b.fits - a.fits).flatMap(part => part.blocks);
}

// Of `items`, those the words fit, the best first.
function fitting(items, words) {
  const found = [];
  for (const item of items) {
    const fits = fit(words, item);
    if (fits) found.push({ item, fits });
  }
  return found.sort((a, b) => b.fits - a.fits);
}
