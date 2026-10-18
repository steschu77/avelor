// The models features pick (accounts.md#model-choice): the AI providers, a
// pick as the host checks it, the picker a feature shows and what keeps a pick
// from working. The way to the Accounts page is the `accounts.open` action
// (features/workspace/accounts.js).

import { accEl } from '../../core/dom.js';
import { locale, t } from '../../core/text.js';
import { runAction } from '../../core/actions.js';
import { accounts } from '../../model/entities.js';
import { showMenu } from '../../ui/menu.js';

export const AI_PROVIDERS = [
  { kind: 'anthropic', name: 'Anthropic', placeholder: 'sk-ant-\u2026' },
  { kind: 'openai', name: 'OpenAI', placeholder: 'sk-\u2026' },
  { kind: 'mistral', name: 'Mistral', placeholder: t('accounts.api_key') },
  { kind: 'gemini', name: 'Gemini', placeholder: 'AIza\u2026' },
  { kind: 'ollama', name: 'Ollama', local: true, address: 'http://localhost:11434' },
];

export function aiProvider(kind) {
  return AI_PROVIDERS.find(p => p.kind === kind);
}

export function isAiAccount(account) {
  return !!aiProvider(account?.kind);
}

// The model a feature picked, `choice` ({ account, model }, or null), as
// `AiAccounts::choose` checks it: `{ account, model }`, which the page
// shows, or with `problem` what the host would refuse: 'provider' (no AI
// account at all), 'pick' (nothing picked), 'gone' (the account was
// removed), 'key' (the account has no readable key), 'offered' (the model
// isn't ticked any more) or 'limit' (the account has spent its monthly
// limit). No other model is ever put in its place.
export function aiModelOf(choice) {
  if (!choice) {
    const any = [...accounts.values()].some(isAiAccount);
    return { problem: any ? 'pick' : 'provider' };
  }
  const account = accounts.get(choice.account);
  const setup = { account, model: choice.model };
  if (!isAiAccount(account)) return { model: choice.model, problem: 'gone' };
  if (!aiUsable(account)) return { ...setup, problem: 'key' };
  if (!(account.offered ?? []).includes(choice.model)) return { ...setup, problem: 'offered' };
  return aiLimitReached(account) ? { ...setup, problem: 'limit' } : setup;
}

// Whether `account` can be called: Ollama always, a key provider with its
// key.
function aiUsable(account) {
  return aiProvider(account.kind).local || account.status === 'connected';
}

// Whether `account` has spent its monthly limit (`spent` is this month's).
export function aiLimitReached(account) {
  return account?.spend_limit != null && (account.spent?.cost ?? 0) >= account.spend_limit;
}

// Whether any AI account offers a model to pick.
export function aiAnyOffered() {
  return [...accounts.values()].some(a => isAiAccount(a) && a.offered?.length);
}

// What `aiModelOf` found wrong (`ai.problem`) for `feature` (a `feature.` text:
// OCR, the scan analysis), in the host's words, and a button to Accounts where the fix
// is there: elements to put into a note beside the feature's picker.
export function aiMissingHint(ai, feature) {
  const name = ai.account ? aiProvider(ai.account.kind).name : '';
  const pick = aiAnyOffered() ? t('ai.pick_model', { feature }) : t('ai.tick_then_pick', { feature });
  const text = {
    provider: t('ai.add_provider', { feature }),
    pick,
    gone: t('ai.picked_account_gone', { feature }),
    key: t('ai.key_again', { feature, name }),
    offered: t('ai.not_offered', { feature, model: ai.model }),
    limit: t('ai.limit_for', { feature, name }),
  }[ai.problem];
  const atAccounts = ai.problem !== 'gone' && (ai.problem !== 'pick' || !aiAnyOffered());
  if (!atAccounts) return [accEl('span', '', text)];
  const open = accEl('button', 'btn btn--ghost', t('workspace.accounts'));
  open.type = 'button';
  open.addEventListener('click', () => aiOpenAccounts(ai.account?.id));
  return [accEl('span', '', text), open];
}

// The picker of a feature's model: a button naming the model `choice` picks
// (the provider in its title), or "Pick a model", marked while the pick has
// a problem (`aiModelOf`). A click opens the menu of the models every AI
// account offers, by account; picking one calls `onPick({ account, model })`.
// Fills `button` (one the page keeps, as the assistant's head does), or a
// new one.
export function aiModelPicker(choice, onPick, button = accEl('button')) {
  const ai = aiModelOf(choice);
  button.type = 'button';
  button.className = 'tool ai-pick';
  button.replaceChildren(accEl('span', 'ai-pick__name', choice?.model ?? t('models.pick')),
    accEl('span', '', '\u2304'));
  if (ai.problem) button.setAttribute('aria-invalid', 'true');
  else button.removeAttribute('aria-invalid');
  button.title = ai.account ? t('models.change', { name: aiTitle(ai.account.kind, ai.account) }) : t('models.pick');
  button.onclick = ev => {
    ev.stopPropagation();
    aiShowModels(button, choice, onPick);
  };
  return button;
}

// The menu of `aiModelPicker`, at `anchor`: per AI account its offered
// models (an account without its key dimmed), the pick checked, and the way
// to Accounts.
export function aiShowModels(anchor, choice, onPick) {
  const items = [];
  for (const account of accounts.values()) {
    if (!isAiAccount(account) || !account.offered?.length) continue;
    const usable = aiUsable(account);
    items.push({ head: aiTitle(account.kind, account) });
    for (const model of account.offered) {
      items.push({
        label: model,
        hint: usable ? '' : t('models.needs_key'),
        checked: choice?.account === account.id && choice?.model === model,
        dim: !usable,
        run: usable ? () => onPick({ account: account.id, model }) : undefined,
      });
    }
  }
  if (!items.length) items.push({ label: t('models.none_ticked'), dim: true, checked: false });
  items.push({ label: t('models.under_accounts'), separator: true, checked: false, run: () => aiOpenAccounts() });
  showMenu(anchor, items, { matchWidth: false });
}

// Open Workspace -> Accounts on `account`'s pane, or on the page as it is
// (the `accounts.open` action, features/workspace/accounts.js).
export function aiOpenAccounts(account) {
  runAction('accounts.open', account);
}

// ----------------------------------------------------------------------------
// formatting

export function aiHost(address) {
  return (address ?? '').replace(/^[a-z]+:\/\//i, '').split('/')[0];
}

function aiIsLoopback(address) {
  return /^(localhost|127\.|\[::1\])/i.test(aiHost(address));
}

// The provider's name; Ollama's with "local", or with its host elsewhere.
export function aiTitle(kind, account) {
  const provider = aiProvider(kind);
  if (!provider.local || !account) return provider.name;
  return `${provider.name} \u00b7 ${aiIsLoopback(account.address) ? t('models.local') : aiHost(account.address)}`;
}

// A price or limit: "$10", "$1.25", "$0.40", "$0.075".
export function aiUsd(n) {
  const cents = Math.round(n * 1e5) / 1e3;
  const digits = Number.isInteger(n) ? 0 : Number.isInteger(cents) ? 2 : 3;
  return new Intl.NumberFormat(locale, { style: 'currency', currency: 'USD', minimumFractionDigits: digits, maximumFractionDigits: digits })
    .format(n);
}
