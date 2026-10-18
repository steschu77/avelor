// Workspace -> Accounts (accounts.md): the open vault's accounts, rendered
// into the expanded vault area while its op is "accounts" -- the Google
// accounts and their connect, then the AI providers: the AI provider menu,
// an AI account's card and pane, and the pane of a new one until its key or
// address checks out. A required field (the key, Ollama's address) stays an
// input until the host has checked it; then it shows as text with a pencil.
// A model picker's way here is the `accounts.open` action
// (`openAccountsPage`). The Email row's auto-sync is Fetch email's
// (`renderEmailSync`). The sidebar follows `accounts_rev` (app.js).

import { accEl, oncePerFrame } from '../../core/dom.js';
import { titled } from '../../core/dom.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import { pageDate } from '../../core/format.js';
import { registerAction } from '../../core/actions.js';
import { locale, t } from '../../core/text.js';
import { store } from '../../model/state.js';
import { accounts } from '../../model/entities.js';
import { vaultSettings } from '../../model/entities.js';
import { entities } from '../../model/entities.js';
import { showMenu } from '../../ui/menu.js';
import { armedButton } from '../../ui/confirm.js';
import {
  addTile, detailFooter, detailHead, detailPane, draftKind, failedRetry, itemCard, listDetailPage,
  listSection, notice, pageList, replaceVaultPage, sentDrafts, setPageFocus, stat, stats,
} from '../../ui/page.js';
import { openVaultOp } from '../../ui/page.js';
import { fieldEditKey, fieldEdits, renderEditField } from '../../ui/field.js';
import { accSeg } from '../../ui/segmented.js';
import { AI_PROVIDERS, aiHost, aiProvider, aiTitle, aiUsd } from './models.js';
import { aiLimitReached } from './models.js';
import { isAiAccount } from './models.js';
import { renderEmailSync } from '../import/email.js';
import { kindName } from '../../ui/icons.js';

// The access rows shown; `tasks` stays in the account (and in every
// reconnect request) without a row.
const ACCOUNT_SERVICES = [
  { key: 'email', name: t('kind.email'), product: 'Gmail' },
  { key: 'calendar', name: t('kind.calendar'), product: 'Google Calendar' },
  { key: 'contacts', name: t('types.contacts'), product: 'Google Contacts' },
];
const ACCOUNT_LEVELS = ['off', 'read', 'full'];
const ACCOUNT_LEVEL_NAMES = { off: t('access.off'), read: t('access.read'), full: t('access.full') };
const NEW_ACCOUNT_LEVELS = { email: 'read', calendar: 'read', contacts: 'off', tasks: 'off' };

let selectedAccount = 'new'; // an account id, 'new' for the Google connect pane, or 'new:<kind>' for a new AI account
let pendingConnect = null; // { account } while the browser sign-in runs
let accountError = null; // { account, message } from the last failed op
const accountDrafts = new Map(); // account id | 'new' -> edited levels
// account id -> 'loading' | { error } | [{ id, name, color, primary }] (list_calendars)
const accountCalendars = new Map();
const calendarToggles = new Map(); // `${account}\n${calendar}` -> checked, until the host confirms
// account id -> its status and calendar scope as last seen: every account
// the page knows
const accountAccess = new Map();

// ----------------------------------------------------------------------------
// host events: the page's own state about the accounts, after the model
// wrote them (model/events.js)

// The opened vault's accounts replace the ones shown.
rpcOn('accounts', ({ items }) => {
  const ids = new Set(items.map(item => item.id));
  for (const id of [...accountAccess.keys()]) {
    if (!ids.has(id)) onAccountRemoved(id);
  }
  for (const item of items) onAccount(item);
});
rpcOn('account', meta => onAccount(meta));
rpcOn('account_removed', ({ id }) => onAccountRemoved(id));

function onAccount(meta) {
  const id = meta.id;
  // A (re)connect or disconnect can change which calendars are visible.
  const access = `${meta.status}\n${meta.scope?.calendar}`;
  if (accountAccess.get(id) !== access) accountCalendars.delete(id);
  if (selectedAccount === 'new' && pendingConnect === null && accountAccess.size === 0) {
    selectedAccount = id; // the first account
  }
  accountAccess.set(id, access);
  accountDrafts.delete(id); // what was granted replaces any edit
  for (const key of calendarToggles.keys()) {
    if (key.startsWith(`${id}\n`)) calendarToggles.delete(key); // confirmed
  }
}

function onAccountRemoved(id) {
  accountAccess.delete(id);
  aiAccountRemoved(id);
  accountDrafts.delete(id);
  accountCalendars.delete(id);
  if (selectedAccount === id) selectedAccount = accounts.keys().next().value ?? 'new';
}

// ----------------------------------------------------------------------------
// actions

// What to show for a failed call about `account` (an id, or 'new'); null
// for one that was cancelled or replaced.
function accountErrorText(e, account) {
  switch (e.code) {
    case 'cancelled':
    case 'superseded':
      return null;
    case 'timeout':
      return t('signin.timed_out');
    case 'denied':
      return t('signin.declined');
    case 'wrong_account':
      return accounts.get(account)?.email ? t('signin.other_account', { email: accounts.get(account).email })
        : t('signin.other_account_this');
    case 'not_connected':
      return t('accounts.not_connected');
    case 'not_found':
      return t('accounts.gone');
    case 'invalid_path':
      return t('accounts.open_vault_first');
    default:
      return e.message;
  }
}

// Connect a new Google account (`account` 'new') or reconnect one. A newer
// connect replaces this one, whose call then ends as cancelled.
function connectGoogle(account) {
  const connect = { account };
  pendingConnect = connect;
  accountError = null;
  const levels = accountLevels(account);
  const args = account === 'new' ? { levels } : { account, levels };
  rpcCall('connect_google', args).then(({ id }) => {
    if (pendingConnect === connect) pendingConnect = null;
    accountDrafts.delete('new');
    selectedAccount = id;
    scheduleRenderAccounts();
  }, e => {
    if (pendingConnect === connect) pendingConnect = null;
    const message = accountErrorText(e, account);
    if (message) accountError = { account, message };
    scheduleRenderAccounts();
  });
  scheduleRenderAccounts();
}

function cancelConnect() {
  if (pendingConnect) rpcCall('cancel_connect');
}

// `disconnect_account` or `remove_account`, on the second click of its
// button (`armedButton`).
function accountOp(cmd, id) {
  const settled = e => {
    const message = e && accountErrorText(e, id);
    accountError = message ? { account: id, message } : null;
    scheduleRenderAccounts();
  };
  rpcCall(cmd, { account: id }).then(() => settled(null), settled);
}

const ACCOUNT_STATUS_LABELS = {
  connected: t('accounts.status.connected'),
  disconnected: t('accounts.status.disconnected'),
  reconnect: t('accounts.status.reconnect'),
};

// Calendar ops report on the calendar list, not on the pane.
function requestCalendars(account) {
  accountCalendars.set(account, 'loading');
  rpcCall('list_calendars', { account }).then(({ calendars }) => {
    accountCalendars.set(account, calendars);
    scheduleRenderAccounts();
  }, e => {
    const message = accountErrorText(e, account) ?? t('accounts.calendars_cancelled');
    accountCalendars.set(account, { error: t('accounts.calendars_failed', { error: message }) });
    scheduleRenderAccounts();
  });
}

// The host confirms with the account (`onAccount` clears the toggle), then
// answers.
function setCalendarSource(account, calendar, checked) {
  const toggle = `${account}\n${calendar.id}`;
  calendarToggles.set(toggle, checked);
  const source = { id: calendar.id, name: calendar.name, color: calendar.color ?? null };
  const args = { account, calendar: source, selected: checked };
  rpcCall('set_calendar_source', args).catch(e => {
    calendarToggles.delete(toggle);
    const message = accountErrorText(e, account);
    if (message) accountError = { account, message };
    scheduleRenderAccounts();
  });
  scheduleRenderAccounts();
}

// Switch a checked calendar's auto-sync; the pane follows the account.
function setCalendarSync(account, calendar, enabled) {
  rpcCall('set_calendar_sync', { account, calendar, enabled }).catch(e => {
    const message = accountErrorText(e, account);
    if (message) accountError = { account, message };
    scheduleRenderAccounts();
  });
}

// The levels a (re)connect asks for: the edited ones, else what is granted.
function accountLevels(account) {
  return accountDrafts.get(account)
    ?? (account === 'new' ? NEW_ACCOUNT_LEVELS : accounts.get(account)?.scope)
    ?? NEW_ACCOUNT_LEVELS;
}

function setAccountLevel(account, service, level) {
  accountDrafts.set(account, { ...accountLevels(account), [service]: level });
  scheduleRenderAccounts();
}

// ----------------------------------------------------------------------------
// rendering

const scheduleRenderAccounts = oncePerFrame(renderAccounts);

// "Email read \u00b7 Calendar full", or "No access".
function accountAccessSummary(scope) {
  const parts = ACCOUNT_SERVICES
    .filter(({ key }) => scope?.[key] && scope[key] !== 'off')
    .map(({ key, name }) => t('access.summary', { service: name, level: ACCOUNT_LEVEL_NAMES[scope[key]] ?? scope[key] }));
  return parts.length ? parts.join(' \u00b7 ') : t('access.none');
}

function renderAccounts() {
  if (store.getState().vault_op !== 'accounts') return;
  replaceVaultPage(listDetailPage(renderAccountList(), renderAccountDetail()));
}

// The selected account's pane, a new AI account's, or the Google connect;
// nothing without a vault to keep an account in.
function renderAccountDetail() {
  if (!store.getState().vault_path) return detailPane();
  const kind = draftKind(selectedAccount);
  if (kind) return renderAiPane(null, kind);
  const account = accounts.get(selectedAccount);
  if (!account) return store.getState().google ? renderConnectPane() : detailPane();
  return isAiAccount(account) ? renderAiPane(account) : renderAccountPane(account);
}

function renderAccountList() {
  const list = pageList(t('workspace.accounts'), t('accounts.intro'));
  if (!store.getState().vault_path) {
    list.append(accEl('div', 'page-text', t('accounts.no_vault')));
    return list;
  }

  const google = [...accounts.values()].filter(account => !isAiAccount(account));
  if (google.length) list.append(listSection(t('sidebar.sources')));
  for (const account of google) {
    list.append(itemCard({
      kind: 'Google',
      status: ACCOUNT_STATUS_LABELS[account.status] ?? account.status,
      title: account.email ?? t('accounts.unknown_account'),
      sub: accountAccessSummary(account.scope),
      current: account.id === selectedAccount,
      onSelect: () => {
        selectedAccount = account.id;
        scheduleRenderAccounts();
      },
    }));
  }

  const ai = [...accounts.values()].filter(isAiAccount);
  if (ai.length) list.append(listSection(t('accounts.ai_providers')));
  for (const account of ai) list.append(renderAiCard(account));

  list.append(listSection(t('accounts.connect')));
  if (store.getState().google) {
    list.append(addTile({
      label: t('accounts.google_account'),
      hint: t('accounts.sign_in_hint'),
      current: selectedAccount === 'new',
      onClick: () => {
        selectedAccount = 'new';
        scheduleRenderAccounts();
      },
    }));
  }
  list.append(renderAiConnect());
  return list;
}

// Off / Read / Full per service; `account` is an id or 'new'.
function renderAccessRows(account) {
  const levels = accountLevels(account);
  const rows = accEl('div', 'accounts__access');
  for (const { key, name, product } of ACCOUNT_SERVICES) {
    const row = accEl('div', 'accounts__access-row');
    const label = accEl('div', 'accounts__service');
    label.append(accEl('span', 'subtitle', name), accEl('span', 'label', product));
    const seg = accSeg(ACCOUNT_LEVELS.map(level => [level, ACCOUNT_LEVEL_NAMES[level]]), levels[key] ?? 'off',
      level => setAccountLevel(account, key, level), pendingConnect !== null);
    row.append(label, seg);
    rows.append(row);

    // A connected account's email can sync on its own
    // (features/import/email.js), and its calendars can each become a
    // source.
    const granted = accounts.get(account);
    const connected = granted?.status === 'connected';
    if (key === 'email' && connected && (granted.scope?.email ?? 'off') !== 'off') {
      rows.append(renderEmailSync(granted, scheduleRenderAccounts));
    }
    if (key === 'calendar' && connected && granted.scope?.calendar !== 'off') {
      rows.append(renderCalendarSources(account));
    }
  }
  return rows;
}

// Checkboxes for the account's calendars: checked ones show as sources in
// the sidebar, each with its auto-sync. Loaded once per account (and again
// after a reconnect).
function renderCalendarSources(account) {
  const list = accEl('div', 'accounts__sub');
  list.append(accEl('span', 'label', t('accounts.calendars_shown')));
  let calendars = accountCalendars.get(account);
  if (calendars === undefined) {
    requestCalendars(account);
    calendars = 'loading';
  }
  if (calendars === 'loading') {
    list.append(accEl('span', 'page-text', t('accounts.loading_calendars')));
    return list;
  }
  if (calendars.error) {
    list.append(failedRetry(calendars.error, () => requestCalendars(account)));
    return list;
  }

  // Checked calendars Google no longer lists stay, so they can be unchecked.
  const checked = accounts.get(account)?.calendars ?? [];
  const missing = checked.filter(c => !calendars.some(listed => listed.id === c.id));
  for (const calendar of [...calendars, ...missing]) {
    const toggle = `${account}\n${calendar.id}`;
    const source = checked.find(c => c.id === calendar.id);
    const row = accEl('div', 'accounts__calendar');
    const pick = accEl('label', 'accounts__pick');
    const box = accEl('input');
    box.type = 'checkbox';
    box.checked = calendarToggles.get(toggle) ?? !!source;
    box.disabled = calendarToggles.has(toggle);
    box.addEventListener('change', () => setCalendarSource(account, calendar, box.checked));
    const swatch = accEl('span', 'accounts__swatch');
    if (calendar.color) swatch.style.background = calendar.color;
    pick.append(box, swatch, accEl('span', '', calendar.name));
    if (calendar.primary) pick.append(accEl('span', 'label', t('accounts.primary')));
    if (missing.includes(calendar)) pick.append(accEl('span', 'label', t('accounts.not_on_google')));
    row.append(pick);
    if (source && !calendarToggles.has(toggle)) row.append(renderCalendarSync(account, source));
    list.append(row);
  }
  if (!calendars.length && !missing.length) {
    list.append(accEl('span', 'page-text', t('accounts.no_calendars')));
  }
  return list;
}

// A checked calendar's auto-sync: a check that has the host sync it every
// 15 minutes (`set_calendar_sync`).
function renderCalendarSync(account, source) {
  const on = !!source.auto_sync;
  const auto = accEl('label', 'accounts__auto');
  const check = accEl('button', 'check', on ? '\u2713' : '');
  check.type = 'button';
  check.setAttribute('aria-pressed', String(on));
  titled(check, on ? t('accounts.stop_sync') : t('accounts.sync_every'));
  check.addEventListener('click', () => setCalendarSync(account, source.id, !on));
  auto.append(check, accEl('span', 'label', t('accounts.auto_sync')));
  return auto;
}

// "Continue in your browser..." with Cancel, or the last error, if any.
function renderConnectState(account) {
  if (pendingConnect?.account === account) {
    const cancel = accEl('button', 'btn', t('field.cancel'));
    cancel.type = 'button';
    cancel.addEventListener('click', cancelConnect);
    return notice('info', accEl('span', '', t('accounts.continue_in_browser')), cancel);
  }
  if (accountError && accountError.account === account) return notice('error', accountError.message);
  return null;
}

function renderConnectPane() {
  const pane = detailPane();
  pane.append(detailHead(t('accounts.connect_google_head')),
    accEl('div', 'pane-title', t('accounts.google_account')),
    accEl('div', 'page-text', t('accounts.connect_google_text')),
    renderAccessRows('new'));

  const actions = accEl('div', 'accounts__actions');
  const signIn = accEl('button', 'btn btn--primary', t('accounts.sign_in_google'));
  signIn.type = 'button';
  signIn.disabled = pendingConnect !== null;
  signIn.addEventListener('click', () => connectGoogle('new'));
  actions.append(signIn);
  pane.append(actions);

  const state = renderConnectState('new');
  if (state) pane.append(state);
  return pane;
}

// Entities that arrived from `id`, by kind: [total, "12 email \u00b7 3 event"].
function accountEntityCounts(id) {
  const byKind = new Map();
  for (const doc of entities.values()) {
    if (doc.source?.remote?.account !== id) continue;
    byKind.set(doc.kind, (byKind.get(doc.kind) ?? 0) + 1);
  }
  const total = [...byKind.values()].reduce((a, b) => a + b, 0);
  const detail = [...byKind].map(([kind, n]) => t('count.of_kind', { kind: kindName(kind), n })).join(' \u00b7 ');
  return [total, detail];
}

function renderAccountPane(account) {
  const pane = detailPane();
  const connected = account.status === 'connected';

  const reconnect = accEl('button', 'btn', connected ? t('accounts.reconnect') : t('accounts.connect'));
  reconnect.type = 'button';
  reconnect.disabled = pendingConnect !== null;
  reconnect.addEventListener('click', () => connectGoogle(account.id));
  const heading = connected
    ? t('accounts.head_connected', { date: pageDate(account.connected) })
    : t('accounts.head_status', { status: ACCOUNT_STATUS_LABELS[account.status] ?? account.status });
  pane.append(detailHead(heading, { button: reconnect }),
    accEl('div', 'pane-title', account.email ?? t('accounts.unknown_account')));

  const [total, detail] = accountEntityCounts(account.id);
  pane.append(stats(
    stat(t('accounts.last_sync'), pageDate(account.last_sync, true)),
    stat(t('accounts.connected_since'), account.connected ? pageDate(account.connected) : t('accounts.unknown')),
    stat(t('accounts.in_vault'), t('count.entities', { n: total }), detail),
    stat(t('accounts.account_id'), `${account.id.slice(0, 8)}\u2026${account.id.slice(-4)}`, t('accounts.in_remote_blocks')),
  ), renderAccessRows(account.id));

  // An edit only takes effect with the next consent.
  const draft = accountDrafts.get(account.id);
  if (connected && draft && ACCOUNT_SERVICES.some(({ key }) => (draft[key] ?? 'off') !== (account.scope?.[key] ?? 'off'))) {
    pane.append(notice('info', t('accounts.reconnect_to_apply')));
  }
  if (account.status === 'disconnected') {
    pane.append(notice('info', t('accounts.disconnected_note')));
  } else if (account.status === 'reconnect') {
    pane.append(notice('error', t('accounts.sign_in_missing')));
  }
  const state = renderConnectState(account.id);
  if (state) pane.append(state);

  // Connected accounts disconnect; the others can be removed for good.
  const [text, op, label, armed] = connected
    ? [t('accounts.disconnect_google_note'), 'disconnect_account', t('accounts.disconnect_dots'), t('accounts.disconnect_again')]
    : [t('accounts.remove_google_note'), 'remove_account', t('accounts.remove_dots'), t('accounts.remove_again')];
  const button = accEl('button', 'btn btn--ghost', label);
  button.type = 'button';
  button.disabled = pendingConnect !== null;
  armedButton(button, armed, () => accountOp(op, account.id),
    `${op} ${account.id}`);
  pane.append(detailFooter(text, button));
  return pane;
}

// Re-render while visible when the op, the vault, the accounts, the
// entities, or what picked a model (the vault's settings:
// "Used by") change.
store.subscribe([
  state => state.vault_op, state => state.vault_path, state => state.accounts_rev,
  state => state.entities_rev, state => state.settings_rev,
], () => scheduleRenderAccounts());

// ----------------------------------------------------------------------------
// AI providers (accounts.md#accounts-page)

const AI_STATUS_LABELS = {
  connected: t('accounts.status.connected'),
  disconnected: t('accounts.status.disconnected'),
  reconnect: t('accounts.status.needs_key'),
  running: t('accounts.status.running'),
  'not running': t('accounts.status.not_running'),
  unknown: '',
};

// account id -> 'loading' | { error } | [{ id, name, context_window, price }]
const aiModels = new Map();
// account id -> { offered?, spend_limit? } sent, until done
const aiSettingsDrafts = sentDrafts();
// account id -> 'testing' | { ok, message } of the last Test
const aiTests = new Map();
// the running connect_ai: { account, field }, or null
let aiConnecting = null;

// ----------------------------------------------------------------------------
// host events (dispatched above)

function aiAccountRemoved(id) {
  aiModels.delete(id);
  aiSettingsDrafts.forget(id);
  aiTests.delete(id);
  for (const key of fieldEdits.keys()) {
    if (key.startsWith(`${id}\n`)) fieldEdits.delete(key);
  }
}

// What to show for a failed AI account call; null for one that was
// cancelled or replaced. The host words what went wrong with a key, an
// address or a provider (`message`).
function aiErrorText(e, account) {
  switch (e.code) {
    case 'cancelled':
    case 'superseded':
      return null;
    case 'not_found':
    case 'invalid_path':
      return accountErrorText(e, account);
    default:
      return e.message;
  }
}

// What picked `model` of the AI account `account`: the names the Accounts
// page shows in its "Used by" column.
function aiUsersOf(account, model) {
  const is = choice => choice?.account === account && choice?.model === model;
  const users = [];
  if (is(vaultSettings().chat_model)) users.push(t('types.conversations'));
  return users;
}

// Open Workspace -> Accounts on `account`'s pane, or on the page as it is:
// the `accounts.open` action, a model picker's way here
// (features/workspace/models.js).
function openAccountsPage(account) {
  if (account) {
    selectedAccount = account;
  }
  openVaultOp('accounts');
  scheduleRenderAccounts();
}
registerAction({ id: 'accounts.open', title: t('accounts.open'), run: openAccountsPage });

// ----------------------------------------------------------------------------
// actions

// Show a new account's pane, its required field ready for input.
function aiStartDraft(kind) {
  const draft = `new:${kind}`;
  for (const field of ['key', 'address']) fieldEdits.delete(fieldEditKey(draft, field));
  selectedAccount = draft;
  setPageFocus(fieldEditKey(draft, aiProvider(kind).local ? 'address' : 'key'));
  scheduleRenderAccounts();
}

// Check and store a key or address (`field`) of `account` (an id, or
// 'new:<kind>' for a new account). A newer check supersedes a running one.
function aiConnect(account, field, value) {
  if (aiConnecting) {
    const running = fieldEdits.get(fieldEditKey(aiConnecting.account, aiConnecting.field));
    if (running) running.checking = false;
  }
  const connect = { account, field };
  aiConnecting = connect;
  const kind = draftKind(account);
  const key = fieldEditKey(account, field);
  const state = fieldEdits.get(key) ?? { value };
  fieldEdits.set(key, { ...state, value, checking: true, error: null });
  const args = kind ? { provider: kind, [field]: value } : { account, [field]: value };
  rpcCall('connect_ai', args, { channel: 'connect_ai' }).then(({ id, models }) => {
    if (aiConnecting === connect) aiConnecting = null;
    aiModels.set(id, models);
    fieldEdits.delete(key);
    if (kind) {
      for (const other of ['key', 'address']) fieldEdits.delete(fieldEditKey(account, other));
      if (selectedAccount === account) selectedAccount = id;
    }
    scheduleRenderAccounts();
  }, e => {
    if (e.code === 'superseded') return; // the newer check owns the field
    if (aiConnecting === connect) aiConnecting = null;
    const failed = fieldEdits.get(key);
    if (failed) {
      failed.checking = false;
      failed.error = aiErrorText(e, account);
      setPageFocus(key);
    }
    scheduleRenderAccounts();
  });
  scheduleRenderAccounts();
}

// Ask for the models: for the list (`models`), or for Test (`test`). For a
// render, which shows the loading state itself.
function aiRequestModels(account, op = 'models') {
  if (op === 'test') aiTests.set(account, 'testing');
  else aiModels.set(account, 'loading');
  rpcCall('list_ai_models', { account }).then(({ models }) => {
    aiModels.set(account, models);
    if (op === 'test') {
      const count = models.length;
      aiTests.set(account, { ok: true, message: t('accounts.works', { n: count }) });
    }
    scheduleRenderAccounts();
  }, e => {
    const message = aiErrorText(e, account) ?? t('accounts.models_cancelled');
    if (op === 'test') aiTests.set(account, { ok: false, message });
    else aiModels.set(account, { error: message });
    scheduleRenderAccounts();
  });
}

// List the models again (Refresh, Retry, Test).
function aiListModels(account, op = 'models') {
  aiRequestModels(account, op);
  scheduleRenderAccounts();
}

// The draft shows what was sent until the last call for the account has
// settled (the `account` event comes before each answer).
function aiSetSettings(account, change) {
  accountError = null;
  const settled = e => {
    const message = e && aiErrorText(e, account);
    accountError = message ? { account, message } : null;
    scheduleRenderAccounts();
  };
  aiSettingsDrafts.send(account, change, () => rpcCall('set_ai_settings', { account, ...change }))
    .then(() => settled(null), settled);
  scheduleRenderAccounts();
}

// The account's settings, with what was sent but isn't confirmed yet.
function aiSettings(account) {
  const saved = accounts.get(account) ?? {};
  const draft = aiSettingsDrafts.get(account) ?? {};
  return {
    offered: draft.offered ?? saved.offered ?? [],
    spend_limit: 'spend_limit' in draft ? draft.spend_limit : saved.spend_limit ?? null,
  };
}

// Tick or untick a model: ticked, the features' pickers offer it. A
// feature that picked an unticked model is refused until it picks another.
function aiToggleOffered(account, modelId, on) {
  const offered = aiSettings(account).offered.filter(m => m !== modelId);
  if (on) offered.push(modelId);
  aiSetSettings(account, { offered });
}

// ----------------------------------------------------------------------------
// formatting

// What was spent: "$3.18".
function aiCost(n) {
  return new Intl.NumberFormat(locale, { style: 'currency', currency: 'USD' }).format(n ?? 0);
}

function aiPrice(price) {
  if (!price) return '\u2014';
  if (price.input === 0 && price.output === 0) return t('accounts.free');
  return `${aiUsd(price.input)} / ${aiUsd(price.output)}`;
}

// "12 calls this month, 2 not priced": what "This month" counts; calls of a
// model the price table doesn't know cost nothing there.
function aiSpentNote(spent) {
  const calls = spent?.calls ?? 0;
  const note = t('accounts.calls_this_month', { n: calls });
  return spent?.unpriced ? t('accounts.not_priced', { note, n: spent.unpriced }) : note;
}

function aiOfferedText(count) {
  return t('accounts.offered_n', { n: count });
}

// ----------------------------------------------------------------------------
// the list: cards and the AI provider menu

function renderAiCard(account) {
  // A local server's status is checked as soon as its card shows.
  if (aiProvider(account.kind).local && !aiModels.has(account.id)) aiRequestModels(account.id);
  const provider = aiProvider(account.kind);
  const status = aiModels.get(account.id) === 'loading' && account.status === 'unknown'
    ? t('accounts.status.checking') : AI_STATUS_LABELS[account.status] ?? account.status;

  const offered = aiOfferedText(account.offered?.length ?? 0);
  const meta = provider.local
    ? `${aiHost(account.address)} \u00b7 ${offered}`
    : account.key
      ? `${account.key} \u00b7 ${offered} \u00b7 ${t('accounts.spent_this_month', { amount: aiCost(account.spent?.cost) })}`
      : `${t('accounts.no_key')} \u00b7 ${offered}`;
  return itemCard({
    kind: provider.name,
    status,
    title: aiTitle(account.kind, account),
    sub: meta,
    current: account.id === selectedAccount,
    onSelect: () => {
      selectedAccount = account.id;
      scheduleRenderAccounts();
    },
  });
}

// "AI provider": a menu of the providers; choosing one opens its new pane.
function renderAiConnect() {
  return addTile({
    label: t('accounts.ai_provider'),
    hint: '\u25be',
    current: !!draftKind(selectedAccount),
    onClick: tile => {
      const items = AI_PROVIDERS.map((provider, i) => ({
        label: provider.name,
        hint: provider.local ? t('models.local') : t('accounts.key_hint'),
        separator: provider.local && i > 0 && !AI_PROVIDERS[i - 1].local,
        run: () => aiStartDraft(provider.kind),
      }));
      showMenu(tile, items, { matchWidth: true });
    },
  });
}

// ----------------------------------------------------------------------------
// the pane

// How the key is kept (accounts.md#secrets): DPAPI on Windows, on Linux a
// file only the user can read.
const ON_WINDOWS = navigator.platform.startsWith('Win');
const KEY_KEPT = ON_WINDOWS ? t('accounts.kept_windows') : t('accounts.kept_file');

// The key of a key provider, checked before it is stored.
function renderAiKeyStat(id, kind, account) {
  const provider = aiProvider(kind);
  const [field, note] = renderEditField(id, 'key', {
    saved: account?.key ?? null,
    required: true,
    type: 'password',
    mono: true,
    placeholder: provider.placeholder,
    editTitle: t('accounts.replace_key'),
    onSave: key => aiConnect(id, 'key', key),
    rerender: scheduleRenderAccounts,
  });
  const hint = account?.status === 'reconnect'
    ? t('accounts.key_unreadable')
    : account?.status === 'disconnected'
      ? t('accounts.key_to_connect')
      : KEY_KEPT;
  return stat(t('accounts.key'), field, note ?? hint);
}

// Ollama's address, checked before it is stored.
function renderAiAddressStat(id, kind, account) {
  const provider = aiProvider(kind);
  const [field, note] = renderEditField(id, 'address', {
    saved: account ? aiHost(account.address) : null,
    required: true,
    mono: true,
    value: account?.address ?? provider.address,
    placeholder: provider.address,
    editTitle: t('accounts.change_address'),
    onSave: address => aiConnect(id, 'address', address),
    rerender: scheduleRenderAccounts,
  });
  return stat(t('accounts.address'), field, note ?? t('accounts.where_answers', { name: provider.name }));
}

function renderAiSpendStat(account) {
  const limit = aiSettings(account.id).spend_limit;
  const [field, note] = renderEditField(account.id, 'spend_limit', {
    saved: limit == null ? null : t('accounts.per_month', { amount: aiUsd(limit) }),
    empty: t('accounts.no_limit'),
    value: limit == null ? '' : String(limit),
    placeholder: t('accounts.limit_placeholder'),
    editTitle: t('accounts.change_limit'),
    onSave: value => {
      const amount = value === '' ? null : Number(value.replace(',', '.').replace(/^\$/, ''));
      if (amount !== null && !(amount > 0)) {
        const key = fieldEditKey(account.id, 'spend_limit');
        fieldEdits.set(key, { value, error: t('accounts.limit_invalid') });
        setPageFocus(key);
        scheduleRenderAccounts();
        return;
      }
      fieldEdits.delete(fieldEditKey(account.id, 'spend_limit'));
      aiSetSettings(account.id, { spend_limit: amount });
    },
    rerender: scheduleRenderAccounts,
  });
  const hint = aiLimitReached(account)
    ? t('accounts.limit_reached')
    : t('accounts.limit_stops');
  return stat(t('accounts.spend_limit'), field, note ?? hint);
}

function renderAiStats(id, kind, account) {
  const provider = aiProvider(kind);
  const accountId = account
    ? stat(t('accounts.account_id'), accEl('span', 'edit-field__text mono mono--small',
      `${account.id.slice(0, 8)}\u2026${account.id.slice(-4)}`), t('accounts.in_ai_blocks'))
    : stat(t('accounts.account_id'), '\u2014', t('accounts.given_on_connect'));

  if (!provider.local) {
    const spent = account?.spent;
    return stats(
      renderAiKeyStat(id, kind, account),
      account
        ? stat(t('accounts.this_month'), aiCost(spent?.cost), aiSpentNote(spent))
        : stat(t('accounts.this_month'), '\u2014'),
      account ? renderAiSpendStat(account) : stat(t('accounts.spend_limit'), '\u2014'),
      accountId);
  }

  const list = account ? aiModels.get(account.id) : undefined;
  let status = '\u2014';
  let statusNote = null;
  if (account) {
    if (list === 'loading' && account.status === 'unknown') status = t('accounts.checking');
    else if (account.status === 'running') status = t('accounts.running');
    else if (account.status === 'not running') status = t('accounts.not_running');
    if (account.status === 'running' && Array.isArray(list)) {
      statusNote = t('accounts.models_here', { n: list.length });
    } else if (account.status === 'not running') {
      statusNote = t('accounts.start_then_test', { name: provider.name });
    }
  }
  const offered = account ? aiSettings(account.id).offered.length : null;
  return stats(
    renderAiAddressStat(id, kind, account),
    stat(t('props.status'), status, statusNote),
    stat(t('accounts.offered'), offered == null ? '\u2014'
      : Array.isArray(list) ? t('count.of_total', { at: offered, n: list.length }) : String(offered), t('accounts.in_pickers')),
    accountId);
}

// The model list: tick to offer a model in the features' pickers; what
// picked it is shown, not chosen, here.
function renderAiModels(account) {
  const provider = aiProvider(account.kind);
  const section = accEl('div', 'ai-models');
  const head = accEl('div', 'ai-models__head');
  head.append(
    accEl('span', 'subtitle ai-models__title', provider.local ? t('accounts.models_on_computer') : t('accounts.models_on_key')),
    accEl('span', 'page-text', t('accounts.ticked_note')));
  const canList = provider.local || !!account.key;
  if (canList) {
    const refresh = accEl('button', 'btn btn--ghost', t('accounts.refresh_list'));
    refresh.type = 'button';
    refresh.disabled = aiModels.get(account.id) === 'loading';
    refresh.addEventListener('click', () => aiListModels(account.id));
    head.append(refresh);
  }
  section.append(head);

  if (!canList) {
    section.append(accEl('div', 'page-text', t('accounts.key_to_list')));
    return section;
  }
  let list = aiModels.get(account.id);
  if (list === undefined) {
    // Once per account and session, like the calendars.
    aiRequestModels(account.id);
    list = 'loading';
  }
  if (list === 'loading') {
    section.append(accEl('div', 'page-text', t('accounts.loading_models')));
    return section;
  }
  if (list.error) {
    section.append(failedRetry(list.error, () => aiListModels(account.id)));
    return section;
  }

  const settings = aiSettings(account.id);
  const byId = new Map(list.map(model => [model.id, model]));
  const ticked = settings.offered.map(id => byId.get(id) ?? { id, name: id, missing: true });
  const rest = list.filter(model => !settings.offered.includes(model.id));

  const columns = accEl('div', 'list-row ai-models__row ai-models__row--head label');
  columns.append(accEl('span'), accEl('span', '', t('accounts.model')), accEl('span', '', t('accounts.per_million')),
    accEl('span', '', t('accounts.used_by')));
  section.append(columns);
  for (const model of [...ticked, ...rest]) {
    section.append(renderAiModelRow(account, model, !rest.includes(model)));
  }
  if (!list.length && !ticked.length) {
    section.append(accEl('div', 'page-text', provider.local
      ? t('accounts.no_local_models')
      : t('accounts.no_key_models')));
  }
  return section;
}

function renderAiModelRow(account, model, ticked) {
  const row = accEl('div', 'list-row ai-models__row');
  if (!ticked) row.classList.add('ai-models__row--off');

  const check = accEl('button', 'check', ticked ? '\u2713' : '');
  check.type = 'button';
  check.setAttribute('aria-pressed', String(ticked));
  titled(check, ticked ? t('accounts.stop_offering') : t('accounts.offer'));
  check.addEventListener('click', () => aiToggleOffered(account.id, model.id, !ticked));

  const name = accEl('span', 'mono ai-models__id', model.id);
  if (model.name && model.name !== model.id) name.title = model.name;
  const price = accEl('span', 'mono mono--small ai-models__price', model.missing ? '\u2014' : aiPrice(model.price));

  // Who picked the model, as information; picked where it is used. An
  // unticked model still names them: they are refused until they pick
  // another.
  const uses = accEl('span', 'ai-models__uses');
  const users = aiUsersOf(account.id, model.id);
  for (const user of users) uses.append(accEl('span', 'chip', user));
  if (!ticked) uses.append(accEl('span', 'note', users.length ? t('accounts.not_offered') : t('accounts.hidden_from_pickers')));
  else if (!users.length) uses.append(accEl('span', 'note', '\u2014'));
  if (model.missing) uses.append(accEl('span', 'note', t('accounts.no_longer_listed')));
  row.append(check, name, price, uses);
  return row;
}

// An AI account's pane (`account`), or a new one's (`account` null, of
// provider `newKind`).
function renderAiPane(account, newKind) {
  const kind = account?.kind ?? newKind;
  const provider = aiProvider(kind);
  const id = account?.id ?? `new:${kind}`;
  const pane = detailPane();

  let label = t('accounts.connect_ai_head', { name: provider.name });
  if (account?.status === 'connected') {
    label = t('accounts.ai_head_connected', { name: provider.name, date: pageDate(account.connected) });
  } else if (account) {
    const status = AI_STATUS_LABELS[account.status] ?? account.status;
    label = status ? t('accounts.ai_head_status', { name: provider.name, status }) : t('accounts.ai_head', { name: provider.name });
  }
  let testButton = null;
  if (account && (provider.local || account.key)) {
    testButton = accEl('button', 'btn', aiTests.get(id) === 'testing' ? t('accounts.testing') : t('accounts.test'));
    testButton.type = 'button';
    testButton.disabled = aiTests.get(id) === 'testing';
    testButton.addEventListener('click', () => aiListModels(id, 'test'));
  }
  pane.append(detailHead(label, { button: testButton }),
    accEl('div', 'pane-title', aiTitle(kind, account)));

  if (!account) {
    pane.append(accEl('div', 'page-text', provider.local
      ? t('accounts.address_checked', { name: provider.name })
      : ON_WINDOWS ? t('accounts.key_checked_windows', { name: provider.name }) : t('accounts.key_checked_file', { name: provider.name })));
  }
  pane.append(renderAiStats(id, kind, account));

  const test = aiTests.get(id);
  if (test && test !== 'testing') {
    pane.append(notice(test.ok ? 'info' : 'error', test.message));
  }
  if (accountError && accountError.account === id) {
    pane.append(notice('error', accountError.message));
  }

  if (!account) {
    pane.append(accEl('div', 'page-text', provider.local
      ? t('accounts.listed_after_address')
      : t('accounts.listed_after_key')));
    return pane;
  }
  pane.append(renderAiModels(account));

  const sends = accEl('div', 'ai-sends');
  sends.append(accEl('span', 'label', t('accounts.sends')),
    accEl('span', '', t('accounts.sends_item')));
  pane.append(sends);

  // A key is disconnected (forgotten) first; the account is then removed.
  const [text, op, opLabel, armed] = !provider.local && account.key
    ? [t('accounts.disconnect_ai_note'), 'disconnect_account', t('accounts.disconnect_dots'), t('accounts.disconnect_again')]
    : [t('accounts.remove_ai_note'), 'remove_account', t('accounts.remove_dots'), t('accounts.remove_again')];
  const button = accEl('button', 'btn btn--ghost', opLabel);
  button.type = 'button';
  armedButton(button, armed, () => accountOp(op, id), `${op} ${id}`);
  pane.append(detailFooter(text, button));
  return pane;
}
