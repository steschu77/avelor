// Fetch email (email.md#fetch-email, `email_*`): the email import of a Google
// account, opened from the sync button of its Email source (`syncSource`,
// features/sidebar.js) and rendered into the expanded vault area while its op
// is "email". Also the account's auto-sync under the Email row of its pane
// (called by features/workspace/accounts.js), with the same filter form. Both
// can save the attachments of new email (which file types, up to what size),
// and turn on its analysis, with the model it runs with. On the page, Esc
// closes it and Ctrl+Enter fetches (`registerVaultPage`).
//
// The filter form is built once per opening and never re-rendered on Fetch
// email, so its inputs keep focus while typing; only its chips and the matches
// pane are. Filter edits are debounced: in Auto-update mode each settled edit
// searches again (`email_search`), in Manual mode it only flags the list as
// stale until Update. A fetch runs as a job; the page follows its `job` events
// to read the matches again.

import { $, accEl, oncePerFrame } from '../../core/dom.js';
import { pressable } from '../../core/dom.js';
import { titled } from '../../core/dom.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import { isoInstant, localDay, shortMonthDate } from '../../core/format.js';
import { pageDate } from '../../core/format.js';
import { shortcutText } from '../../core/actions.js';
import { t } from '../../core/text.js';
import { store } from '../../model/state.js';
import { accounts } from '../../model/entities.js';
import { dropBox, showMenu } from '../../ui/menu.js';
import { armedButton } from '../../ui/confirm.js';
import {
  closeVaultArea, openVaultOp, registerVaultPage,
} from '../../ui/page.js';
import { keepingFocus, renderPatternField } from '../../ui/field.js';
import { accSeg } from '../../ui/segmented.js';
import { iconEl } from '../../ui/icons.js';
import {
  AI_ANALYSIS_OFF, aiModelOf, aiPickerWithHint,
} from '../workspace/models.js';
import { WATCH_READ_STEP } from '../workspace/folders.js';

const EMAIL_PAGE_SIZE = 50; // matches per page, as the host pages them
const EMAIL_DEBOUNCE_MS = 500;
const EMAIL_ONLY = [
  ['starred', t('email.only.starred')], ['attachment', t('email.only.attachment')], ['unread', t('email.only.unread')],
  ['important', t('email.only.important')], ['sent', t('email.only.sent')],
];
const EMAIL_DATE_PRESETS = [
  [null, t('email.since.none')], ['last', t('email.since.last')], ['day', t('email.since.day')],
  ['week', t('email.since.week')], ['month', t('email.since.month')], ['custom', t('email.since.custom')],
];

let email = null; // the open page's state, see openEmailFetch
const emailFetchJobs = new Map(); // job id -> account, of the fetches started here

// ----------------------------------------------------------------------------
// opening and closing

// Open the page for `account_id`, filled with the filter of its last fetch.
export function openEmailFetch(account_id) {
  const account = accounts.get(account_id);
  if (!account) return;
  if (email) clearTimeout(email.timer);

  const saved = account.fetch_email;
  const filter = { labels: [], from: [], to: [], words: '', ...structuredClone(saved?.filter ?? {}) };
  if (!saved || (filter.since === 'last' && !saved.last_fetch)) {
    filter.since = saved?.last_fetch ? 'last' : 'week';
  }
  const before = filter.before ? new Date(filter.before) : null;
  if (before) before.setDate(before.getDate() - 1); // stored exclusive, shown inclusive

  email = {
    account: account_id,
    filter,
    custom: {
      from: filter.since === 'custom' ? emailDateInput(filter.after) : '',
      to: filter.since === 'custom' && before ? emailDateInput(before) : '',
    },
    auto: saved?.auto_update ?? true,
    analysis: { ...AI_ANALYSIS_OFF, ...saved?.analysis },
    attachments: { ...EMAIL_ATTACHMENTS_OFF, ...saved?.attachments },
    searching: false, // while a `email_search` / `email_headers` is answered
    error: null,
    matches: null, // [{ id, in_vault }] of the last search
    truncated: false,
    listFilter: null, // the filter JSON `matches` was read with
    headers: new Map(), // id -> { date, from, subject, in_vault }
    selected: new Set(),
    page: 0,
    fromTop: false, // the matches list starts at its top when drawn next
    fetching: false, // while `email_fetch` is answered
    fetchError: null,
    timer: null,
    els: {},
  };
  // The filter form edits `email.filter` and `email.custom` in place.
  email.form = {
    account: account_id, filter: email.filter, custom: email.custom, text: {},
    withDate: true, focusKey: 'email', onChange: emailFilterChanged, els: {},
  };
  emailLabelCache.delete(account_id); // its labels as they are now
  emailLabels(account_id);
  openVaultOp('email', t('sidebar.fetch_email'));
  renderEmailPage();
  if (email.auto) emailSearch();
}

function closeEmailPage() {
  if (email) clearTimeout(email.timer);
  closeVaultArea();
}

// Leaving the page (another op, or the area closed) stops pending work.
store.subscribe([state => state.vault_op, state => state.vault_expanded], ([vault_op, vault_expanded]) => {
  if (!email || (vault_op === 'email' && vault_expanded)) return;
  clearTimeout(email.timer);
  if (vault_op !== 'email') email = null;
});

// The account changed (e.g. its last fetch after a fetch): refresh the
// header line and the date presets that depend on it; and the attachments'
// reading and the analysis, whose picks may have changed with the AI
// accounts.
store.subscribe(state => state.accounts_rev, () => {
  if (!email) return;
  if (!accounts.has(email.account)) return closeEmailPage();
  renderEmailSubtitle();
  renderEmailDate(email.form);
  renderEmailAttachmentsSection();
  renderEmailAnalysisSection();
  scheduleRenderEmailMatches();
});

// Ctrl+Enter clicks Fetch, so replacing bodies takes it twice too.
registerVaultPage('email', { close: closeEmailPage, submit: () => email?.els.fetch?.click() });

// ----------------------------------------------------------------------------
// host answers and events: an answer is dropped if the page was opened again
// meanwhile (`email` is no longer the state that asked)

// The account's labels, for the "+" menus of the filter forms: asked once,
// then shared (Fetch email asks again on every opening); a failure is shown
// in the menu.
const emailLabelCache = new Map(); // account -> { labels: [{ id, name }] | null, error }

function emailLabels(account) {
  let entry = emailLabelCache.get(account);
  if (!entry) {
    entry = { labels: null, error: null };
    emailLabelCache.set(account, entry);
    rpcCall('email_labels', { account }).then(({ labels }) => {
      entry.labels = labels;
    }, e => {
      entry.error = e.message;
    });
  }
  return entry;
}

// `email_search`'s answer: the matches replace the list.
function onEmailMatches(reply) {
  email.searching = false;
  email.error = null;
  email.matches = reply.matches;
  email.truncated = reply.truncated;
  email.headers = new Map();
  email.page = 0;
  email.fromTop = true;
  email.selected = new Set(reply.matches.filter(m => !m.in_vault).map(m => m.id));
  emailApplyHeaders(reply.headers);
  scheduleRenderEmailMatches();
}

// `email_headers`' answer: a page's headers.
function onEmailHeaders(headers) {
  email.searching = false;
  emailApplyHeaders(headers);
  scheduleRenderEmailMatches();
}

// A preview request failed; a superseded one has its successor running.
function emailPreviewFailed(m, e) {
  if (e.code === 'superseded' || email !== m) return;
  m.searching = false;
  m.error = e.message;
  scheduleRenderEmailMatches();
}

// Keep the headers of a page. A header can reveal an email as already in the
// vault (by Message-ID, e.g. imported from another account): it is unticked.
function emailApplyHeaders(headers) {
  for (const header of headers) {
    email.headers.set(header.id, header);
    const match = email.matches?.find(m => m.id === header.id);
    if (match && header.in_vault && !match.in_vault) {
      match.in_vault = true;
      email.selected.delete(header.id);
    }
  }
}

// A fetch started here ended: a shown page for that account reads its
// matches again (Auto-update), or flags them as stale.
rpcOn('job', job => {
  if (job.state !== 'done' || !emailFetchJobs.has(job.id)) return;
  const account = emailFetchJobs.get(job.id);
  emailFetchJobs.delete(job.id);
  if (email?.account !== account) return;
  const shown = store.getState().vault_op === 'email' && store.getState().vault_expanded;
  if (shown && email.auto) {
    emailSearch();
  } else {
    email.listFilter = null; // the list predates the fetch
    scheduleRenderEmailMatches();
  }
});

// ----------------------------------------------------------------------------
// filter

// The filter as the host takes it: dates resolved to UTC instants.
function emailHostFilter() {
  const [after, before] = emailDateBounds(email.filter.since, email.account, email.custom);
  const filter = { ...email.filter };
  delete filter.after;
  delete filter.before;
  if (after) filter.after = isoInstant(after);
  if (before) filter.before = isoInstant(before);
  return filter;
}

// [after, before] of the date preset `since`, from local midnight; `last`
// is the account's last fetch, `custom` the range `custom` holds.
function emailDateBounds(since, account, custom) {
  const midnight = daysAgo => {
    const date = new Date();
    date.setHours(0, 0, 0, 0);
    date.setDate(date.getDate() - daysAgo);
    return date;
  };
  switch (since) {
    case 'last': {
      const last = accounts.get(account)?.fetch_email?.last_fetch;
      return [last ? new Date(last) : null, null];
    }
    case 'day': return [midnight(1), null];
    case 'week': return [midnight(7), null];
    case 'month': {
      const date = midnight(0);
      date.setMonth(date.getMonth() - 1);
      return [date, null];
    }
    case 'custom': {
      const from = emailParseDateInput(custom.from);
      const to = emailParseDateInput(custom.to);
      if (to) to.setDate(to.getDate() + 1); // the end day is included
      return [from, to];
    }
    default: return [null, null];
  }
}

// A local date as an <input type="date"> value (yyyy-mm-dd), and back.
function emailDateInput(date) {
  return date ? localDay(new Date(date)) : '';
}

function emailParseDateInput(value) {
  const [y, m, d] = (value ?? '').split('-').map(Number);
  return y && m && d ? new Date(y, m - 1, d) : null;
}

// The filter changed: debounce, then search (Auto-update) or flag the list as stale (Manual).
function emailFilterChanged() {
  clearTimeout(email.timer);
  email.timer = setTimeout(() => {
    email.timer = null;
    if (email.auto) emailSearch();
  }, EMAIL_DEBOUNCE_MS);
  scheduleRenderEmailMatches();
}

// Search with the current filter; a newer search or page supersedes it.
function emailSearch() {
  clearTimeout(email.timer);
  email.timer = null;
  const m = email;
  const filter = emailHostFilter();
  m.listFilter = JSON.stringify(filter);
  m.searching = true;
  m.error = null;
  rpcCall('email_search', { account: m.account, filter }, { channel: 'email-preview' }).then(reply => {
    if (email === m) onEmailMatches(reply);
  }, e => emailPreviewFailed(m, e));
  scheduleRenderEmailMatches();
}

// Whether the list was read with another filter than the current one.
function emailIsStale() {
  return email.matches !== null && (email.timer !== null || email.listFilter !== JSON.stringify(emailHostFilter()));
}

// ----------------------------------------------------------------------------
// selection, paging, fetching

function emailSelect(which) {
  const ids = email.matches?.filter(m => which === 'all' || (which === 'new' && !m.in_vault)) ?? [];
  email.selected = new Set(ids.map(m => m.id));
  scheduleRenderEmailMatches();
}

function emailToggle(id) {
  if (email.selected.has(id)) email.selected.delete(id);
  else email.selected.add(id);
  scheduleRenderEmailMatches();
}

function emailPageCount() {
  return Math.max(1, Math.ceil((email.matches?.length ?? 0) / EMAIL_PAGE_SIZE));
}

function emailShowPage(page) {
  email.page = Math.max(0, Math.min(page, emailPageCount() - 1));
  email.fromTop = true;
  const start = email.page * EMAIL_PAGE_SIZE;
  const missing = email.matches.slice(start, start + EMAIL_PAGE_SIZE)
    .map(m => m.id)
    .filter(id => !email.headers.has(id));
  if (missing.length) {
    const m = email;
    m.searching = true;
    rpcCall('email_headers', { account: m.account, ids: missing }, { channel: 'email-preview' }).then(({ headers }) => {
      if (email === m) onEmailHeaders(headers);
    }, e => emailPreviewFailed(m, e));
  }
  scheduleRenderEmailMatches();
}

// The selected emails already in the vault: their bodies get replaced.
function emailReplacing() {
  return email.matches?.filter(m => m.in_vault && email.selected.has(m.id)) ?? [];
}

// Whether Fetch can run: a list to fetch from, not about to be replaced by
// a pending Auto-update search, and the attachments' reading and the
// analysis off or with a working pick.
function emailCanFetch() {
  return email.matches !== null && email.selected.size > 0 && !email.fetching
    && !email.searching && !(email.auto && email.timer !== null)
    && !emailReadingBlocked(email.attachments) && !emailAnalysisBlocked(email.analysis);
}

// Fetch the selection; replacing bodies takes a second click on Fetch
// (`renderEmailFooter`). The account remembers the filter the list was read
// with.
function emailFetch() {
  if (!email || !emailCanFetch()) return;
  const replace = emailReplacing().map(m => m.id);
  const m = email;
  m.fetchError = null;
  m.fetching = true;
  const ids = m.matches.filter(match => m.selected.has(match.id)).map(match => match.id);
  const filter = m.listFilter ? JSON.parse(m.listFilter) : emailHostFilter();
  const args = {
    account: m.account, ids, replace, filter, auto_update: m.auto, analysis: m.analysis,
    attachments: m.attachments,
  };
  // Once the fetch runs, the page closes; the job card shows it.
  rpcCall('email_fetch', args).then(({ job }) => {
    emailFetchJobs.set(job, args.account);
    if (email !== m) return;
    m.fetching = false;
    closeEmailPage();
  }, e => {
    if (email !== m) return;
    m.fetching = false;
    m.fetchError = e.message;
    scheduleRenderEmailMatches();
  });
  scheduleRenderEmailMatches();
}

// ----------------------------------------------------------------------------
// rendering: the page, built once per opening

function renderEmailPage() {
  const els = email.els;
  const page = accEl('div', 'page email');

  const head = accEl('div', 'page-head');
  els.subtitle = accEl('span', 'meta');
  head.append(accEl('div', 'page-title', t('sidebar.fetch_email')), els.subtitle);
  page.append(head, accEl('div', 'page-text', t('email.intro')));

  const grid = accEl('div', 'email__grid');
  const form = accEl('div', 'email__form');
  const section = accEl('div', 'section-head', t('email.section.filter'));
  section.append(accEl('span', 'section-head__note', t('email.all_must_match')));
  const filter = accEl('div', 'section-body');
  filter.append(renderEmailFilterForm(email.form));
  form.append(section, filter);

  els.attachments = accEl('div');
  els.analysis = accEl('div');
  form.append(els.attachments, els.analysis);

  els.matches = accEl('div', 'email__matches');
  grid.append(form, els.matches);
  page.append(grid);
  $('vault-area-body').replaceChildren(page);

  renderEmailAttachmentsSection();
  renderEmailAnalysisSection();
  renderEmailSubtitle();
  renderEmailMatches();
}

function renderEmailSubtitle() {
  const account = accounts.get(email.account);
  const last = account?.fetch_email?.last_fetch;
  email.els.subtitle.textContent = [
    'Gmail', account?.email ?? t('accounts.google_account'),
    last ? t('email.last_fetch', { date: shortMonthDate(new Date(last), true) }) : t('email.never_fetched'),
  ].join(' \u00b7 ');
}

// ----------------------------------------------------------------------------
// the filter form, of Fetch email and of an account's auto-sync
//
// `form`: { account (whose labels the "+" menu offers), filter (edited in
// place), custom ({ from, to } of a custom date range), text (the text
// inputs as typed, by key, so a rebuilt form shows them as they were),
// withDate (the Date row), focusKey (prefix of the inputs'
// data-focus-key), onChange (called after every edit), els (filled in
// here) }. The rows' chips and the date re-render in place, so the inputs
// keep focus while typing.

function renderEmailFilterForm(form) {
  const els = form.els;
  const box = accEl('div', 'email__filter');
  const rows = accEl('div', 'form-grid');
  const row = (key, field) => {
    rows.append(accEl('span', 'label', key), field);
    return field;
  };

  els.labels = row(t('email.labels'), accEl('div', 'form-field'));

  const only = row(t('email.only'), accEl('div', 'form-field'));
  for (const [key, name] of EMAIL_ONLY) {
    const toggle = accEl('button', 'email__toggle', name);
    toggle.type = 'button';
    toggle.setAttribute('aria-pressed', String(!!form.filter[key]));
    toggle.addEventListener('click', () => {
      form.filter[key] = !form.filter[key];
      toggle.setAttribute('aria-pressed', String(form.filter[key]));
      form.onChange();
    });
    only.append(toggle);
  }

  const textRow = (key, name, placeholder, list) => {
    const field = row(name, accEl('div', 'form-field'));
    const input = accEl('input', 'form-input');
    input.type = 'text';
    input.placeholder = placeholder;
    input.spellcheck = false;
    input.dataset.focusKey = `${form.focusKey}-${key}`;
    input.value = form.text[key] ?? (list ? form.filter[key].join(', ') : form.filter[key]);
    input.addEventListener('input', () => {
      form.text[key] = input.value;
      form.filter[key] = list
        ? input.value.split(/[,;]/).map(s => s.trim()).filter(Boolean)
        : input.value;
      form.onChange();
    });
    field.append(input);
  };
  textRow('from', t('props.from'), t('email.from_placeholder'), true);
  textRow('to', t('email.to'), t('email.to_placeholder'), true);
  textRow('words', t('email.words'), t('email.words_placeholder'), false);

  if (form.withDate) els.date = row(t('docs.date'), accEl('div', 'form-field form-field--wrap'));
  box.append(rows);

  renderEmailLabels(form);
  renderEmailDate(form);
  return box;
}

// Label chips (click to remove), "+" for the account's labels, any/all.
function renderEmailLabels(form) {
  const field = form.els.labels;
  field.replaceChildren();
  const { labels } = form.filter;
  for (const name of labels) {
    const chip = accEl('button', 'chip chip--action', `${name} \u00d7`);
    chip.type = 'button';
    chip.title = t('id_documents.remove');
    chip.addEventListener('click', () => {
      form.filter.labels = labels.filter(l => l !== name);
      renderEmailLabels(form);
      form.onChange();
    });
    field.append(chip);
  }
  const add = accEl('button', 'email__add', labels.length ? '+' : t('email.add_label'));
  add.type = 'button';
  titled(add, t('email.add_label_title'));
  add.addEventListener('click', ev => {
    ev.stopPropagation();
    const known = emailLabels(form.account);
    const items = known.labels === null
      ? [{ label: known.error ? t('email.labels_unavailable', { error: known.error }) : t('email.loading_labels'), dim: true }]
      : known.labels.filter(l => !labels.includes(l.name)).map(l => ({
        label: l.name,
        run: () => {
          form.filter.labels = [...form.filter.labels, l.name];
          renderEmailLabels(form);
          form.onChange();
        },
      }));
    showMenu(add, items.length ? items : [{ label: t('email.no_more_labels'), dim: true }]);
  });
  field.append(add);

  if (labels.length > 1) {
    field.append(accSeg([[false, t('email.any_of')], [true, t('email.all_of')]], !!form.filter.every, every => {
      form.filter.every = every;
      renderEmailLabels(form);
      form.onChange();
    }));
  }
}

// The date preset menu, and the range inputs of a custom range; nothing
// without the Date row.
function renderEmailDate(form) {
  const field = form.els.date;
  if (!field) return;
  field.replaceChildren();
  const hint = key => {
    const [after] = key === 'custom' ? [] : emailDateBounds(key, form.account, form.custom);
    return after ? shortMonthDate(after) : '';
  };
  const lastFetch = accounts.get(form.account)?.fetch_email?.last_fetch;

  const [, name] = EMAIL_DATE_PRESETS.find(([key]) => key === (form.filter.since ?? null)) ?? EMAIL_DATE_PRESETS[0];
  const button = dropBox(accEl('button'), name, hint(form.filter.since));
  button.addEventListener('click', ev => {
    ev.stopPropagation();
    showMenu(button, EMAIL_DATE_PRESETS.map(([key, label]) => {
      const date = hint(key);
      const unavailable = key === 'last' && !lastFetch;
      return {
        label,
        hint: date,
        checked: key === (form.filter.since ?? null),
        dim: unavailable,
        run: unavailable ? undefined : () => {
          form.filter.since = key;
          renderEmailDate(form);
          form.onChange();
        },
      };
    }));
  });
  field.append(button);

  if (form.filter.since === 'custom') {
    const range = accEl('span', 'email__range');
    for (const [key, name] of [['from', t('props.from')], ['to', t('email.to')]]) {
      const input = accEl('input', 'input');
      input.type = 'date';
      input.value = form.custom[key];
      input.addEventListener('change', () => {
        form.custom[key] = input.value;
        form.onChange();
      });
      range.append(accEl('span', 'label', name), input);
    }
    range.append(accEl('span', 'meta', t('email.both_optional')));
    field.append(range);
  }
}

// ----------------------------------------------------------------------------
// auto-sync (email.md#email-auto-sync): an account's own filter, apart from
// the one Fetch email remembers, set up under the Email row of its pane
// (features/workspace/accounts.js) with the form above; the host fetches what is new every 15
// minutes (`set_email_sync`)

const emailSyncForms = new Map(); // account id -> the form of its auto-sync filter

// A vault opened: its accounts' forms start from what they keep.
rpcOn('accounts', () => emailSyncForms.clear());

// The Email row's auto-sync: a check, and while it is on, the filter form of
// what it fetches; `rerender`: the Accounts page's scheduler, for a
// refused change's note (`emailSetSync`).
export function renderEmailSync(account, rerender) {
  const sync = account.fetch_email?.auto_sync;
  const on = !!sync?.enabled;
  const form = emailSyncForm(account);
  form.rerender = rerender;
  const box = accEl('div', 'accounts__sub');
  const head = accEl('div', 'accounts__sync');
  const check = accEl('button', 'check', on ? '\u2713' : '');
  check.type = 'button';
  check.setAttribute('aria-pressed', String(on));
  titled(check, on ? t('accounts.stop_sync') : t('email.sync_title'));
  check.addEventListener('click', () => emailSetSync(account.id, { enabled: !on }));
  head.append(check, accEl('span', 'subtitle', t('accounts.auto_sync')), accEl('span', 'note', on
    ? t('email.sync_on', { date: pageDate(sync.since, true) })
    : t('email.sync_off')));
  box.append(head);
  if (on) {
    box.append(renderEmailFilterForm(form));
    box.append(renderEmailSyncAttachments(form, { ...EMAIL_ATTACHMENTS_OFF, ...sync.attachments }));
    box.append(renderEmailSyncAnalysis(form, { ...AI_ANALYSIS_OFF, ...sync.analysis }));
  }
  if (form.error) box.append(accEl('div', 'error-note', form.error));
  return box;
}

// The form of `account`'s auto-sync filter: made once from the filter it
// keeps, then edited here, so a re-render of the pane keeps what is typed.
// An edit is saved once typing pauses.
function emailSyncForm(account) {
  let form = emailSyncForms.get(account.id);
  if (form) return form;
  const saved = account.fetch_email?.auto_sync?.filter ?? {};
  form = {
    account: account.id,
    filter: { labels: [], from: [], to: [], words: '', ...structuredClone(saved) },
    custom: { from: '', to: '' },
    text: {},
    withDate: false,
    focusKey: `email-sync-${account.id}`,
    timer: null,
    error: null,
    els: {},
  };
  form.onChange = () => {
    clearTimeout(form.timer);
    form.timer = setTimeout(() => {
      form.timer = null;
      emailSetSync(form.account, { filter: form.filter });
    }, EMAIL_DEBOUNCE_MS);
  };
  emailSyncForms.set(account.id, form);
  emailLabels(account.id); // for the "+" menu
  return form;
}

// Change `account`'s auto-sync; the `account` event shows it, a refusal
// shows under it.
function emailSetSync(account, change) {
  const form = emailSyncForms.get(account);
  rpcCall('set_email_sync', { account, ...change }).then(() => {
    if (form?.error) {
      form.error = null;
      form.rerender();
    }
  }, e => {
    if (form) form.error = e.message;
    form?.rerender();
  });
}

// ----------------------------------------------------------------------------
// the attachments of new email (email.md#attachments), of Fetch email and of an
// account's auto-sync: whether they are saved, which ones, up to what size,
// and whether they are read, with which model

const EMAIL_ATTACHMENTS_OFF = {
  enabled: false, patterns: ['*.pdf', '*.jpg', '*.jpeg', '*.png'], 'max-size': 10,
  read: false, models: {},
};
const EMAIL_ATTACHMENT_SIZES = [[5, '5 MB'], [10, '10 MB'], [25, '25 MB']];

// `attachments` ({ enabled, patterns, 'max-size', read, models }) as a seg,
// Off or On, and a body: while on, the file types (`renderPatternField`, of
// `owner`), the size they are saved up to, and whether they are read
// (`renderEmailReading`). `onChange` gets the attachments as changed;
// `rerender` (the page's scheduler) runs for an edit that changes nothing.
function renderEmailAttachments(owner, attachments, onChange, rerender) {
  const seg = accSeg([[false, t('watch.off')], [true, t('watch.on')]], attachments.enabled,
    enabled => onChange({ ...attachments, enabled }));
  const body = accEl('div');
  if (!attachments.enabled) {
    body.append(accEl('div', 'page-text email__text', t('email.attachments_off')));
    return { seg, body };
  }
  const types = accEl('div', 'form-field');
  types.append(...renderPatternField(owner, attachments.patterns,
    patterns => onChange({ ...attachments, patterns }), rerender));
  const size = accEl('div', 'form-field');
  size.append(accSeg(EMAIL_ATTACHMENT_SIZES, attachments['max-size'],
    max => onChange({ ...attachments, 'max-size': max })));
  const read = accEl('div', 'form-field');
  read.append(accSeg([[false, t('watch.off')], [true, t('watch.on')]], !!attachments.read,
    on => onChange({ ...attachments, read: on })));
  const grid = accEl('div', 'form-grid');
  grid.append(accEl('span', 'label', t('watch.file_types')), types, accEl('span', 'label', t('email.up_to')), size,
    accEl('span', 'label', t('watch.label.read')), read);
  body.append(grid, accEl('div', 'page-text email__text', t('email.attachments_on')));
  if (attachments.read) {
    const { model, text } = renderEmailReading(owner, attachments, onChange);
    grid.append(...model);
    body.append(text);
  }
  return { seg, body };
}

// While the attachments are read: the row of the model that reads them
// (`emailModelRow`, its box's focus key after `owner`), and the sentence of
// what it reads with that model while the pick works, else what keeps it
// from working (`aiPickerWithHint`).
function renderEmailReading(owner, attachments, onChange) {
  const pick = attachments.models?.[WATCH_READ_STEP] ?? null;
  const { picker, hint } = aiPickerWithHint(pick,
    model => onChange({ ...attachments, models: { ...attachments.models, [WATCH_READ_STEP]: model } }),
    t('feature.reading_attachments'), { box: true, focusKey: `${owner}-reading-model` });
  const text = accEl('div', 'page-text email__text');
  text.append(hint ?? t('email.reads_with', { model: pick.model }));
  return { model: emailModelRow(picker), text };
}

// The row of a model's box (`aiModelPicker`) in a form grid: its label and
// its field.
function emailModelRow(picker) {
  const field = accEl('div', 'form-field');
  field.append(picker);
  return [accEl('span', 'label', t('accounts.model')), field];
}

// Whether `attachments` are saved and read, but the reading's pick wouldn't
// work (the host refuses a fetch then).
function emailReadingBlocked(attachments) {
  return attachments.enabled && !!attachments.read
    && !!aiModelOf(attachments.models?.[WATCH_READ_STEP] ?? null).problem;
}

// Fetch email's second section, Attachments: built again on its changes, the
// focus kept in its file types' input.
function renderEmailAttachmentsSection() {
  const block = email.els.attachments;
  const { seg, body } = renderEmailAttachments('email-fetch', email.attachments, attachments => {
    email.attachments = attachments;
    renderEmailAttachmentsSection();
    scheduleRenderEmailMatches();
  }, renderEmailAttachmentsSection);
  const head = accEl('div', 'section-head', t('email.section.attachments'));
  head.append(seg);
  keepingFocus(block, () => block.replaceChildren(head, body));
}

// The auto-sync's attachments, under its filter; saved at once.
function renderEmailSyncAttachments(form, attachments) {
  const save = next => emailSetSync(form.account, { attachments: next });
  const { seg, body } = renderEmailAttachments(form.focusKey, attachments, save, () => form.rerender());
  const grid = accEl('div', 'form-grid');
  const field = accEl('div', 'form-field');
  field.append(seg);
  grid.append(accEl('span', 'label', t('email.attachments')), field);
  const box = accEl('div', 'email__filter');
  box.append(grid, body);
  return box;
}

// ----------------------------------------------------------------------------
// the analysis of new email (email.md#analysis), of Fetch email and of an
// account's auto-sync: a level, and the model it runs with

// `analysis` ({ level, model }) as a seg of its levels, the row of its
// model while it is on (`emailModelRow`, its box's focus key after
// `owner`; none while off), and a body: the sentence of what it sends to
// that model while the pick works, else what keeps it from working
// (`aiPickerWithHint`). `onChange` gets the analysis as changed.
function renderEmailAnalysis(owner, analysis, onChange) {
  const seg = accSeg([['off', t('watch.off')], ['lite', t('watch.level.lite')], ['medium', t('watch.level.medium')]], analysis.level,
    level => onChange({ ...analysis, level }));
  const body = accEl('div', 'page-text email__text');
  if (analysis.level === 'off') {
    body.append(t('email.analysis_off'));
    return { seg, model: [], body };
  }
  const { picker, hint } = aiPickerWithHint(analysis.model, model => onChange({ ...analysis, model }),
    t('feature.email_analysis'), { box: true, focusKey: `${owner}-analysis-model` });
  const model = analysis.model?.model;
  body.append(hint ?? (analysis.level === 'medium'
    ? t('email.analysis_medium', { model }) : t('email.analysis_lite', { model })));
  return { seg, model: emailModelRow(picker), body };
}

// Whether `analysis` is on but its pick wouldn't work (the host refuses a
// fetch then).
function emailAnalysisBlocked(analysis) {
  return analysis.level !== 'off' && !!aiModelOf(analysis.model).problem;
}

// Fetch email's third section, Analysis: built again on its changes, and
// when the accounts change (the pick's problems), the focus kept on its
// model's box.
function renderEmailAnalysisSection() {
  const block = email.els.analysis;
  const { seg, model, body } = renderEmailAnalysis('email-fetch', email.analysis, analysis => {
    email.analysis = analysis;
    renderEmailAnalysisSection();
    scheduleRenderEmailMatches();
  });
  const head = accEl('div', 'section-head', t('email.section.analysis'));
  head.append(seg);
  const grid = accEl('div', 'form-grid');
  grid.append(...model);
  keepingFocus(block, () => block.replaceChildren(head, grid, body));
}

// The auto-sync's analysis, under its filter; saved at once.
function renderEmailSyncAnalysis(form, analysis) {
  const { seg, model, body } = renderEmailAnalysis(form.focusKey, analysis,
    next => emailSetSync(form.account, { analysis: next }));
  const grid = accEl('div', 'form-grid');
  const field = accEl('div', 'form-field');
  field.append(seg);
  grid.append(accEl('span', 'label', t('watch.step.analysis')), field, ...model);
  const box = accEl('div', 'email__filter');
  box.append(grid, body);
  return box;
}

// ----------------------------------------------------------------------------
// rendering: the matches pane

const scheduleRenderEmailMatches = oncePerFrame(() => {
  if (email?.els.matches) renderEmailMatches();
});

function renderEmailMatches() {
  const box = accEl('div', 'email__box');

  const head = accEl('div', 'section-head', t('email.matches'));
  head.append(accSeg([[true, t('email.auto_update')], [false, t('email.manual')]], email.auto, auto => {
    email.auto = auto;
    if (auto && emailIsStale()) emailSearch();
    scheduleRenderEmailMatches();
  }));
  if (!email.auto) {
    const refresh = accEl('button', 'icon-btn');
    refresh.append(iconEl('#i-arrow-sync'));
    refresh.type = 'button';
    refresh.title = t('email.update_matches');
    refresh.disabled = email.searching;
    refresh.addEventListener('click', emailSearch);
    head.append(refresh);
  }
  box.append(head);

  if (!email.auto && emailIsStale()) {
    const stale = accEl('div', 'email__stale', t('email.stale'));
    const update = accEl('button', 'btn btn--primary', t('email.update'));
    update.type = 'button';
    update.addEventListener('click', emailSearch);
    stale.append(update);
    box.append(stale);
  }
  if (email.error) box.append(accEl('div', 'error-note', email.error));

  const matches = email.matches;
  if (matches === null) {
    box.append(accEl('div', 'page-text email__empty',
      email.searching ? t('email.reading_matches') : t('email.press_update')));
    drawEmailMatches(box);
    return;
  }

  const inVault = matches.filter(m => m.in_vault).length;
  const count = accEl('div', 'email__count');
  if (email.searching) count.classList.add('email__count--busy');
  count.append(
    accEl('span', 'subtitle', email.truncated ? t('email.matches_more', { n: matches.length }) : t('email.matches_n', { n: matches.length })),
    accEl('span', 'meta', [t('email.in_vault_n', { n: inVault }), t('count.new', { n: matches.length - inVault }),
      t('email.selected_n', { n: email.selected.size })].join(' \u00b7 ')),
    accEl('span', 'meta push', email.searching ? t('email.reading') : email.auto ? t('email.updates_as_you_edit') : ''));
  box.append(count);
  if (email.truncated) {
    box.append(accEl('div', 'note email__note', t('email.truncated', { n: matches.length })));
  }

  const select = accEl('div', 'select-bar');
  for (const [which, name] of [['new', t('email.select.new')], ['all', t('email.select.all')], ['none', t('email.select.none')]]) {
    const button = accEl('button', '', name);
    button.type = 'button';
    button.dataset.focusKey = `email-select:${which}`;
    button.addEventListener('click', () => emailSelect(which));
    select.append(button);
  }
  select.append(accEl('span', 'select-bar__note', t('email.select_note')));
  box.append(select);

  const list = accEl('div', 'email__list');
  const start = email.page * EMAIL_PAGE_SIZE;
  for (const match of matches.slice(start, start + EMAIL_PAGE_SIZE)) {
    list.append(renderEmailRow(match));
  }
  if (!matches.length) list.append(accEl('div', 'page-text email__empty', t('email.no_matches')));
  box.append(list);

  if (matches.length > EMAIL_PAGE_SIZE) box.append(renderEmailPager());
  drawEmailMatches(box);
}

// The matches pane drawn anew as `box` and the footer: a tick keeps the
// list where it was scrolled to and the focus where it was; another page
// or new matches start at the top.
function drawEmailMatches(box) {
  const pane = email.els.matches;
  const top = email.fromTop ? 0 : pane.querySelector('.email__list')?.scrollTop ?? 0;
  email.fromTop = false;
  keepingFocus(pane, () => pane.replaceChildren(box, renderEmailFooter()));
  const list = pane.querySelector('.email__list');
  if (list) list.scrollTop = top;
}

function renderEmailRow(match) {
  const header = email.headers.get(match.id);
  const selected = email.selected.has(match.id);
  const row = pressable(accEl('div', 'list-row email__row'));
  row.dataset.focusKey = `email-row:${match.id}`;
  if (!header) row.classList.add('email__row--loading');
  if (selected && match.in_vault) row.classList.add('email__row--replace');

  const box = accEl('span', 'check', selected ? '\u2713' : '');
  box.setAttribute('aria-checked', String(selected));
  const subject = accEl('span', 'row-title', header ? (header.subject || t('email.no_subject')) : '\u2026');
  const date = accEl('span', 'meta email__date-cell', header ? shortMonthDate(new Date(header.date)) : '');
  const from = accEl('span', 'meta email__from', header ? emailSender(header.from) : '');
  const status = accEl('span', 'label email__status',
    match.in_vault ? (selected ? t('email.status.replace') : t('email.status.in_vault')) : t('email.status.new'));
  row.append(box, subject, date, from, status);
  row.title = header?.from ?? '';
  row.addEventListener('click', () => emailToggle(match.id));
  return row;
}

// "Hausverwaltung Krause" of `"Hausverwaltung Krause" <info@\u2026>`, else the address.
function emailSender(from) {
  const name = from.replace(/<[^>]*>/, '').replace(/"/g, '').trim();
  return name || from.replace(/[<>]/g, '').trim();
}

function renderEmailPager() {
  const pager = accEl('div', 'meta email__pager');
  const pages = emailPageCount();
  const start = email.page * EMAIL_PAGE_SIZE;
  const end = Math.min(start + EMAIL_PAGE_SIZE, email.matches.length);
  const step = (label, title, page, disabled) => {
    const button = accEl('button', 'btn', label);
    button.type = 'button';
    titled(button, title);
    button.disabled = disabled || email.searching;
    button.addEventListener('click', () => emailShowPage(page));
    return button;
  };
  pager.append(
    step('\u2039', t('email.previous_page'), email.page - 1, email.page === 0),
    accEl('span', '', t('email.page_of', { from: start + 1, to: end, n: email.matches.length })),
    step('\u203a', t('email.next_page'), email.page + 1, email.page >= pages - 1));
  return pager;
}

function renderEmailFooter() {
  const footer = accEl('div', 'email__footer');
  const n = email.selected.size;
  const replace = emailReplacing().map(m => m.id);
  const replacing = replace.length;
  const label = email.fetching ? t('email.starting')
    : email.attachments.enabled ? t('email.fetch_with_attachments', { n }) : t('email.fetch_n', { n });
  const fetch = accEl('button', 'btn btn--primary', label);
  fetch.type = 'button';
  fetch.title = shortcutText('Ctrl+Enter');
  fetch.disabled = !emailCanFetch();
  // Replacing bodies takes a second click; another selection to replace
  // asks again (the key). Red only while it can run.
  if (replacing && !fetch.disabled) {
    fetch.classList.add('btn--danger');
    armedButton(fetch, t('email.replace_again', { n: replacing }), emailFetch,
      `email_fetch ${replace.join(' ')}`);
  } else {
    fetch.addEventListener('click', emailFetch);
  }
  email.els.fetch = fetch;

  const cancel = accEl('button', 'btn btn--ghost', t('field.cancel'));
  cancel.type = 'button';
  cancel.title = shortcutText('Escape');
  cancel.addEventListener('click', closeEmailPage);
  footer.append(fetch, cancel);

  if (replacing) {
    footer.append(accEl('div', 'note email__warning', t('email.replacing', { n: replacing })));
  }
  if (emailReadingBlocked(email.attachments)) {
    footer.append(accEl('div', 'note email__warning', t('email.reading_blocked')));
  }
  if (emailAnalysisBlocked(email.analysis)) {
    footer.append(accEl('div', 'note email__warning', t('email.analysis_blocked')));
  }
  if (email.fetchError) footer.append(accEl('div', 'error-note', email.fetchError));
  return footer;
}
