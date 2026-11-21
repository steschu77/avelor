// Settings (ui.md#settings): how Avelor works for the user on this PC. Its
// page, rendered into the expanded vault area while its op is "settings",
// and the Help and settings menu at the vault panel's foot.

import { $, accEl, oncePerFrame, setAria } from '../../core/dom.js';
import { registerAction, runAction, shortcutText } from '../../core/actions.js';
import { fileSize } from '../../core/format.js';
import { rpcCall } from '../../core/rpc.js';
import { locale, t } from '../../core/text.js';
import { newEventCalendar, pickNewEventCalendar } from '../../model/calendar.js';
import { entities, me } from '../../model/entities.js';
import { showError, store, vaultOpen } from '../../model/state.js';
import { iconEl } from '../../ui/icons.js';
import { dropBox, showMenu } from '../../ui/menu.js';
import { accSeg, keyedSeg } from '../../ui/segmented.js';
import {
  closeVaultArea, detailPane, listDetailPage, listSection, openVaultOp, registerVaultPage, replaceVaultPage, setPageFocus,
} from '../../ui/page.js';
import { contactMenu } from '../facts.js';
import { calendarMenu } from '../panel/event-fields.js';

const SHORTCUT = 'Ctrl+,';

const scheduleRenderSettings = oncePerFrame(renderSettings);

function renderSettings() {
  const { vault_op, language } = store.getState();
  if (vault_op !== 'settings' || !language) return;
  const parts = [lookPart(language), youPart(), pcPart()];

  const links = accEl('div', 'settings__links');
  links.append(...parts.map(({ id, title, now }) => linkRow(title, accEl('span', 'meta', now), () => showPart(id))));
  const places = accEl('div', 'settings__links');
  places.append(...PLACES.map(([name, where, open]) => {
    const end = accEl('span', 'meta settings__place', where);
    end.append(iconEl('#i-chevron-right'));
    return linkRow(name, end, open);
  }));
  const list = accEl('div', 'page list-detail__list');
  const inner = accEl('div', 'settings__list');
  inner.append(accEl('div', 'page-title', t('settings.name')), accEl('div', 'page-text', t('settings.intro')),
    listSection(t('settings.here')), links, listSection(t('settings.elsewhere')), places);
  list.append(inner);

  const detail = detailPane();
  detail.classList.add('settings__parts');
  detail.append(...parts.map(partEl));
  const page = listDetailPage(list, detail);
  page.classList.add('list-detail--settings');
  replaceVaultPage(page);
}

// What is set elsewhere: its name, the place, and how it opens.
const PLACES = [
  [t('settings.place.google'), t('workspace.accounts'), () => openVaultOp('accounts')],
  [t('settings.place.ai'), t('workspace.accounts'), () => openVaultOp('accounts')],
  [t('settings.place.folders'), t('workspace.folders'), () => openVaultOp('folders')],
  [t('settings.place.vault'), t('workspace.vault'), () => openVaultOp('vault')],
  [t('settings.place.dashboard'), t('lens.dashboard'), () => {
    closeVaultArea();
    runAction('lens.dashboard');
  }],
  [t('palette.shortcuts'), t('palette.name'), () => runAction('keys')],
];

// A row of the list: its name, and at its end what it is set to or where.
function linkRow(name, end, open) {
  const row = accEl('button', 'list-row settings__link');
  row.type = 'button';
  const title = accEl('span', 'row-title', name);
  title.title = name;
  row.append(title, end);
  row.addEventListener('click', open);
  return row;
}

// ----------------------------------------------------------------------------
// A part of the page: its `title`, where its settings hold (`scope`), what it
// is set to now, as the list says it (`now`), and its `rows`.
function partEl({ id, title, scope, rows }) {
  const part = accEl('section', 'settings__part');
  part.id = `settings-${id}`;
  part.setAttribute('aria-label', title);
  const head = accEl('div', 'label section-head', title);
  head.append(accEl('span', 'section-head__note', scope));
  part.append(head, ...rows);
  return part;
}

function showPart(id) {
  const part = $(`settings-${id}`);
  part.scrollIntoView({ block: 'start' });
  part.querySelector('[data-focus-key]:not([aria-pressed="false"])')?.focus({ preventScroll: true });
}

// A row: its name over what it does (`note`, texts and elements), its
// controls at the end.
function settingRow(name, note, ...controls) {
  const row = accEl('div', 'setting');
  const text = accEl('div', 'setting__text');
  const noteEl = accEl('span', 'note');
  noteEl.append(...note);
  text.append(accEl('span', 'setting__name', name), noteEl);
  const control = accEl('div', 'setting__control');
  control.append(...controls);
  row.append(text, control);
  return row;
}

// The controls of a row, `name` their setting's, found again by `key`. What
// is the vault's waits until it is read (`busy`): until then it isn't known
// which of the ids kept are its own.
function settingSeg(name, key, options, current, onPick) {
  const seg = keyedSeg(accSeg(options, current, onPick), options, key);
  seg.setAttribute('role', 'group');
  seg.setAttribute('aria-label', name);
  return seg;
}

function settingBox(name, key, label, hint, open, busy = false) {
  const box = dropBox(accEl('button'), label, hint);
  box.disabled = busy;
  box.dataset.focusKey = key;
  box.setAttribute('aria-label', `${name}: ${label}`);
  box.addEventListener('click', ev => {
    ev.stopPropagation();
    open(box);
  });
  return box;
}

// ----------------------------------------------------------------------------
// Look and language

function lookPart(language) {
  return {
    id: 'look', title: t('settings.look'), scope: t('settings.every'), now: `${languageName(language.run)} \u00b7 ${shownTheme()}`,
    rows: [languageRow(language), themeRow()],
  };
}

// The language and its form of address, both from the next start; what is
// picked for it stands in the place of what the row does.
function languageRow({ picked, formal, run, run_formal, system, all }) {
  const name = t('settings.language');
  const id = picked ?? system;
  const { forms } = all.find(language => language.id === id);
  const form = forms[Number(formal)];
  const note = [t('settings.language_note')];
  if (id !== run || (form && formal !== run_formal)) {
    const language = form ? t('settings.in_form', { language: languageName(id), form: form.name }) : languageName(id);
    note.splice(0, 1, accEl('span', 'setting__next', t('settings.next', { language })));
    if (form) note.push(` ${form.says}`);
  }
  const controls = [];
  if (form) {
    const options = forms.map((one, i) => [Boolean(i), one.name, { title: one.says }]);
    controls.push(settingSeg(t('settings.address'), 'address', options, formal, value => setLanguage({ formal: value })));
  }
  const items = [
    { label: t('settings.system'), hint: languageName(system), checked: !picked, run: () => setLanguage({ picked: null }) },
    ...all.map((language, i) => ({
      label: languageName(language.id), desc: language.id === run ? '' : nameIn(locale, language.id), separator: i === 0,
      checked: picked === language.id, run: () => setLanguage({ picked: language.id }),
    })),
  ];
  controls.push(settingBox(name, 'language', picked ? languageName(id) : t('settings.system'), picked ? '' : languageName(id),
    box => showMenu(box, items, { matchWidth: true })));
  return settingRow(name, note, ...controls);
}

// A language by its own name, which some write small, and in another's words.
function languageName(id) {
  const name = nameIn(id, id);
  return name[0].toLocaleUpperCase(id) + name.slice(1);
}

function nameIn(other, id) {
  return new Intl.DisplayNames([other], { type: 'language' }).of(id);
}

// The theme, at once, the window's title bar with it (`set_theme`). What is
// picked is the root's `data-theme` (css/tokens.css), none for the system's.
const THEMES = [['system', t('settings.system')], ['light', t('settings.light')], ['dark', t('settings.dark')]];
const THEME_NOTES = { light: t('settings.theme_light'), dark: t('settings.theme_dark') };
const root = document.documentElement;
const systemDark = matchMedia('(prefers-color-scheme: dark)');

const pickedTheme = () => root.dataset.theme ?? 'system';

// The theme shown, in a sentence: "dark".
function shownTheme() {
  const dark = root.dataset.theme ? root.dataset.theme === 'dark' : systemDark.matches;
  return (dark ? t('settings.dark') : t('settings.light')).toLocaleLowerCase(locale);
}

function themeRow() {
  const name = t('settings.theme');
  const picked = pickedTheme();
  const note = THEME_NOTES[picked] ?? t('settings.theme_system', { theme: shownTheme() });
  return settingRow(name, [note], settingSeg(name, 'theme', THEMES, picked, setTheme));
}

function setTheme(theme) {
  rpcCall('set_theme', { theme }).then(() => {
    if (theme === 'system') delete root.dataset.theme;
    else root.dataset.theme = theme;
    scheduleRenderSettings();
  }).catch(e => showError(e.message));
}

systemDark.addEventListener('change', () => scheduleRenderSettings());

function setLanguage(change) {
  const language = { ...store.getState().language, ...change };
  rpcCall('set_language', { language: language.picked, formal: language.formal })
    .then(() => store.setState({ language }))
    .catch(e => showError(e.message));
}

// ----------------------------------------------------------------------------
// The user in the vault

function youPart() {
  const open = vaultOpen();
  return {
    id: 'you', title: open ? t('settings.you', { vault: store.getState().vault_name }) : t('settings.you_none'),
    scope: open ? t('settings.this_vault') : '', now: entities.get(me())?.title ?? '',
    rows: open ? [meRow(), calendarRow()] : [noVaultRow()],
  };
}

const indexing = () => store.getState().vault_state === 'indexing';

// Who the user is: a contact of the vault, or a new one by its name.
function meRow() {
  const name = t('settings.who');
  const id = me();
  const label = id ? entities.get(id).title || t('untitled') : t('facts.nobody');
  return settingRow(name, [t('settings.who_note')], settingBox(name, 'who', label, '',
    box => contactMenu(box, id && `@${id}`, ref => setMe(ref?.slice(1) ?? null), name, { make: makeMe }), indexing()));
}

function setMe(contact) {
  rpcCall('set_me', { contact }).then(me => store.setState({ me })).catch(e => showError(e.message));
}

function makeMe(title) {
  rpcCall('create_entity', { title, roles: { contact: { type: 'person' } } })
    .then(({ id }) => setMe(id))
    .catch(e => showError(e.message));
}

// The calendar a new event starts with (`openDraft`, model/drafts.js).
function calendarRow() {
  const name = t('settings.calendar');
  const id = newEventCalendar();
  const label = id ? entities.get(id).title || t('untitled') : t('watch.none');
  return settingRow(name, [t('settings.calendar_note')], settingBox(name, 'calendar', label, '',
    box => calendarMenu(box, id, pickNewEventCalendar, name, { writable: true }), indexing()));
}

function noVaultRow() {
  const open = accEl('button', 'btn', t('vault.open_dots'));
  open.type = 'button';
  open.dataset.focusKey = 'open-vault';
  open.addEventListener('click', () => runAction('vault.switch'));
  return settingRow(t('settings.who'), [t('settings.per_vault')], open);
}

// ----------------------------------------------------------------------------
// What Avelor keeps on this PC outside the vaults (the host's kept.rs)

let kept = null; // the answer to `kept`, asked as the page opens
let removing = false; // whether what a removal does is shown, to confirm it

function countKept() {
  rpcCall('kept', {}, { channel: 'kept' }).then(answer => {
    kept = answer;
    scheduleRenderSettings();
  }).catch(e => {
    if (e.code !== 'superseded') showError(e.message);
  });
}

function pcPart() {
  const size = kept ? fileSize(kept.settings + kept.secrets + kept.catalog + kept.web + kept.log) : '';
  return {
    id: 'pc', title: t('settings.pc'), scope: kept ? t('settings.outside', { size }) : '', now: size,
    rows: [...(kept ? [keptEl()] : []), removeRow()],
  };
}

function keptEl() {
  const el = accEl('div', 'kept');
  const folder = accEl('div', 'kept__folder');
  const show = accEl('button', 'btn', t('sidebar.show_in_folder'));
  show.type = 'button';
  show.dataset.focusKey = 'show-data';
  show.addEventListener('click', () => rpcCall('reveal_data').catch(e => showError(e.message)));
  folder.append(accEl('span', 'mono truncate-start', kept.folder), show);
  el.append(folder);
  for (const [name, note, bytes] of [
    [t('settings.name'), t('settings.kept.settings'), kept.settings],
    [t('settings.kept.secrets'), secretsText(), kept.secrets],
    [t('settings.kept.index'), t('settings.kept.index_note'), kept.catalog],
    [t('settings.kept.window'), t('settings.kept.window_note'), kept.web],
    [t('settings.kept.log'), t('settings.kept.log_note'), kept.log],
  ]) {
    const row = accEl('div', 'list-row kept__row');
    row.append(accEl('span', 'row-title', name), accEl('span', 'note', note), accEl('span', 'meta', fileSize(bytes)));
    el.append(row);
  }
  return el;
}

// "3 Google sign-ins, 4 AI keys; only your account on this PC can read them".
function secretsText() {
  const { google, ai, client } = kept;
  const what = [
    ...(google ? [t('settings.kept.google', { n: google })] : []),
    ...(ai ? [t('settings.kept.ai', { n: ai })] : []),
    ...(client ? [t('settings.kept.client')] : []),
  ];
  return what.length ? t('settings.kept.only_you', { what: what.join(', ') }) : t('settings.kept.no_secrets');
}

// The removal: its first click says what goes and what stays, its second
// does it.
function removeRow() {
  const remove = accEl('button', 'btn btn--danger-text', t('settings.remove_dots'));
  remove.type = 'button';
  remove.dataset.focusKey = 'remove';
  remove.addEventListener('click', () => showRemoval(true));
  const row = settingRow(t('settings.remove'), [t('settings.remove_note')], ...(removing ? [] : [remove]));
  if (removing) row.append(removalEl());
  return row;
}

function removalEl() {
  const el = accEl('div', 'removal prose');
  el.setAttribute('role', 'alertdialog');
  el.setAttribute('aria-label', t('settings.remove'));
  const list = accEl('ul');
  for (const text of [
    t('settings.gone.vaults'),
    ...(!kept || kept.google || kept.ai ? [t('settings.gone.secrets')] : []),
    ...(!kept || kept.google ? [t('settings.gone.google')] : []),
    t('settings.gone.start'),
  ]) list.append(accEl('li', '', text));
  const go = accEl('button', 'btn btn--danger', t('settings.remove_quit'));
  go.type = 'button';
  go.dataset.focusKey = 'remove-quit';
  go.addEventListener('click', () => rpcCall('remove_data').catch(e => showError(e.message)));
  const keep = accEl('button', 'btn', t('field.cancel'));
  keep.type = 'button';
  keep.dataset.focusKey = 'keep';
  keep.addEventListener('click', () => showRemoval(false));
  const foot = accEl('div', 'removal__foot');
  foot.append(go, keep);
  el.append(accEl('p', '', t('settings.gone')), list, foot);
  return el;
}

function showRemoval(on) {
  removing = on;
  setPageFocus(on ? 'keep' : 'remove');
  renderSettings();
}

// ----------------------------------------------------------------------------
// Open the page, the focus on the control `key`, if given.
function openSettings(key) {
  removing = false;
  if (key) setPageFocus(key);
  openVaultOp('settings');
  scheduleRenderSettings();
  countKept();
}

// Esc takes a removal back first.
registerVaultPage('settings', { close: () => (removing ? showRemoval(false) : closeVaultArea()), ofVault: false });
store.subscribe([
  state => state.vault_op, state => state.language, state => state.vault_state, state => state.vault_name,
  state => state.entities_rev, state => state.prefs, state => state.me,
], () => scheduleRenderSettings());

registerAction({
  id: 'settings', group: 'go', title: t('settings.name'), icon: '#i-settings', also: t('settings.also'),
  shortcut: SHORTCUT, page: true, place: 'page', end: () => t('palette.page'), run: () => openSettings(),
});
// The rows as commands: the title, the other words, the control focused, and
// when there is one.
for (const [id, title, also, control, when] of [
  ['language', t('settings.language'), t('settings.language_also'), () => 'language'],
  ['theme', t('settings.theme'), t('settings.theme_also'), () => `theme:${pickedTheme()}`],
  ['who', t('settings.who'), t('settings.who_also'), () => 'who', vaultOpen],
  ['calendar', t('settings.calendar'), t('settings.calendar_also'), () => 'calendar', vaultOpen],
  ['remove', t('settings.remove'), t('settings.remove_also'), () => 'remove'],
]) {
  registerAction({
    id: `settings.${id}`, group: 'settings', title, icon: '#i-settings', also, page: true, when,
    run: () => openSettings(control()),
  });
}

const helpItem = $('help-item');
helpItem.addEventListener('click', () => {
  showMenu(helpItem, [
    { label: t('palette.shortcuts'), run: () => runAction('keys') },
    { label: t('settings.name'), hint: shortcutText(SHORTCUT), run: () => openSettings() },
  ], { matchWidth: true });
});
store.subscribe([state => state.vault_op, state => state.vault_expanded], ([vault_op, vault_expanded]) => {
  setAria(helpItem, 'aria-current', vault_expanded && vault_op === 'settings');
});
