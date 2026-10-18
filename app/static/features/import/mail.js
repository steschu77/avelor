// Fetch mail (mail.md#fetch-mail, `mail_*`): the email import of a Google
// account, opened from the sync button of its Mail source (`syncSource`,
// features/sidebar.js) and rendered into the expanded vault area while its op
// is "mail". Also the account's auto-sync under the Mail row of its pane
// (called by features/workspace/accounts.js), with the same filter form. Both
// can turn on the analysis of new mail, with the model it runs with. On the
// page, Esc closes it and Ctrl+Enter fetches (`registerVaultPage`).
//
// The filter form is built once per opening and never re-rendered on Fetch
// mail, so its inputs keep focus while typing; only its chips and the matches
// pane are. Filter edits are debounced: in Auto-update mode each settled edit
// searches again (`mail_search`), in Manual mode it only flags the list as
// stale until Update. A fetch runs as a job; the page follows its `job` events
// to read the matches again.

import { $, accEl, oncePerFrame } from '../../core/dom.js';
import { rpcCall, rpcOn } from '../../core/rpc.js';
import {
  isoInstant, localDay, plural, shortMonthDate,
} from '../../core/format.js';
import { pageDate } from '../../core/format.js';
import { store } from '../../model/state.js';
import { accounts } from '../../model/entities.js';
import { showMenu } from '../../ui/menu.js';
import { armedButton } from '../../ui/confirm.js';
import {
  closeVaultArea, openVaultOp, registerVaultPage,
} from '../../ui/page.js';
import { notice } from '../../ui/page.js';
import { accSeg } from '../../ui/segmented.js';
import { iconEl } from '../../ui/icons.js';
import {
  aiAnyOffered, aiMissingHint, aiModelOf, aiModelPicker,
} from '../workspace/models.js';

const MAIL_PAGE_SIZE = 50; // matches per page, as the host pages them
const MAIL_DEBOUNCE_MS = 500;
const MAIL_ONLY = [
  ['starred', 'Starred'], ['attachment', 'Has attachment'], ['unread', 'Unread'],
  ['important', 'Important'], ['sent', 'Sent by me'],
];
const MAIL_DATE_PRESETS = [
  [null, 'No date filter'], ['last', 'Since last time'], ['day', 'Since yesterday'],
  ['week', 'Since last week'], ['month', 'Since last month'], ['custom', 'Custom date range\u2026'],
];

let mail = null; // the open page's state, see openMailFetch
const mailFetchJobs = new Map(); // job id -> account, of the fetches started here

// ----------------------------------------------------------------------------
// opening and closing

// Open the page for `account_id`, filled with the filter of its last fetch.
export function openMailFetch(account_id) {
  const account = accounts.get(account_id);
  if (!account) return;
  if (mail) clearTimeout(mail.timer);

  const saved = account.mail;
  const filter = { labels: [], from: [], to: [], words: '', ...structuredClone(saved?.filter ?? {}) };
  if (!saved || (filter.since === 'last' && !saved.last_fetch)) {
    filter.since = saved?.last_fetch ? 'last' : 'week';
  }
  const before = filter.before ? new Date(filter.before) : null;
  if (before) before.setDate(before.getDate() - 1); // stored exclusive, shown inclusive

  mail = {
    account: account_id,
    filter,
    custom: {
      from: filter.since === 'custom' ? mailDateInput(filter.after) : '',
      to: filter.since === 'custom' && before ? mailDateInput(before) : '',
    },
    auto: saved?.auto_update ?? true,
    analysis: { ...MAIL_ANALYSIS_OFF, ...saved?.analysis },
    searching: false, // while a `mail_search` / `mail_headers` is answered
    error: null,
    matches: null, // [{ id, in_vault }] of the last search
    truncated: false,
    listFilter: null, // the filter JSON `matches` was read with
    headers: new Map(), // id -> { date, from, subject, in_vault }
    selected: new Set(),
    page: 0,
    fetching: false, // while `mail_fetch` is answered
    fetchError: null,
    timer: null,
    els: {},
  };
  // The filter form edits `mail.filter` and `mail.custom` in place.
  mail.form = {
    account: account_id, filter: mail.filter, custom: mail.custom, text: {},
    withDate: true, focusKey: 'mail', onChange: mailFilterChanged, els: {},
  };
  mailLabelCache.delete(account_id); // its labels as they are now
  mailLabels(account_id);
  openVaultOp('mail', 'Fetch mail');
  renderMailPage();
  if (mail.auto) mailSearch();
}

function closeMailPage() {
  if (mail) clearTimeout(mail.timer);
  closeVaultArea();
}

// Leaving the page (another op, or the area closed) stops pending work.
store.subscribe([state => state.vault_op, state => state.vault_expanded], ([vault_op, vault_expanded]) => {
  if (!mail || (vault_op === 'mail' && vault_expanded)) return;
  clearTimeout(mail.timer);
  if (vault_op !== 'mail') mail = null;
});

// The account changed (e.g. its last fetch after a fetch): refresh the
// header line and the date presets that depend on it; and the analysis,
// whose pick may have changed with the AI accounts.
store.subscribe(state => state.accounts_rev, () => {
  if (!mail) return;
  if (!accounts.has(mail.account)) return closeMailPage();
  renderMailSubtitle();
  renderMailDate(mail.form);
  renderMailAnalysisSection();
  scheduleRenderMailMatches();
});

// Ctrl+Enter clicks Fetch, so replacing bodies takes it twice too.
registerVaultPage('mail', { close: closeMailPage, submit: () => mail?.els.fetch?.click() });

// ----------------------------------------------------------------------------
// host answers and events: an answer is dropped if the page was opened again
// meanwhile (`mail` is no longer the state that asked)

// The account's labels, for the "+" menus of the filter forms: asked once,
// then shared (Fetch mail asks again on every opening); a failure is shown
// in the menu.
const mailLabelCache = new Map(); // account -> { labels: [{ id, name }] | null, error }

function mailLabels(account) {
  let entry = mailLabelCache.get(account);
  if (!entry) {
    entry = { labels: null, error: null };
    mailLabelCache.set(account, entry);
    rpcCall('mail_labels', { account }).then(({ labels }) => {
      entry.labels = labels;
    }, e => {
      entry.error = e.message;
    });
  }
  return entry;
}

// `mail_search`'s answer: the matches replace the list.
function onMailMatches(reply) {
  mail.searching = false;
  mail.error = null;
  mail.matches = reply.matches;
  mail.truncated = reply.truncated;
  mail.headers = new Map();
  mail.page = 0;
  mail.selected = new Set(reply.matches.filter(m => !m.in_vault).map(m => m.id));
  mailApplyHeaders(reply.headers);
  scheduleRenderMailMatches();
}

// `mail_headers`' answer: a page's headers.
function onMailHeaders(headers) {
  mail.searching = false;
  mailApplyHeaders(headers);
  scheduleRenderMailMatches();
}

// A preview request failed; a superseded one has its successor running.
function mailPreviewFailed(m, e) {
  if (e.code === 'superseded' || mail !== m) return;
  m.searching = false;
  m.error = e.message;
  scheduleRenderMailMatches();
}

// Keep the headers of a page. A header can reveal a mail as already in the
// vault (by Message-ID, e.g. imported from another account): it is unticked.
function mailApplyHeaders(headers) {
  for (const header of headers) {
    mail.headers.set(header.id, header);
    const match = mail.matches?.find(m => m.id === header.id);
    if (match && header.in_vault && !match.in_vault) {
      match.in_vault = true;
      mail.selected.delete(header.id);
    }
  }
}

// A fetch started here ended: a shown page for that account reads its
// matches again (Auto-update), or flags them as stale.
rpcOn('job', job => {
  if (job.state !== 'done' || !mailFetchJobs.has(job.id)) return;
  const account = mailFetchJobs.get(job.id);
  mailFetchJobs.delete(job.id);
  if (mail?.account !== account) return;
  const shown = store.getState().vault_op === 'mail' && store.getState().vault_expanded;
  if (shown && mail.auto) {
    mailSearch();
  } else {
    mail.listFilter = null; // the list predates the fetch
    scheduleRenderMailMatches();
  }
});

// ----------------------------------------------------------------------------
// filter

// The filter as the host takes it: dates resolved to UTC instants.
function mailHostFilter() {
  const [after, before] = mailDateBounds(mail.filter.since, mail.account, mail.custom);
  const filter = { ...mail.filter };
  delete filter.after;
  delete filter.before;
  if (after) filter.after = isoInstant(after);
  if (before) filter.before = isoInstant(before);
  return filter;
}

// [after, before] of the date preset `since`, from local midnight; `last`
// is the account's last fetch, `custom` the range `custom` holds.
function mailDateBounds(since, account, custom) {
  const midnight = daysAgo => {
    const date = new Date();
    date.setHours(0, 0, 0, 0);
    date.setDate(date.getDate() - daysAgo);
    return date;
  };
  switch (since) {
    case 'last': {
      const last = accounts.get(account)?.mail?.last_fetch;
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
      const from = mailParseDateInput(custom.from);
      const to = mailParseDateInput(custom.to);
      if (to) to.setDate(to.getDate() + 1); // the end day is included
      return [from, to];
    }
    default: return [null, null];
  }
}

// A local date as an <input type="date"> value (yyyy-mm-dd), and back.
function mailDateInput(date) {
  return date ? localDay(new Date(date)) : '';
}

function mailParseDateInput(value) {
  const [y, m, d] = (value ?? '').split('-').map(Number);
  return y && m && d ? new Date(y, m - 1, d) : null;
}

// The filter changed: debounce, then search (Auto-update) or flag the list as stale (Manual).
function mailFilterChanged() {
  clearTimeout(mail.timer);
  mail.timer = setTimeout(() => {
    mail.timer = null;
    if (mail.auto) mailSearch();
  }, MAIL_DEBOUNCE_MS);
  scheduleRenderMailMatches();
}

// Search with the current filter; a newer search or page supersedes it.
function mailSearch() {
  clearTimeout(mail.timer);
  mail.timer = null;
  const m = mail;
  const filter = mailHostFilter();
  m.listFilter = JSON.stringify(filter);
  m.searching = true;
  m.error = null;
  rpcCall('mail_search', { account: m.account, filter }, { channel: 'mail-preview' }).then(reply => {
    if (mail === m) onMailMatches(reply);
  }, e => mailPreviewFailed(m, e));
  scheduleRenderMailMatches();
}

// Whether the list was read with another filter than the current one.
function mailIsStale() {
  return mail.matches !== null && (mail.timer !== null || mail.listFilter !== JSON.stringify(mailHostFilter()));
}

// ----------------------------------------------------------------------------
// selection, paging, fetching

function mailSelect(which) {
  const ids = mail.matches?.filter(m => which === 'all' || (which === 'new' && !m.in_vault)) ?? [];
  mail.selected = new Set(ids.map(m => m.id));
  scheduleRenderMailMatches();
}

function mailToggle(id) {
  if (mail.selected.has(id)) mail.selected.delete(id);
  else mail.selected.add(id);
  scheduleRenderMailMatches();
}

function mailPageCount() {
  return Math.max(1, Math.ceil((mail.matches?.length ?? 0) / MAIL_PAGE_SIZE));
}

function mailShowPage(page) {
  mail.page = Math.max(0, Math.min(page, mailPageCount() - 1));
  const start = mail.page * MAIL_PAGE_SIZE;
  const missing = mail.matches.slice(start, start + MAIL_PAGE_SIZE)
    .map(m => m.id)
    .filter(id => !mail.headers.has(id));
  if (missing.length) {
    const m = mail;
    m.searching = true;
    rpcCall('mail_headers', { account: m.account, ids: missing }, { channel: 'mail-preview' }).then(({ headers }) => {
      if (mail === m) onMailHeaders(headers);
    }, e => mailPreviewFailed(m, e));
  }
  scheduleRenderMailMatches();
}

// The selected mails already in the vault: their bodies get replaced.
function mailReplacing() {
  return mail.matches?.filter(m => m.in_vault && mail.selected.has(m.id)) ?? [];
}

// Whether Fetch can run: a list to fetch from, not about to be replaced by
// a pending Auto-update search, and an analysis that is off or has a
// working pick.
function mailCanFetch() {
  return mail.matches !== null && mail.selected.size > 0 && !mail.fetching
    && !mail.searching && !(mail.auto && mail.timer !== null)
    && !mailAnalysisBlocked(mail.analysis);
}

// Fetch the selection; replacing bodies takes a second click on Fetch
// (`renderMailFooter`). The account remembers the filter the list was read
// with.
function mailFetch() {
  if (!mail || !mailCanFetch()) return;
  const replace = mailReplacing().map(m => m.id);
  const m = mail;
  m.fetchError = null;
  m.fetching = true;
  const ids = m.matches.filter(match => m.selected.has(match.id)).map(match => match.id);
  const filter = m.listFilter ? JSON.parse(m.listFilter) : mailHostFilter();
  const args = { account: m.account, ids, replace, filter, auto_update: m.auto, analysis: m.analysis };
  // Once the fetch runs, the page closes; the job card shows it.
  rpcCall('mail_fetch', args).then(({ job }) => {
    mailFetchJobs.set(job, args.account);
    if (mail !== m) return;
    m.fetching = false;
    closeMailPage();
  }, e => {
    if (mail !== m) return;
    m.fetching = false;
    m.fetchError = e.message;
    scheduleRenderMailMatches();
  });
  scheduleRenderMailMatches();
}

// ----------------------------------------------------------------------------
// rendering: the page, built once per opening

function renderMailPage() {
  const els = mail.els;
  const page = accEl('div', 'page mail');

  const head = accEl('div', 'page-head');
  els.subtitle = accEl('span', 'meta');
  head.append(accEl('div', 'page-title', 'Fetch mail'), els.subtitle);
  page.append(head, accEl('div', 'page-text',
    'Only headers are read to count matches. Bodies are downloaded and converted when you press Fetch.'));

  const grid = accEl('div', 'mail__grid');
  const form = accEl('div', 'mail__form');
  const section = accEl('div', 'section-head', '1 \u00b7 Filter');
  section.append(accEl('span', 'section-head__note', 'all conditions must match'));
  const filter = accEl('div', 'section-body');
  filter.append(renderMailFilterForm(mail.form));
  form.append(section, filter);

  els.analysis = accEl('div');
  form.append(els.analysis);

  els.matches = accEl('div', 'mail__matches');
  grid.append(form, els.matches);
  page.append(grid);
  $('vault-area-body').replaceChildren(page);

  renderMailAnalysisSection();
  renderMailSubtitle();
  renderMailMatches();
}

function renderMailSubtitle() {
  const account = accounts.get(mail.account);
  const last = account?.mail?.last_fetch;
  mail.els.subtitle.textContent = [
    'Gmail', account?.email ?? 'Google account',
    last ? `last fetch ${shortMonthDate(new Date(last), true)}` : 'never fetched',
  ].join(' \u00b7 ');
}

// ----------------------------------------------------------------------------
// the filter form, of Fetch mail and of an account's auto-sync
//
// `form`: { account (whose labels the "+" menu offers), filter (edited in
// place), custom ({ from, to } of a custom date range), text (the text
// inputs as typed, by key, so a rebuilt form shows them as they were),
// withDate (the Date row), focusKey (prefix of the inputs'
// data-focus-key), onChange (called after every edit), els (filled in
// here) }. The rows' chips and the date re-render in place, so the inputs
// keep focus while typing.

function renderMailFilterForm(form) {
  const els = form.els;
  const box = accEl('div', 'mail__filter');
  const rows = accEl('div', 'form-grid');
  const row = (key, field) => {
    rows.append(accEl('span', 'label', key), field);
    return field;
  };

  els.labels = row('Labels', accEl('div', 'form-field'));

  const only = row('Only', accEl('div', 'form-field'));
  for (const [key, name] of MAIL_ONLY) {
    const toggle = accEl('button', 'mail__toggle', name);
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
  textRow('from', 'From', 'address or domain, several separated by commas', true);
  textRow('to', 'To', 'anyone', true);
  textRow('words', 'Words', 'in subject or body', false);

  if (form.withDate) els.date = row('Date', accEl('div', 'form-field form-field--wrap'));
  box.append(rows);

  renderMailLabels(form);
  renderMailDate(form);
  return box;
}

// Label chips (click to remove), "+" for the account's labels, any/all.
function renderMailLabels(form) {
  const field = form.els.labels;
  field.replaceChildren();
  const { labels } = form.filter;
  for (const name of labels) {
    const chip = accEl('button', 'chip chip--action', `${name} \u00d7`);
    chip.type = 'button';
    chip.title = 'Remove';
    chip.addEventListener('click', () => {
      form.filter.labels = labels.filter(l => l !== name);
      renderMailLabels(form);
      form.onChange();
    });
    field.append(chip);
  }
  const add = accEl('button', 'mail__add', labels.length ? '+' : '+ label');
  add.type = 'button';
  add.addEventListener('click', ev => {
    ev.stopPropagation();
    const known = mailLabels(form.account);
    const items = known.labels === null
      ? [{ label: known.error ? `Labels unavailable: ${known.error}` : 'Loading labels\u2026', dim: true }]
      : known.labels.filter(l => !labels.includes(l.name)).map(l => ({
        label: l.name,
        run: () => {
          form.filter.labels = [...form.filter.labels, l.name];
          renderMailLabels(form);
          form.onChange();
        },
      }));
    showMenu(add, items.length ? items : [{ label: 'No more labels', dim: true }]);
  });
  field.append(add);

  if (labels.length > 1) {
    field.append(accSeg([[false, 'any of'], [true, 'all of']], !!form.filter.every, every => {
      form.filter.every = every;
      renderMailLabels(form);
      form.onChange();
    }));
  }
}

// The date preset menu, and the range inputs of a custom range; nothing
// without the Date row.
function renderMailDate(form) {
  const field = form.els.date;
  if (!field) return;
  field.replaceChildren();
  const hint = key => {
    const [after] = key === 'custom' ? [] : mailDateBounds(key, form.account, form.custom);
    return after ? shortMonthDate(after) : '';
  };
  const lastFetch = accounts.get(form.account)?.mail?.last_fetch;

  const button = accEl('button', 'mail__date');
  button.type = 'button';
  const [, name] = MAIL_DATE_PRESETS.find(([key]) => key === (form.filter.since ?? null)) ?? MAIL_DATE_PRESETS[0];
  button.append(accEl('span', '', name), accEl('span', 'meta mail__date-hint', hint(form.filter.since)),
    iconEl('#i-chevron-down', 'mail__date-caret'));
  button.addEventListener('click', ev => {
    ev.stopPropagation();
    showMenu(button, MAIL_DATE_PRESETS.map(([key, label]) => {
      const date = hint(key);
      const unavailable = key === 'last' && !lastFetch;
      return {
        label,
        hint: date,
        checked: key === (form.filter.since ?? null),
        dim: unavailable,
        run: unavailable ? undefined : () => {
          form.filter.since = key;
          renderMailDate(form);
          form.onChange();
        },
      };
    }));
  });
  field.append(button);

  if (form.filter.since === 'custom') {
    const range = accEl('span', 'mail__range');
    for (const [key, name] of [['from', 'From'], ['to', 'To']]) {
      const input = accEl('input', 'input');
      input.type = 'date';
      input.value = form.custom[key];
      input.addEventListener('change', () => {
        form.custom[key] = input.value;
        form.onChange();
      });
      range.append(accEl('span', 'label', name), input);
    }
    range.append(accEl('span', 'meta', 'both optional'));
    field.append(range);
  }
}

// ----------------------------------------------------------------------------
// auto-sync (mail.md#mail-auto-sync): an account's own filter, apart from
// the one Fetch mail remembers, set up under the Mail row of its pane
// (features/workspace/accounts.js) with the form above; the host fetches what is new every 15
// minutes (`set_mail_sync`)

const mailSyncForms = new Map(); // account id -> the form of its auto-sync filter

// A vault opened: its accounts' forms start from what they keep.
rpcOn('accounts', () => mailSyncForms.clear());

// The Mail row's auto-sync: a check, and while it is on, the filter form of
// what it fetches; `rerender`: the Accounts page's scheduler, for a
// refused change's note (`mailSetSync`).
export function renderMailSync(account, rerender) {
  const sync = account.mail?.auto_sync;
  const on = !!sync?.enabled;
  const form = mailSyncForm(account);
  form.rerender = rerender;
  const box = accEl('div', 'accounts__sub');
  const head = accEl('div', 'accounts__sync');
  const check = accEl('button', 'check', on ? '\u2713' : '');
  check.type = 'button';
  check.setAttribute('aria-pressed', String(on));
  check.title = on ? 'Stop the auto-sync' : 'Fetch new mail on its own';
  check.addEventListener('click', () => mailSetSync(account.id, { enabled: !on }));
  head.append(check, accEl('span', 'subtitle', 'Auto-sync'), accEl('span', 'note', on
    ? `New mail that matches the filter below, every 15 minutes, since ${pageDate(sync.since, true)}.`
    : 'Fetch new mail that matches a filter every 15 minutes.'));
  box.append(head);
  if (on) {
    box.append(renderMailFilterForm(form));
    box.append(renderMailSyncAnalysis(account.id, { ...MAIL_ANALYSIS_OFF, ...sync.analysis }));
  }
  if (form.error) box.append(accEl('div', 'error-note', form.error));
  return box;
}

// The form of `account`'s auto-sync filter: made once from the filter it
// keeps, then edited here, so a re-render of the pane keeps what is typed.
// An edit is saved once typing pauses.
function mailSyncForm(account) {
  let form = mailSyncForms.get(account.id);
  if (form) return form;
  const saved = account.mail?.auto_sync?.filter ?? {};
  form = {
    account: account.id,
    filter: { labels: [], from: [], to: [], words: '', ...structuredClone(saved) },
    custom: { from: '', to: '' },
    text: {},
    withDate: false,
    focusKey: `mail-sync-${account.id}`,
    timer: null,
    error: null,
    els: {},
  };
  form.onChange = () => {
    clearTimeout(form.timer);
    form.timer = setTimeout(() => {
      form.timer = null;
      mailSetSync(form.account, { filter: form.filter });
    }, MAIL_DEBOUNCE_MS);
  };
  mailSyncForms.set(account.id, form);
  mailLabels(account.id); // for the "+" menu
  return form;
}

// Change `account`'s auto-sync; the `account` event shows it, a refusal
// shows under it.
function mailSetSync(account, change) {
  const form = mailSyncForms.get(account);
  rpcCall('set_mail_sync', { account, ...change }).then(() => {
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
// the analysis of new mail (mail.md#analysis), of Fetch mail and of an
// account's auto-sync: a level, and the model it runs with

const MAIL_ANALYSIS_OFF = { level: 'off', model: null };

// `analysis` ({ level, model }) as a seg of its levels and a body: with the
// analysis on, the picker of its model, and what keeps the pick from working
// (`aiMissingHint`; not for a plain "pick a model", which the picker says
// itself). `onChange` gets the analysis as changed.
function renderMailAnalysis(analysis, onChange) {
  const seg = accSeg([['off', 'Off'], ['lite', 'Lite']], analysis.level,
    level => onChange({ ...analysis, level }));
  const body = accEl('div', 'page-text mail__text');
  if (analysis.level === 'off') {
    body.append('New mails keep their subject as title; nothing is sent to an AI.');
    return { seg, body };
  }
  const picker = aiModelPicker(analysis.model, model => onChange({ ...analysis, model }));
  body.append('Sends each new mail to ', picker,
    ' for its kind, a clean title, and labels and projects from this vault.');
  const ai = aiModelOf(analysis.model);
  if (ai.problem && !(ai.problem === 'pick' && aiAnyOffered())) {
    body.append(notice('hint', ...aiMissingHint(ai, 'the mail analysis')));
  }
  return { seg, body };
}

// Whether `analysis` is on but its pick wouldn't work (the host refuses a
// fetch then).
function mailAnalysisBlocked(analysis) {
  return analysis.level !== 'off' && !!aiModelOf(analysis.model).problem;
}

// Fetch mail's third section, Analysis: built again on its changes, and
// when the accounts change (the pick's problems).
function renderMailAnalysisSection() {
  const { seg, body } = renderMailAnalysis(mail.analysis, analysis => {
    mail.analysis = analysis;
    renderMailAnalysisSection();
    scheduleRenderMailMatches();
  });
  const head = accEl('div', 'section-head', '2 \u00b7 Analysis');
  head.append(seg);
  mail.els.analysis.replaceChildren(head, body);
}

// The auto-sync's analysis, under its filter; saved at once.
function renderMailSyncAnalysis(account, analysis) {
  const { seg, body } = renderMailAnalysis(analysis, next => mailSetSync(account, { analysis: next }));
  const grid = accEl('div', 'form-grid');
  const field = accEl('div', 'form-field');
  field.append(seg);
  grid.append(accEl('span', 'label', 'Analysis'), field);
  const box = accEl('div', 'mail__filter');
  box.append(grid, body);
  return box;
}

// ----------------------------------------------------------------------------
// rendering: the matches pane

const scheduleRenderMailMatches = oncePerFrame(() => {
  if (mail?.els.matches) renderMailMatches();
});

function renderMailMatches() {
  const box = accEl('div', 'mail__box');

  const head = accEl('div', 'section-head', 'Matches');
  head.append(accSeg([[true, 'Auto-update'], [false, 'Manual']], mail.auto, auto => {
    mail.auto = auto;
    if (auto && mailIsStale()) mailSearch();
    scheduleRenderMailMatches();
  }));
  if (!mail.auto) {
    const refresh = accEl('button', 'icon-btn');
    refresh.append(iconEl('#i-arrow-sync'));
    refresh.type = 'button';
    refresh.title = 'Update matches';
    refresh.disabled = mail.searching;
    refresh.addEventListener('click', mailSearch);
    head.append(refresh);
  }
  box.append(head);

  if (!mail.auto && mailIsStale()) {
    const stale = accEl('div', 'mail__stale', 'The filter changed since this list was read.');
    const update = accEl('button', 'btn btn--primary', 'Update');
    update.type = 'button';
    update.addEventListener('click', mailSearch);
    stale.append(update);
    box.append(stale);
  }
  if (mail.error) box.append(accEl('div', 'error-note', mail.error));

  const matches = mail.matches;
  if (matches === null) {
    box.append(accEl('div', 'page-text mail__empty',
      mail.searching ? 'Reading matches\u2026' : 'Press Update to read the matches.'));
    mail.els.matches.replaceChildren(box, renderMailFooter());
    return;
  }

  const inVault = matches.filter(m => m.in_vault).length;
  const count = accEl('div', 'mail__count');
  if (mail.searching) count.classList.add('mail__count--busy');
  count.append(
    accEl('span', 'subtitle', mail.truncated ? `${matches.length}+ matches` : plural(matches.length, 'match', 'matches')),
    accEl('span', 'meta',
      `${inVault} in the vault \u00b7 ${matches.length - inVault} new \u00b7 ${mail.selected.size} selected`),
    accEl('span', 'meta push', mail.searching ? 'reading\u2026' : mail.auto ? 'updates as you edit' : ''));
  box.append(count);
  if (mail.truncated) {
    box.append(accEl('div', 'note mail__note', `Only the newest ${matches.length} are listed; narrow the filter to see the rest.`));
  }

  const select = accEl('div', 'select-bar');
  for (const [which, name] of [['new', 'New only'], ['all', 'All'], ['none', 'None']]) {
    const button = accEl('button', '', name);
    button.type = 'button';
    button.addEventListener('click', () => mailSelect(which));
    select.append(button);
  }
  select.append(accEl('span', 'select-bar__note', 'in the vault are unticked; ticking one replaces its body'));
  box.append(select);

  const list = accEl('div', 'mail__list');
  const start = mail.page * MAIL_PAGE_SIZE;
  for (const match of matches.slice(start, start + MAIL_PAGE_SIZE)) {
    list.append(renderMailRow(match));
  }
  if (!matches.length) list.append(accEl('div', 'page-text mail__empty', 'No mail matches this filter.'));
  box.append(list);

  if (matches.length > MAIL_PAGE_SIZE) box.append(renderMailPager());
  mail.els.matches.replaceChildren(box, renderMailFooter());
}

function renderMailRow(match) {
  const header = mail.headers.get(match.id);
  const selected = mail.selected.has(match.id);
  const row = accEl('div', 'list-row mail__row');
  if (!header) row.classList.add('mail__row--loading');
  if (selected && match.in_vault) row.classList.add('mail__row--replace');

  const box = accEl('span', 'check', selected ? '\u2713' : '');
  box.setAttribute('aria-checked', String(selected));
  const subject = accEl('span', 'row-title', header ? (header.subject || 'No subject') : '\u2026');
  const date = accEl('span', 'meta mail__date-cell', header ? shortMonthDate(new Date(header.date)) : '');
  const from = accEl('span', 'meta mail__from', header ? mailSender(header.from) : '');
  const status = accEl('span', 'label mail__status',
    match.in_vault ? (selected ? 'replace' : 'in vault') : 'new');
  row.append(box, subject, date, from, status);
  row.title = header?.from ?? '';
  row.addEventListener('click', () => mailToggle(match.id));
  return row;
}

// "Hausverwaltung Krause" of `"Hausverwaltung Krause" <info@\u2026>`, else the address.
function mailSender(from) {
  const name = from.replace(/<[^>]*>/, '').replace(/"/g, '').trim();
  return name || from.replace(/[<>]/g, '').trim();
}

function renderMailPager() {
  const pager = accEl('div', 'meta mail__pager');
  const pages = mailPageCount();
  const start = mail.page * MAIL_PAGE_SIZE;
  const end = Math.min(start + MAIL_PAGE_SIZE, mail.matches.length);
  const step = (label, title, page, disabled) => {
    const button = accEl('button', 'btn', label);
    button.type = 'button';
    button.title = title;
    button.disabled = disabled || mail.searching;
    button.addEventListener('click', () => mailShowPage(page));
    return button;
  };
  pager.append(
    step('\u2039', 'Previous page', mail.page - 1, mail.page === 0),
    accEl('span', '', `${start + 1}\u2013${end} of ${mail.matches.length}`),
    step('\u203a', 'Next page', mail.page + 1, mail.page >= pages - 1));
  return pager;
}

function renderMailFooter() {
  const footer = accEl('div', 'mail__footer');
  const n = mail.selected.size;
  const replace = mailReplacing().map(m => m.id);
  const replacing = replace.length;
  const label = mail.fetching ? 'Starting\u2026' : `Fetch ${n} \u00b7 plain text`;
  const fetch = accEl('button', 'btn btn--primary', label);
  fetch.type = 'button';
  fetch.title = 'Ctrl+Enter';
  fetch.disabled = !mailCanFetch();
  // Replacing bodies takes a second click; another selection to replace
  // asks again (the key).
  if (replacing) {
    fetch.classList.add('btn--danger');
    armedButton(fetch, `Click again to replace ${plural(replacing, 'body', 'bodies')}`, mailFetch,
      `mail_fetch ${replace.join(' ')}`);
  } else {
    fetch.addEventListener('click', mailFetch);
  }
  mail.els.fetch = fetch;

  const cancel = accEl('button', 'btn btn--ghost', 'Cancel');
  cancel.type = 'button';
  cancel.title = 'Esc';
  cancel.addEventListener('click', closeMailPage);
  footer.append(fetch, cancel);

  if (replacing) {
    footer.append(accEl('div', 'note mail__warning',
      `${replacing} selected ${replacing === 1 ? 'mail is' : 'mails are'} already in the vault: fetching replaces ${replacing === 1 ? 'its body' : 'their bodies'}, and edits to ${replacing === 1 ? 'it' : 'them'} are lost. Title, projects, labels and threads stay.`));
  }
  if (mailAnalysisBlocked(mail.analysis)) {
    footer.append(accEl('div', 'note mail__warning',
      'The analysis is on: pick a model that works for it, or turn it off.'));
  }
  if (mail.fetchError) footer.append(accEl('div', 'error-note', mail.fetchError));
  return footer;
}
