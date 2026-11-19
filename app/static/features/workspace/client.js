// Workspace -> Accounts: the user's own Google client (accounts.md#own-client)
// in a build that has none of its own, above the access rows of the Google
// connect and of each Google account. It is given as its id and secret, or
// as the file Google hands out (`set_google_client`, `load_google_client`);
// the host checks it with Google before it keeps it. Kept, it shows as its
// id and its secret's ends, with Replace. `google_client` (`ready`, and its
// event) is 'built-in', the own client, or null while there is none.

import { accEl } from '../../core/dom.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import { t } from '../../core/text.js';
import { store } from '../../model/state.js';
import { fieldInput } from '../../ui/field.js';
import { notice } from '../../ui/page.js';
import { openLink } from '../selection.js';

const GUIDE = 'https://github.com/steschu77/avelor/blob/main/docs/accounts.md#own-client';

// The client being given, { id, secret, checking, error }; null while the
// kept one shows.
let draft = null;

// A client was kept: the form closes.
rpcOn('google_client', client => {
  draft = null;
  store.setState({ google_client: client });
});

// Whether the build takes the user's own client.
export function ownGoogleClient() {
  const { google, google_client } = store.getState();
  return google && google_client !== 'built-in';
}

// Whether there is a client to sign in with.
export function hasGoogleClient() {
  return store.getState().google_client != null;
}

// Send the client (`call`), the form waiting for the answer; the event
// before it closes the form.
function keepClient(call, args, rerender) {
  const sent = draft;
  sent.checking = true;
  sent.error = null;
  rpcCall(call, args).then(() => {
    sent.checking = false;
    rerender();
  }, e => {
    sent.checking = false;
    if (e.code !== 'cancelled') sent.error = e.message;
    rerender();
  });
  rerender();
}

export function renderGoogleClient(rerender) {
  const client = store.getState().google_client;
  const box = accEl('div', 'accounts__sub');
  const head = accEl('div', 'accounts__client');
  const guide = accEl('button', 'btn btn--ghost accounts__guide', t('accounts.client.guide'));
  guide.type = 'button';
  guide.addEventListener('click', () => openLink(GUIDE));
  head.append(accEl('span', 'subtitle', t('accounts.client.head')), guide);
  box.append(head, accEl('div', 'page-text', t('accounts.client.text')));
  if (client && !draft) {
    box.append(renderKeptClient(client, rerender));
  } else {
    box.append(...renderClientForm(client, rerender));
  }
  return box;
}

function renderKeptClient(client, rerender) {
  const row = accEl('div', 'accounts__client');
  row.append(accEl('span', 'label', t('accounts.client.id')), accEl('span', 'mono accounts__client-id', client.id),
    accEl('span', 'meta', t('accounts.client.secret_ends', { secret: client.secret })));
  const replace = accEl('button', 'btn', t('accounts.client.replace'));
  replace.type = 'button';
  replace.addEventListener('click', () => {
    draft = { id: '', secret: '', checking: false, error: null };
    rerender();
  });
  row.append(replace);
  return row;
}

// The id and the secret, Save, and Google's file; Cancel while replacing.
function renderClientForm(client, rerender) {
  draft ??= { id: '', secret: '', checking: false, error: null };
  const form = draft;
  const canSave = () => !form.checking && form.id.trim() !== '' && form.secret.trim() !== '';
  const save = accEl('button', 'btn btn--primary', t('field.save'));
  save.type = 'button';
  save.disabled = !canSave();
  const submit = () => {
    if (canSave()) keepClient('set_google_client', { id: form.id, secret: form.secret }, rerender);
  };
  save.addEventListener('click', submit);

  const field = (name, type, label, placeholder) => {
    const input = fieldInput(type, `google-client\n${name}`, form[name], 'input input--mono edit-field__input');
    input.placeholder = placeholder;
    input.spellcheck = false;
    input.autocomplete = 'off';
    input.disabled = form.checking;
    input.addEventListener('input', () => {
      form[name] = input.value;
      save.disabled = !canSave();
    });
    input.addEventListener('keydown', e => {
      if (e.key !== 'Enter') return;
      e.preventDefault();
      submit();
    });
    const row = accEl('label', 'accounts__client');
    row.append(accEl('span', 'label', label), input);
    return row;
  };

  const load = accEl('button', 'btn', t('accounts.client.load'));
  load.type = 'button';
  load.disabled = form.checking;
  load.addEventListener('click', () => keepClient('load_google_client', {}, rerender));
  const actions = accEl('div', 'accounts__actions');
  actions.append(save, load);
  if (client) {
    const cancel = accEl('button', 'btn btn--ghost', t('field.cancel'));
    cancel.type = 'button';
    cancel.disabled = form.checking;
    cancel.addEventListener('click', () => {
      draft = null;
      rerender();
    });
    actions.append(cancel);
  }

  const parts = [
    field('id', 'text', t('accounts.client.id'), '\u2026.apps.googleusercontent.com'),
    field('secret', 'password', t('accounts.client.secret'), 'GOCSPX-\u2026'),
    actions,
  ];
  if (form.checking) parts.push(accEl('span', 'meta', t('accounts.client.checking')));
  else if (form.error) parts.push(accEl('div', 'error-note', form.error));
  if (client) parts.push(notice('hint', t('accounts.client.replace_note')));
  return parts;
}
