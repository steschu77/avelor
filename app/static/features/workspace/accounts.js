// Workspace -> Accounts (accounts.md): the open vault's accounts, rendered
// into the expanded vault area while its op is "accounts" -- the AI
// providers: the AI provider menu, an AI account's card and pane, and the
// pane of a new one until its key or address checks out. A required field
// (the key, Ollama's address) stays an input until the host has checked
// it; then it shows as text with a pencil.

import { accEl, oncePerFrame } from '../../core/dom.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import { pageDate, plural } from '../../core/format.js';
import { store } from '../../model/state.js';
import { accounts } from '../../model/entities.js';
import { showMenu } from '../../ui/menu.js';
import { armedButton } from '../../ui/confirm.js';
import {
  addTile, detailFooter, detailHead, detailPane, draftKind, failedRetry, itemCard, listDetailPage,
  listSection, notice, pageList, replaceVaultPage, sentDrafts, setPageFocus, stat, stats,
} from '../../ui/page.js';
import { fieldEditKey, fieldEdits, renderEditField } from '../../ui/field.js';
import { AI_PROVIDERS, aiHost, aiProvider, aiTitle, aiUsd } from './models.js';

let selectedAccount = null; // an account id, or 'new:<kind>' for a new AI account
let accountError = null; // { account, message } from the last failed op
// account id -> its status as last seen: every account the page knows
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
  if (selectedAccount === null && accountAccess.size === 0) {
    selectedAccount = id; // the first account
  }
  accountAccess.set(id, meta.status);
}

function onAccountRemoved(id) {
  accountAccess.delete(id);
  aiAccountRemoved(id);
  if (selectedAccount === id) selectedAccount = accounts.keys().next().value ?? null;
}

// ----------------------------------------------------------------------------
// actions

// What to show for a failed call; null for one that was cancelled or
// replaced.
function accountErrorText(e, account) {
  switch (e.code) {
    case 'cancelled':
    case 'superseded':
      return null;
    case 'not_found':
      return "The account isn't there any more.";
    case 'invalid_path':
      return 'Open a vault first: accounts are kept in it.';
    default:
      return e.message;
  }
}

// `disconnect_account` or `remove_account`, on the second click of its
// button (`armedButton`).
function accountOp(cmd, id) {
  const settled = e => {
    const message = e && accountErrorText(e);
    accountError = message ? { account: id, message } : null;
    scheduleRenderAccounts();
  };
  rpcCall(cmd, { account: id }).then(() => settled(null), settled);
}

// ----------------------------------------------------------------------------
// rendering

const scheduleRenderAccounts = oncePerFrame(renderAccounts);

function renderAccounts() {
  if (store.getState().vault_op !== 'accounts') return;
  replaceVaultPage(listDetailPage(renderAccountList(), renderAccountDetail()));
}

// The selected account's pane or a new AI account's; nothing without a
// vault to keep an account in.
function renderAccountDetail() {
  if (!store.getState().vault_path) return detailPane();
  const kind = draftKind(selectedAccount);
  if (kind) return renderAiPane(null, kind);
  const account = accounts.get(selectedAccount);
  if (!account) return detailPane();
  return renderAiPane(account);
}

function renderAccountList() {
  const list = pageList('Accounts', 'AI providers.');
  if (!store.getState().vault_path) {
    list.append(accEl('div', 'page-text', 'Accounts are kept in the vault: open a vault to see them.'));
    return list;
  }

  const ai = [...accounts.values()];
  if (ai.length) list.append(listSection('AI providers'));
  for (const account of ai) list.append(renderAiCard(account));

  list.append(listSection('Connect'), renderAiConnect());
  return list;
}

// Re-render while visible when the op, the vault or the accounts change.
store.subscribe([
  state => state.vault_op, state => state.vault_path, state => state.accounts_rev,
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
      return accountErrorText(e);
    default:
      return e.message;
  }
}

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
      failed.error = aiErrorText(e);
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
    const message = aiErrorText(e) ?? 'Listing the models was cancelled.';
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
    const message = e && aiErrorText(e);
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

// Tick or untick a model: ticked, the features' pickers offer it.
function aiToggleOffered(account, modelId, on) {
  const offered = aiSettings(account).offered.filter(m => m !== modelId);
  if (on) offered.push(modelId);
  aiSetSettings(account, { offered });
}

// ----------------------------------------------------------------------------
// formatting

function aiPrice(price) {
  if (!price) return '\u2014';
  if (price.input === 0 && price.output === 0) return 'free';
  return `${aiUsd(price.input)} / ${aiUsd(price.output)}`;
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
      ? `${account.key} \u00b7 ${offered}`
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
  const hint = 'calls stop at the limit';
  return stat('Spend limit', field, note ?? hint);
}

function renderAiStats(id, kind, account) {
  const provider = aiProvider(kind);
  const accountId = account
    ? stat('Account id', accEl('span', 'edit-field__text mono mono--small',
      `${account.id.slice(0, 8)}\u2026${account.id.slice(-4)}`), 'in every ai: block')
    : stat('Account id', '\u2014', 'given on connect');

  if (!provider.local) {
    return stats(
      renderAiKeyStat(id, kind, account),
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

// The model list: tick to offer a model in the features' pickers.
function renderAiModels(account) {
  const provider = aiProvider(account.kind);
  const section = accEl('div', 'ai-models');
  const head = accEl('div', 'ai-models__head');
  head.append(
    accEl('span', 'subtitle ai-models__title', provider.local ? 'Models on this computer' : 'Models on this key'),
    accEl('span', 'page-text', 'Ticked models are offered wherever a model is picked.'));
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
    // Once per account and session.
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
  columns.append(accEl('span'), accEl('span', '', 'Model'), accEl('span', '', 'Per 1M in / out'));
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
  row.append(check, name, price);
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

  // A key is disconnected (forgotten) first; the account is then removed.
  const [text, op, opLabel] = !provider.local && account.key
    ? ['Disconnecting forgets the key and keeps everything it wrote.',
      'disconnect_account', 'Disconnect']
    : ['Removing forgets the account. What it wrote keeps naming it.',
      'remove_account', 'Remove account'];
  const button = accEl('button', 'btn btn--ghost', `${opLabel}\u2026`);
  button.type = 'button';
  armedButton(button, `Click again to ${opLabel.toLowerCase()}`, () => accountOp(op, id), `${op} ${id}`);
  pane.append(detailFooter(text, button));
  return pane;
}
