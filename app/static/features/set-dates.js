// Set the dates (ui.md#set-the-dates): the days typed into a trip's Days fact while
// it has a sketch aren't written at once. A page in the vault area shows
// what they make of the sketch (`setDatesProposal`, model/trip.js): its
// journeys and its stays, planned, and a day off for each traveler with a work
// calendar, each a row to tick with why and when, by part. Apply
// (Ctrl+Enter) creates the ticked events in the trip (`create_entity`),
// then writes the days and drops the sketch (`set_project`); nothing is
// written before it. Esc or Cancel leaves the days as they were.

import { accEl } from '../core/dom.js';
import { DAY_MONTH, WEEKDAY, addDays, daysBetween, parseDate } from '../core/format.js';
import { newGroup, rpcCall } from '../core/rpc.js';
import { t } from '../core/text.js';
import { projectCalendar, withFact } from '../model/blueprint.js';
import { writableCalendar } from '../model/calendar.js';
import { projects } from '../model/entities.js';
import { store } from '../model/state.js';
import { setProjectLocal } from '../model/structure.js';
import { setDatesProposal } from '../model/trip.js';
import { docIcon, iconEl } from '../ui/icons.js';
import { closeVaultArea, openVaultOp, registerVaultPage, replaceVaultPage } from '../ui/page.js';

// The page's state while it is open: the trip, the days, the proposal, the
// rows ticked and those already made (a retry after a failure skips them).
let setDates = null;

// The parts of a proposal: its key, its head, and where its events go.
const PARTS = [
  ['journeys', t('bp.rule.journeys'), t('set_dates.journeys_note'), n => t('set_dates.made_journeys', { n })],
  ['stays', t('set_dates.stays'), t('set_dates.stays_note'), n => t('set_dates.made_stays', { n })],
  ['off', t('set_dates.days_off'), t('set_dates.off_note'), n => t('set_dates.made_off', { n })],
];

const rowsOf = proposal => PARTS.flatMap(([part]) => proposal[part]);
const dayText = day => `${WEEKDAY.format(day)} ${DAY_MONTH.format(day)}`;

// Open Set the dates for the days `days` (`{ from, to }`) of the trip `id`.
export function openSetDates(id, days) {
  const project = projects.get(id)?.project;
  if (!project) return;
  const proposal = setDatesProposal(id, project, days, writableCalendar(projectCalendar(id)));
  // the op is published before the area expands, which the listener below
  // takes for leaving the page: the state comes after
  openVaultOp('set-dates', t('set_dates.head'));
  setDates = { id, days, proposal, ticked: new Set(rowsOf(proposal).map(row => row.key)), made: new Set(), busy: false, error: null };
  render();
}

// Leaving the page (another op, or the area closed) drops it.
store.subscribe([state => state.vault_op, state => state.vault_expanded], ([vault_op, vault_expanded]) => {
  if (setDates && !(vault_op === 'set-dates' && vault_expanded)) setDates = null;
});

registerVaultPage('set-dates', { close: closeVaultArea, submit: () => apply() });

// ----------------------------------------------------------------------------
function render() {
  if (!setDates) return;
  const { id, days, proposal } = setDates;
  const sheet = accEl('section', 'new__sheet');
  const head = accEl('h2', 'pane-title new__kind');
  head.append(iconEl('#i-calendar'), t('set_dates.head'));
  const from = parseDate(days.from);
  const to = parseDate(days.to);
  const { sketched, nights } = proposal;
  const differs = sketched && (nights < sketched.min || nights > sketched.max);
  const line = accEl('p', 'set-dates__days');
  line.append(accEl('span', 'label', projects.get(id)?.name ?? t('set_dates.the_trip')), `${dayText(from)} \u2013 ${dayText(to)}`,
    accEl('span', 'meta', [t('trip.nights', { n: nights }), differs ? t('trip.sketch_said', { sketched: rangeText(sketched) }) : null]
      .filter(Boolean).join(' \u00b7 ')));
  sheet.append(head, line);
  for (const [part, label, note] of PARTS) {
    const rows = proposal[part];
    if (rows.length) sheet.append(...partEls(label, note, rows));
  }
  if (!rowsOf(proposal).length) sheet.append(accEl('p', 'note', t('set_dates.no_stops')));
  if (setDates.error) sheet.append(accEl('div', 'error-note new__error', setDates.error));
  sheet.append(footEl());
  const page = accEl('div', 'page');
  page.append(sheet);
  replaceVaultPage(page);
}

function rangeText({ min, max, about }) {
  return min !== max ? `${min}\u2013${max}` : about ? t('trip.about_n', { n: min }) : String(min);
}

// A part's head (its rows ticked, a toggle for all of them) and its rows.
function partEls(label, note, rows) {
  const open = rows.filter(row => !setDates.made.has(row.key));
  const ticked = rows.filter(row => setDates.ticked.has(row.key)).length;
  const head = accEl('div', 'proposal-head');
  const all = open.every(row => setDates.ticked.has(row.key));
  const toggle = accEl('button', 'tool label', all ? t('email.select.none') : t('email.select.all'));
  toggle.type = 'button';
  toggle.disabled = setDates.busy || !open.length;
  toggle.addEventListener('click', () => {
    for (const row of open) {
      if (all) setDates.ticked.delete(row.key);
      else setDates.ticked.add(row.key);
    }
    render();
  });
  head.append(accEl('span', 'label', label), accEl('span', 'meta', `${t('count.of_total', { at: ticked, n: rows.length })} \u00b7 ${note}`), toggle);
  const list = accEl('ul', 'proposal');
  list.append(...rows.map(rowEl));
  return [head, list];
}

function rowEl(row) {
  const item = accEl('li');
  const label = accEl('label', 'proposal__row');
  const box = accEl('input');
  box.type = 'checkbox';
  box.checked = setDates.ticked.has(row.key);
  box.disabled = setDates.busy || setDates.made.has(row.key);
  box.dataset.focusKey = `set-dates:${row.key}`;
  box.addEventListener('change', () => {
    if (box.checked) setDates.ticked.add(row.key);
    else setDates.ticked.delete(row.key);
    render();
  });
  const text = accEl('span', 'proposal__text');
  text.append(accEl('span', '', row.title), accEl('span', 'proposal__why', setDates.made.has(row.key) ? t('set_dates.made') : row.why));
  const end = accEl('span', 'proposal__end');
  const last = row.event.type === 'stay' ? row.to : addDays(row.to, -1);
  end.append(accEl('span', 'meta', daysBetween(row.from, last) > 0 ? `${dayText(row.from)} \u2013 ${dayText(last)}` : dayText(row.from)));
  label.append(box, iconEl(docIcon({ kind: 'event', event: row.event })), text, end);
  item.append(label);
  return item;
}

// Apply, Cancel, and what Apply writes.
function footEl() {
  const foot = accEl('div', 'new__foot');
  const count = rowsOf(setDates.proposal).filter(row => setDates.ticked.has(row.key) && !setDates.made.has(row.key)).length;
  const applyBtn = accEl('button', 'btn btn--primary', count ? t('set_dates.apply_n', { n: count }) : t('set_dates.apply'));
  applyBtn.type = 'button';
  applyBtn.disabled = setDates.busy;
  applyBtn.addEventListener('click', () => apply());
  const cancel = accEl('button', 'btn btn--ghost', t('field.cancel'));
  cancel.type = 'button';
  cancel.addEventListener('click', closeVaultArea);
  const made = PARTS.map(([part, , , counted]) => {
    const n = setDates.proposal[part].filter(row => setDates.ticked.has(row.key) && !setDates.made.has(row.key)).length;
    return n ? counted(n) : null;
  }).filter(Boolean);
  foot.append(applyBtn, cancel, accEl('span', 'meta', made.length ? t('set_dates.writes', { what: made.join(', ') }) : t('set_dates.writes_days')));
  return foot;
}

// ----------------------------------------------------------------------------
// The ticked rows' events, one after the other, each in the trip; then the
// days, and the sketch gone; one step to undo. A failure stops there: what
// was made stays made, and Apply again makes the rest, a step of its own.
async function apply() {
  const state = setDates;
  if (!state || state.busy) return;
  state.busy = true;
  state.error = null;
  render();
  const step = newGroup('set-dates');
  try {
    for (const row of rowsOf(state.proposal)) {
      if (!state.ticked.has(row.key) || state.made.has(row.key)) continue;
      await rpcCall('create_entity', { roles: { event: row.event }, title: row.title, projects: [state.id], labels: [] }, { step });
      state.made.add(row.key);
    }
  } catch (e) {
    state.busy = false;
    state.error = e.message;
    if (setDates === state) render();
    return;
  }
  const project = projects.get(state.id)?.project;
  if (project) {
    const next = withFact(project, 'days', state.days);
    delete next.sketch;
    setProjectLocal(state.id, next, step);
  }
  if (setDates === state) closeVaultArea();
}
