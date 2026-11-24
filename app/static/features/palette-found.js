// What Go to finds (ui.md#go-to): the places, the entries, the commands and
// the keys, each prepared for matching as the dialog opens, and the blocks of
// rows for the words typed. The dialog draws them (features/palette.js).

import { keyedActions, listedActions, runAction, shortcutKeys } from '../core/actions.js';
import { DATE_FORMAT, parseDate } from '../core/format.js';
import { fit, foundBy, prepare } from '../core/match.js';
import { t } from '../core/text.js';
import { store } from '../model/state.js';
import { NO_PROJECT, entities, matchesFilter, projects, projectsOf, threadProjects, threads } from '../model/entities.js';
import { KIND_ICONS, docIcon, kindName } from '../ui/icons.js';
import { closeVaultArea } from '../ui/page.js';
import { KEY_GROUPS } from './keys.js';
import { selectById } from './selection.js';

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

// How many rows a block shows at first, and how many more the row for more
// of them adds.
const PLACES = 3;
const ENTRIES = 8;
const HERE = 5; // of the entries in the filter, with others elsewhere
const ELSEWHERE = 4;
const COMMANDS = 4;
const KEYS = 2;
const MORE = 50;

// Everything that has a name, as it is now: the entries (every entity but a
// calendar, and the threads), each `here` while it is in the vault panel's
// filter, if there is one; the places and the commands, the actions with one
// (core/actions.js); the keys, in their groups.
export function collect() {
  const state = store.getState();
  const filtered = Boolean(state.selected_project || state.selected_source || state.selected_labels.length || state.word_filter);
  const threadsHere = new Set();
  const entries = [];
  for (const doc of entities.values()) {
    if (doc.kind === 'calendar') continue;
    const here = !filtered || matchesFilter(doc, state);
    if (here) for (const id of doc.threads ?? []) threadsHere.add(id);
    entries.push({ ...prepare(doc.title || t('untitled'), doc.labels), id: doc.id, here, time: doc.date?.getTime() || 0 });
  }
  for (const thread of threads.values()) {
    const time = parseDate(thread.date)?.getTime() || 0;
    entries.push({ ...prepare(thread.title || t('untitled'), thread.labels), id: thread.id, here: !filtered || threadsHere.has(thread.id), time });
  }
  const actions = listedActions().map(action => ({ ...action, ...prepare(action.title, action.also) }));
  const order = GROUPS.map(([group]) => group);
  const keyed = keyedActions().map(action => ({ ...action, name: action.title }))
    .sort((a, b) => order.indexOf(a.group) - order.indexOf(b.group));
  const key = entry => ({
    ...entry, ...prepare(entry.name, entry.also),
    pressed: entry.shortcuts.map(shortcut => [...shortcut.split('+'), ...shortcutKeys(shortcut)].map(name => prepare(name).low)),
  });
  return {
    filter: filtered ? filterName(state) : null,
    entries,
    places: actions.filter(action => action.place),
    commands: actions.filter(action => action.group),
    keys: KEY_GROUPS.map(({ first = [], keys, commands = () => false, ...group }) => (
      { ...group, keys: [...first, ...keyed.filter(commands), ...keys].map(key) })),
  };
}

// The vault panel's filter as a head says it: the project's name, alone.
function filterName({ selected_project, selected_source, selected_labels, word_filter }) {
  const alone = !selected_source && !selected_labels.length && !word_filter;
  const name = selected_project === NO_PROJECT ? t('sidebar.no_project') : projects.get(selected_project)?.name;
  return (alone && name) || '';
}

// ----------------------------------------------------------------------------
// the rows

// A row: its `icon`, its `name` (`serif`, an entry's), the other words it was
// found by (`also`, or the label a `label`), what stands at its `end`, its
// `shortcuts`; what Enter does (`verb`, `run`), and Ctrl+Enter (`second`);
// `outside` the filter; `more`, for more of its kind; `stays`: Enter leaves
// the dialog open; `works`, without a `run`: where its key works;
// `unmarked`: the words are its key's, not its name's.
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

function entryRow({ id, name, time, ...entry }, words, outside = false) {
  const doc = entities.get(id);
  const kind = doc ? (doc.kind === 'document' && doc.type) || kindName(doc.kind) : kindName('thread');
  const [project] = doc ? projectsOf(doc) : threadProjects(id);
  const what = outside ? projects.get(project)?.name || t('sidebar.no_project') : kind;
  return {
    icon: doc ? docIcon(doc) : KIND_ICONS.thread.icon, name, serif: true, outside,
    label: foundBy(words, entry).join(', '),
    end: [what, time ? DATE_FORMAT.format(time) : ''].filter(Boolean).join(' \u00b7 '),
    verb: t('palette.opens_card'),
    run: () => {
      closeVaultArea();
      selectById(id);
    },
    second: { verb: t('palette.shows_in_documents'), run: () => runAction('documents.show', id) },
  };
}

// ----------------------------------------------------------------------------
// the blocks: `{ head, note?, rows }`

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
// entries (those in the filter before the others), the commands, the keys
// that are none (by a word they start with).
// `unfolded`: how many more rows a block shows by its key, `unfold(key, n)`
// sets it.
export function foundBlocks({ filter, entries, places, commands, keys }, words, { unfolded, unfold }) {
  const cut = (key, items, n, rowOf) => {
    const shown = n + (unfolded.get(key) ?? 0);
    const rows = items.slice(0, shown).map(({ item }) => rowOf(item));
    const left = items.length - shown;
    if (left <= 0) return rows;
    const more = {
      icon: '#i-chevron-down', name: t('palette.more', { n: left }), more: true, stays: true,
      verb: t('palette.shows_more'), run: () => unfold(key, shown - n + MORE),
    };
    return [...rows, more];
  };
  const parts = [];

  const place = fitting(places, words).sort((a, b) => Boolean(a.item.archived) - Boolean(b.item.archived));
  if (place.length) {
    parts.push({ fits: place[0].fits, blocks: [{ head: t('palette.name'), rows: place.slice(0, PLACES).map(({ item }) => placeRow(item, words)) }] });
  }

  const entry = fitting(entries, words);
  if (entry.length) {
    const here = entry.filter(({ item }) => item.here);
    const there = entry.filter(({ item }) => !item.here);
    const blocks = [];
    if (here.length) {
      const head = filter === null ? t('palette.entries') : filter ? t('palette.in', { name: filter }) : t('palette.in_filter');
      blocks.push({ head, rows: cut('here', here, there.length ? HERE : ENTRIES, item => entryRow(item, words)) });
    }
    if (there.length) {
      const none = filter ? t('palette.none_in', { name: filter }) : t('palette.none_in_filter');
      blocks.push({
        head: here.length ? t('palette.elsewhere') : t('palette.entries'), note: here.length ? '' : none,
        rows: cut('there', there, ELSEWHERE, item => entryRow(item, words, true)),
      });
    }
    parts.push({ fits: entry[0].fits, blocks });
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

// Of `items`, those the words fit, the best first, then the newest.
function fitting(items, words) {
  const found = [];
  for (const item of items) {
    const fits = fit(words, item);
    if (fits) found.push({ item, fits });
  }
  return found.sort((a, b) => b.fits - a.fits || (b.item.time ?? 0) - (a.item.time ?? 0));
}
