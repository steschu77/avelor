// Workspace -> Accounts (accounts.md): the open vault's accounts, rendered
// into the expanded vault area while its op is "accounts" -- the Google
// accounts and their connect, then the AI providers: the AI provider menu,
// an AI account's card and pane, and the pane of a new one until its key or
// address checks out. A required field (the key, Ollama's address) stays an
// input until the host has checked it; then it shows as text with a pencil.
// A model picker's way here is the `accounts.open` action
// (`openAccountsPage`). The sidebar follows `accounts_rev` (app.js).

import { accEl, oncePerFrame } from '../../core/dom.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import { pageDate, plural } from '../../core/format.js';
import { registerAction } from '../../core/actions.js';
import { store } from '../../model/state.js';
import { accounts } from '../../model/entities.js';
import { vaultSettings } from '../../model/entities.js';
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

// The access rows shown; `tasks` stays in the account (and in every
// reconnect request) without a row.
const ACCOUNT_SERVICES = [
  { key: 'email', name: 'Mail', product: 'Gmail' },
  { key: 'calendar', name: 'Calendar', product: 'Google Calendar' },
  { key: 'contacts', name: 'Contacts', product: 'Google Contacts' },
];
const ACCOUNT_LEVELS = ['off', 'read', 'full'];
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
      return 'The sign-in timed out.';
    case 'denied':
      return 'The sign-in was declined.';
    case 'wrong_account':
      return `Signed in with another Google account than ${accounts.get(account)?.email ?? 'this one'}.`;
    case 'not_connected':
      return "The account isn't connected.";
    case 'not_found':
      return "The account isn't there any more.";
    case 'invalid_path':
      return 'Open a vault first: accounts are kept in it.';
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
  connected: 'connected',
  disconnected: 'disconnected',
  reconnect: 'needs reconnect',
};

// Calendar ops report on the calendar list, not on the pane.
function requestCalendars(account) {
  accountCalendars.set(account, 'loading');
  rpcCall('list_calendars', { account }).then(({ calendars }) => {
    accountCalendars.set(account, calendars);
    scheduleRenderAccounts();
  }, e => {
    const message = accountErrorText(e, account) ?? 'Listing the calendars was cancelled.';
    accountCalendars.set(account, { error: `Could not list the calendars: ${message}` });
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

// "Mail read \u00b7 Calendar full", or "No access".
function accountAccessSummary(scope) {
  const parts = ACCOUNT_SERVICES
    .filter(({ key }) => scope?.[key] && scope[key] !== 'off')
    .map(({ key, name }) => `${name} ${scope[key]}`);
  return parts.length ? parts.join(' \u00b7 ') : 'No access';
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
  const list = pageList('Accounts', 'Remote sources and AI providers.');
  if (!store.getState().vault_path) {
    list.append(accEl('div', 'page-text', 'Accounts are kept in the vault: open a vault to see them.'));
    return list;
  }

  const google = [...accounts.values()].filter(account => !isAiAccount(account));
  if (google.length) list.append(listSection('Sources'));
  for (const account of google) {
    list.append(itemCard({
      kind: 'Google',
      status: ACCOUNT_STATUS_LABELS[account.status] ?? account.status,
      title: account.email ?? 'Unknown account',
      sub: accountAccessSummary(account.scope),
      current: account.id === selectedAccount,
      onSelect: () => {
        selectedAccount = account.id;
        scheduleRenderAccounts();
      },
    }));
  }

  const ai = [...accounts.values()].filter(isAiAccount);
  if (ai.length) list.append(listSection('AI providers'));
  for (const account of ai) list.append(renderAiCard(account));

  list.append(listSection('Connect'));
  if (store.getState().google) {
    list.append(addTile({
      label: 'Google account',
      hint: 'sign in',
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
    const seg = accSeg(ACCOUNT_LEVELS.map(level => [level, level]), levels[key] ?? 'off',
      level => setAccountLevel(account, key, level), pendingConnect !== null);
    row.append(label, seg);
    rows.append(row);

    // A connected account's calendars can each become a source.
    const granted = accounts.get(account);
    const connected = granted?.status === 'connected';
    if (key === 'calendar' && connected && granted.scope?.calendar !== 'off') {
      rows.append(renderCalendarSources(account));
    }
  }
  return rows;
}

// Checkboxes for the account's calendars: checked ones show as sources in
// the sidebar. Loaded once per account (and again after a reconnect).
function renderCalendarSources(account) {
  const list = accEl('div', 'accounts__sub');
  list.append(accEl('span', 'label', 'Calendars shown as sources'));
  let calendars = accountCalendars.get(account);
  if (calendars === undefined) {
    requestCalendars(account);
    calendars = 'loading';
  }
  if (calendars === 'loading') {
    list.append(accEl('span', 'page-text', 'Loading calendars\u2026'));
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
    if (calendar.primary) pick.append(accEl('span', 'label', 'primary'));
    if (missing.includes(calendar)) pick.append(accEl('span', 'label', 'not found on Google'));
    row.append(pick);
    list.append(row);
  }
  if (!calendars.length && !missing.length) {
    list.append(accEl('span', 'page-text', 'This account has no calendars.'));
  }
  return list;
}

// "Continue in your browser..." with Cancel, or the last error, if any.
function renderConnectState(account) {
  if (pendingConnect?.account === account) {
    const cancel = accEl('button', 'btn', 'Cancel');
    cancel.type = 'button';
    cancel.addEventListener('click', cancelConnect);
    return notice('info', accEl('span', '', 'Continue in your browser\u2026'), cancel);
  }
  if (accountError && accountError.account === account) return notice('error', accountError.message);
  return null;
}

function renderConnectPane() {
  const pane = detailPane();
  pane.append(detailHead('Connect \u00b7 Google account'),
    accEl('div', 'pane-title', 'Google account'),
    accEl('div', 'page-text',
      'Sign in with Google in your browser. Avelor asks for the access chosen here, plus your name and email address.'),
    renderAccessRows('new'));

  const actions = accEl('div', 'accounts__actions');
  const signIn = accEl('button', 'btn btn--primary', 'Sign in with Google');
  signIn.type = 'button';
  signIn.disabled = pendingConnect !== null;
  signIn.addEventListener('click', () => connectGoogle('new'));
  actions.append(signIn);
  pane.append(actions);

  const state = renderConnectState('new');
  if (state) pane.append(state);
  return pane;
}

function renderAccountPane(account) {
  const pane = detailPane();
  const connected = account.status === 'connected';

  const reconnect = accEl('button', 'btn', connected ? 'Reconnect' : 'Connect');
  reconnect.type = 'button';
  reconnect.disabled = pendingConnect !== null;
  reconnect.addEventListener('click', () => connectGoogle(account.id));
  const heading = connected
    ? `Account \u00b7 Google \u00b7 connected ${pageDate(account.connected)}`
    : `Account \u00b7 Google \u00b7 ${ACCOUNT_STATUS_LABELS[account.status] ?? account.status}`;
  pane.append(detailHead(heading, { button: reconnect }),
    accEl('div', 'pane-title', account.email ?? 'Unknown account'));

  pane.append(stats(
    stat('Connected since', account.connected ? pageDate(account.connected) : 'Unknown'),
  ), renderAccessRows(account.id));

  // An edit only takes effect with the next consent.
  const draft = accountDrafts.get(account.id);
  if (connected && draft && ACCOUNT_SERVICES.some(({ key }) => (draft[key] ?? 'off') !== (account.scope?.[key] ?? 'off'))) {
    pane.append(notice('info', 'Reconnect to apply the changed access.'));
  }
  if (account.status === 'disconnected') {
    pane.append(notice('info',
      'Disconnected: nothing is read from this account. Connecting the same Google account again continues with this account and its documents.'));
  } else if (account.status === 'reconnect') {
    pane.append(notice('error',
      'The sign-in for this account is missing on this computer. Connect to use it again.'));
  }
  const state = renderConnectState(account.id);
  if (state) pane.append(state);

  // Connected accounts disconnect; the others can be removed for good.
  const [text, op, label] = connected
    ? ['Disconnecting signs out and keeps the account and every document, so connecting it again continues where it left off.',
      'disconnect_account', 'Disconnect']
    : ['Removing forgets the account. Documents keep naming it, but connecting the same Google account later starts a new one.',
      'remove_account', 'Remove account'];
  const button = accEl('button', 'btn btn--ghost', `${label}\u2026`);
  button.type = 'button';
  button.disabled = pendingConnect !== null;
  armedButton(button, `Click again to ${label.toLowerCase()}`, () => accountOp(op, account.id),
    `${op} ${account.id}`);
  pane.append(detailFooter(text, button));
  return pane;
}

// Re-render while visible when the op, the vault, the accounts, or what
// picked a model (the vault's settings: "Used by") change.
store.subscribe([
  state => state.vault_op, state => state.vault_path, state => state.accounts_rev,
  state => state.settings_rev,
], () => scheduleRenderAccounts());

// ----------------------------------------------------------------------------
// AI providers (accounts.md#accounts-page)

const AI_STATUS_LABELS = {
  connected: 'connected',
  disconnected: 'disconnected',
  reconnect: 'needs a key',
  running: 'running',
  'not running': 'not running',
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
  if (is(vaultSettings().chat_model)) users.push('Chats');
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
registerAction({ id: 'accounts.open', title: 'Open Accounts', run: openAccountsPage });

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
      aiTests.set(account, { ok: true, message: `Works \u00b7 ${plural(count, 'model')}` });
    }
    scheduleRenderAccounts();
  }, e => {
    const message = aiErrorText(e, account) ?? 'Listing the models was cancelled.';
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
  return `$${(n ?? 0).toFixed(2)}`;
}

function aiPrice(price) {
  if (!price) return '\u2014';
  if (price.input === 0 && price.output === 0) return 'free';
  return `${aiUsd(price.input)} / ${aiUsd(price.output)}`;
}

// "12 calls this month, 2 not priced": what "This month" counts; calls of a
// model the price table doesn't know cost nothing there.
function aiSpentNote(spent) {
  const calls = spent?.calls ?? 0;
  const note = `${plural(calls, 'call')} this month`;
  return spent?.unpriced ? `${note}, ${spent.unpriced} not priced` : note;
}

function aiOfferedText(count) {
  return `${plural(count, 'model')} offered`;
}

// ----------------------------------------------------------------------------
// the list: cards and the AI provider menu

function renderAiCard(account) {
  // A local server's status is checked as soon as its card shows.
  if (aiProvider(account.kind).local && !aiModels.has(account.id)) aiRequestModels(account.id);
  const provider = aiProvider(account.kind);
  const status = aiModels.get(account.id) === 'loading' && account.status === 'unknown'
    ? 'checking' : AI_STATUS_LABELS[account.status] ?? account.status;

  const offered = aiOfferedText(account.offered?.length ?? 0);
  const meta = provider.local
    ? `${aiHost(account.address)} \u00b7 ${offered}`
    : account.key
      ? `${account.key} \u00b7 ${offered} \u00b7 ${aiCost(account.spent?.cost)} this month`
      : `No key \u00b7 ${offered}`;
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
    label: 'AI provider',
    hint: '\u25be',
    current: !!draftKind(selectedAccount),
    onClick: tile => {
      const items = AI_PROVIDERS.map((provider, i) => ({
        label: provider.name,
        hint: provider.local ? 'local' : 'key',
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
const KEY_KEPT = navigator.platform.startsWith('Win')
  ? 'encrypted for this Windows user' : 'in a file only this user can read';

// The key of a key provider, checked before it is stored.
function renderAiKeyStat(id, kind, account) {
  const provider = aiProvider(kind);
  const [field, note] = renderEditField(id, 'key', {
    saved: account?.key ?? null,
    required: true,
    type: 'password',
    mono: true,
    placeholder: provider.placeholder,
    editTitle: 'Replace key',
    onSave: key => aiConnect(id, 'key', key),
    rerender: scheduleRenderAccounts,
  });
  const hint = account?.status === 'reconnect'
    ? 'The saved key can\u2019t be read here; enter it again.'
    : account?.status === 'disconnected'
      ? 'Enter the key to connect again.'
      : KEY_KEPT;
  return stat('Key', field, note ?? hint);
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
    editTitle: 'Change address',
    onSave: address => aiConnect(id, 'address', address),
    rerender: scheduleRenderAccounts,
  });
  return stat('Address', field, note ?? `where ${provider.name} answers`);
}

function renderAiSpendStat(account) {
  const limit = aiSettings(account.id).spend_limit;
  const [field, note] = renderEditField(account.id, 'spend_limit', {
    saved: limit == null ? null : `${aiUsd(limit)} a month`,
    empty: 'No limit',
    value: limit == null ? '' : String(limit),
    placeholder: 'USD a month, empty for none',
    editTitle: 'Change limit',
    onSave: value => {
      const amount = value === '' ? null : Number(value.replace(',', '.').replace(/^\$/, ''));
      if (amount !== null && !(amount > 0)) {
        const key = fieldEditKey(account.id, 'spend_limit');
        fieldEdits.set(key, { value, error: 'Enter an amount above 0, or nothing for no limit.' });
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
    ? 'reached: calls stop until next month or a higher limit'
    : 'calls stop at the limit';
  return stat('Spend limit', field, note ?? hint);
}

function renderAiStats(id, kind, account) {
  const provider = aiProvider(kind);
  const accountId = account
    ? stat('Account id', accEl('span', 'edit-field__text mono mono--small',
      `${account.id.slice(0, 8)}\u2026${account.id.slice(-4)}`), 'in every ai: block')
    : stat('Account id', '\u2014', 'given on connect');

  if (!provider.local) {
    const spent = account?.spent;
    return stats(
      renderAiKeyStat(id, kind, account),
      account
        ? stat('This month', aiCost(spent?.cost), aiSpentNote(spent))
        : stat('This month', '\u2014'),
      account ? renderAiSpendStat(account) : stat('Spend limit', '\u2014'),
      accountId);
  }

  const list = account ? aiModels.get(account.id) : undefined;
  let status = '\u2014';
  let statusNote = null;
  if (account) {
    if (list === 'loading' && account.status === 'unknown') status = 'Checking\u2026';
    else if (account.status === 'running') status = 'Running';
    else if (account.status === 'not running') status = 'Not running';
    if (account.status === 'running' && Array.isArray(list)) {
      statusNote = `${plural(list.length, 'model')} on this computer`;
    } else if (account.status === 'not running') {
      statusNote = `Start ${provider.name}, then Test.`;
    }
  }
  const offered = account ? aiSettings(account.id).offered.length : null;
  return stats(
    renderAiAddressStat(id, kind, account),
    stat('Status', status, statusNote),
    stat('Offered', offered == null ? '\u2014'
      : Array.isArray(list) ? `${offered} of ${list.length}` : String(offered), 'in model pickers'),
    accountId);
}

// The model list: tick to offer a model in the features' pickers; what
// picked it is shown, not chosen, here.
function renderAiModels(account) {
  const provider = aiProvider(account.kind);
  const section = accEl('div', 'ai-models');
  const head = accEl('div', 'ai-models__head');
  head.append(
    accEl('span', 'subtitle ai-models__title', provider.local ? 'Models on this computer' : 'Models on this key'),
    accEl('span', 'page-text', 'Ticked models are offered wherever a model is picked: in the assistant.'));
  const canList = provider.local || !!account.key;
  if (canList) {
    const refresh = accEl('button', 'btn btn--ghost', 'Refresh list');
    refresh.type = 'button';
    refresh.disabled = aiModels.get(account.id) === 'loading';
    refresh.addEventListener('click', () => aiListModels(account.id));
    head.append(refresh);
  }
  section.append(head);

  if (!canList) {
    section.append(accEl('div', 'page-text', 'Enter the key to list the models.'));
    return section;
  }
  let list = aiModels.get(account.id);
  if (list === undefined) {
    // Once per account and session, like the calendars.
    aiRequestModels(account.id);
    list = 'loading';
  }
  if (list === 'loading') {
    section.append(accEl('div', 'page-text', 'Loading models\u2026'));
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
  columns.append(accEl('span'), accEl('span', '', 'Model'), accEl('span', '', 'Per 1M in / out'),
    accEl('span', '', 'Used by'));
  section.append(columns);
  for (const model of [...ticked, ...rest]) {
    section.append(renderAiModelRow(account, model, !rest.includes(model)));
  }
  if (!list.length && !ticked.length) {
    section.append(accEl('div', 'page-text', provider.local
      ? 'No models on this computer yet. Pull one with `ollama pull`, then Refresh list.'
      : 'This key offers no models.'));
  }
  return section;
}

function renderAiModelRow(account, model, ticked) {
  const row = accEl('div', 'list-row ai-models__row');
  if (!ticked) row.classList.add('ai-models__row--off');

  const check = accEl('button', 'check', ticked ? '\u2713' : '');
  check.type = 'button';
  check.setAttribute('aria-pressed', String(ticked));
  check.title = ticked ? 'Stop offering this model' : 'Offer this model';
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
  if (!ticked) uses.append(accEl('span', 'note', users.length ? 'not offered' : 'hidden from pickers'));
  else if (!users.length) uses.append(accEl('span', 'note', '\u2014'));
  if (model.missing) uses.append(accEl('span', 'note', 'no longer listed'));
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

  let label = `Connect \u00b7 AI provider \u00b7 ${provider.name}`;
  if (account?.status === 'connected') {
    label = `AI provider \u00b7 ${provider.name} \u00b7 connected ${pageDate(account.connected)}`;
  } else if (account) {
    const status = AI_STATUS_LABELS[account.status] ?? account.status;
    label = `AI provider \u00b7 ${provider.name}${status ? ` \u00b7 ${status}` : ''}`;
  }
  let testButton = null;
  if (account && (provider.local || account.key)) {
    testButton = accEl('button', 'btn', aiTests.get(id) === 'testing' ? 'Testing\u2026' : 'Test');
    testButton.type = 'button';
    testButton.disabled = aiTests.get(id) === 'testing';
    testButton.addEventListener('click', () => aiListModels(id, 'test'));
  }
  pane.append(detailHead(label, { button: testButton }),
    accEl('div', 'pane-title', aiTitle(kind, account)));

  if (!account) {
    pane.append(accEl('div', 'page-text', provider.local
      ? `The address is checked with ${provider.name} before the account is saved.`
      : `The key is checked with ${provider.name} before it is saved, then kept ${KEY_KEPT}.`));
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
      ? 'The models are listed once the address checks out.'
      : 'The models are listed once the key checks out.'));
    return pane;
  }
  pane.append(renderAiModels(account));

  const sends = accEl('div', 'ai-sends');
  sends.append(accEl('span', 'label', 'Sends'),
    accEl('span', '', 'The item a task works on.'));
  pane.append(sends);

  // A key is disconnected (forgotten) first; the account is then removed.
  const [text, op, opLabel] = !provider.local && account.key
    ? ['Disconnecting forgets the key and keeps everything it wrote. What picked its models waits for the key or another model.',
      'disconnect_account', 'Disconnect']
    : ['Removing forgets the account. What it wrote keeps naming it; what picked its models needs another model.',
      'remove_account', 'Remove account'];
  const button = accEl('button', 'btn btn--ghost', `${opLabel}\u2026`);
  button.type = 'button';
  armedButton(button, `Click again to ${opLabel.toLowerCase()}`, () => accountOp(op, id), `${op} ${id}`);
  pane.append(detailFooter(text, button));
  return pane;
}
