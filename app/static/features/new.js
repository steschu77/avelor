// New: a menu of the types the app makes -- at the sidebar's New item,
// the + of "Created Here", or Ctrl+N -- then the New page of the type
// picked, rendered into the expanded vault area while its op is "new"; and the Task tab of the panel's context column, a new
// task for what the panel shows (a card's Task button opens it). Create
// sends `create_entity` (api.md#structure): the host writes the
// file into its type's folder and pushes the entity. The fields follow the
// vault's roles (vault.md#well-known-roles). An event says what part of a
// trip it is, if it is one (Type), planned or booked, and its calendar, which
// a trip's prefills when the sidebar selects one; its times are each in a
// zone, and a trip's part asks for its own fields, each place's time in
// that place's zone (panel.md#trip-fields).
// A draft's form is built once, so its inputs keep focus; only its chips,
// the list's items, the notes and the buttons are built again.
// "+ Task from this" of an assistant reply runs the `task.with` action
// (`openTaskWith`).

import { $, accEl, titled } from '../core/dom.js';
import { rpcCall, rpcOn } from '../core/rpc.js';
import { isoInstant, localDay, localTime, parseDate, relativeDay, zonedInstant } from '../core/format.js';
import { registerAction, shortcutText } from '../core/actions.js';
import { t } from '../core/text.js';
import { store, vaultOpen } from '../model/state.js';
import { currentDraft, openDraft, showDraft } from '../model/drafts.js';
import { entities, projectsFor } from '../model/entities.js';
import { eventScheduleLines } from '../model/schedule.js';
import { LOCAL_ZONE, STATES, TRIP_TYPES, MEANS, byNights, placeZone } from '../model/trip.js';
import { MEANS_ICONS, docIcon, iconEl } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import { closeVaultArea, registerVaultPage } from '../ui/page.js';
import { accSeg } from '../ui/segmented.js';
import { calendarMenu } from './panel/event-fields.js';
import {
  contextTarget, createdNote, registerContextTab, selectContextTab,
  shownContextTab,
} from './context.js';
import { TASK_STATUSES } from './panel/details.js';
import {
  addChip, linkChip, linkTarget, projectChips, labelChips,
} from './panel/panel.js';
import { EVENT_STATUSES, zoneButton } from './panel/event-fields.js';
import { eventScheduleField } from './panel/schedule-field.js';
import { placeInput } from './panel/trip-fields.js';
import { watchFormGrid } from './workspace/folders.js';

// The types New makes: their derived kind, what Go to calls it (`command`,
// `also`), the folder the host writes them into (shown only; av-vault's
// `schema::layout` decides), and how their title is asked for; also the
// draft card's (features/panel/draft.js).
export const NEW_TYPES = [
  {
    kind: 'document', name: t('kind.document'), command: t('new.document'), also: t('new.document_also'), folder: 'Documents', label: t('new.label.title'), hint: t('new.label.title'),
    missing: t('new.enter_title'), create: t('new.create.document'), createTitle: t('new.create_title.document'),
  },
  {
    kind: 'task', name: t('kind.task'), command: t('new.task'), folder: 'Tasks', label: t('new.label.title'), hint: t('new.hint.task'),
    missing: t('new.enter_title'), create: t('new.create.task'), createTitle: t('new.create_title.task'),
  },
  {
    kind: 'event', name: t('kind.event'), command: t('day.new_event'), folder: 'Calendar\\Created Here', label: t('new.label.title'), hint: t('new.label.title'),
    missing: t('new.enter_title'), create: t('new.create.event'), createTitle: t('new.create_title.event'),
  },
  {
    kind: 'contact', name: t('kind.contact'), command: t('new.contact'), also: t('new.contact_also'), folder: 'Contacts', label: t('docs.name'), hint: t('docs.name'),
    missing: t('new.enter_name'), create: t('new.create.contact'), createTitle: t('new.create_title.contact'),
  },
  {
    kind: 'list', name: t('kind.list'), command: t('new.list'), folder: 'Lists', label: t('new.label.title'), hint: t('new.hint.list'),
    missing: t('new.enter_title'), create: t('new.create.list'), createTitle: t('new.create_title.list'),
  },
  {
    kind: 'record', name: t('kind.record'), command: t('new.record'), folder: 'Records', label: t('new.label.title'), hint: t('new.label.title'),
    missing: t('new.enter_title'), create: t('new.create.record'), createTitle: t('new.create_title.record'),
  },
];
const NEW_CONTACT_TYPES = [['person', t('new.contact.person')], ['org', t('new.contact.org')]];
// An event's Type: a plain one, or a part of a trip.
const NEW_EVENT_TYPES = [['', t('kind.event')], ...TRIP_TYPES];

function newTypeIcon(type) {
  return docIcon({ kind: type.kind, type: type.kind === 'document' ? 'note' : null });
}

// The draft's icon: its type's, an event's by what part of a trip it is.
function newDraftIcon(d) {
  if (d.type.kind !== 'event') return newTypeIcon(d.type);
  return docIcon({ kind: 'event', event: { type: d.eventType, by: d.by } });
}

// A new draft of `kind` (one of NEW_TYPES). `page`: it is the New page's,
// which shows the folder and can be cancelled (else the Task tab's);
// `linked`: the entity a task is made for; `projects`: the projects it
// starts in; `scope`: the project selected in the sidebar, noted as such
// while it is the only one; `onCreated(id)`: what its host does once it is
// created.
function newDraft(kind, { page = false, linked = null, projects = [], scope = null, onCreated }) {
  const today = localDay(new Date());
  return {
    type: NEW_TYPES.find(t => t.kind === kind),
    page,
    title: '',
    day: today, // a document's date
    status: 'todo', // a task's
    due: '',
    start: today, // an event's
    startTime: '09:00',
    end: today,
    endTime: '10:00',
    location: '',
    eventType: '', // what part of a trip it is, if any
    schedule: { period: 'once', days: null, until: null, count: null, rest: [] }, // a plain event's
    eventStatus: 'confirmed', // planned (tentative) or booked
    calendar: null, // its calendar entity
    by: 'flight', // a journey's
    from: '',
    to: '',
    place: '', // a stay's, a layover's, an activity's
    booking: '',
    fromZone: LOCAL_ZONE, // the zones of its times: the start's (a plain event's and an activity's both)
    toZone: LOCAL_ZONE, // a journey's arrival's
    contactType: 'person', // a contact's
    email: '',
    phone: '',
    address: '',
    items: [''], // a list's
    projects: [...projects],
    scope,
    labels: [],
    linked, // the entity it is `linked-to` (a task's, in the Task tab)
    body: '',
    created: null, // { id, title } of the one made before, in the Task tab
    error: null, // why Create failed
    busy: false, // Create is under way
    onCreated,
    els: {}, // the parts built again: chips, linked, items, error, foot
  };
}

// ----------------------------------------------------------------------------
// the menu and the New page

let newPage = null; // the New page's draft

registerAction({
  id: 'new', group: 'new', title: t('new.menu'), icon: '#i-add', shortcut: 'Ctrl+N',
  when: vaultOpen,
  run: anchor => showNewMenu(anchor ?? $('new-item')),
});
for (const type of NEW_TYPES) {
  registerAction({
    id: `new.a_${type.kind}`, group: 'new', title: type.command, icon: newTypeIcon(type), also: type.also,
    when: vaultOpen,
    run: () => openDraft(type.kind),
  });
}

// New event on the day `day` (the day view's, features/panel/panel.js).
registerAction({
  id: 'new.event',
  when: vaultOpen,
  run: day => openDraft('event', { day: localDay(day) }),
});

// The types at `anchor`, each a draft card in the side panel
// (model/drafts.js); a kept draft first, shown again.
function showNewMenu(anchor) {
  const items = NEW_TYPES.map(type => ({
    label: type.name,
    icon: newTypeIcon(type),
    hint: type.folder.split('\\')[0],
    run: () => openDraft(type.kind),
  }));
  const kept = currentDraft();
  if (kept) items.unshift({ label: t('draft.back', { title: kept.title || t('untitled') }), icon: docIcon(kept), run: showDraft });
  showMenu(anchor, items, { head: t('new.head') });
}

// The New page's sheet: its head and the form; built again when an
// event's Type changes its fields.
function renderNewSheet(d) {
  const kindEl = accEl('h2', 'pane-title new__kind');
  d.els.icon = iconEl(newDraftIcon(d));
  kindEl.append(d.els.icon, d.type.name);
  d.els.sheet.replaceChildren(kindEl, ...renderNewForm(d));
  renderNewChips(d);
  renderNewFoot(d);
}

// Leaving the page (another op, or the area closed) drops its draft.
store.subscribe([state => state.vault_op, state => state.vault_expanded], ([vault_op, vault_expanded]) => {
  if (newPage && !(vault_op === 'new' && vault_expanded)) newPage = null;
});

registerVaultPage('new', {
  close: closeVaultArea,
  submit: () => {
    if (newPage) createNew(newPage);
  },
});

// ----------------------------------------------------------------------------
// the Task tab: a new task for what the panel shows

const taskTabEl = $('context-task');
const taskDrafts = new Map(); // what a task is for ('' for nothing) -> its draft
let taskShown = null; // the draft the tab shows

registerContextTab({ id: 'task', title: t('kind.task'), el: taskTabEl, show: focus => renderTaskTab(focus) });

registerAction({
  id: 'task.create', title: t('new.create_task_action'), shortcut: 'Ctrl+Enter',
  when: () => shownContextTab() === 'task' && !!taskShown && taskTabEl.contains(document.activeElement),
  run: () => createNew(taskShown),
});

// The draft for `target`, kept while the app runs: linked to it, in its
// projects (`projectsFor`). Once created, a fresh one takes its place,
// noting the one made.
function taskDraft(target) {
  let d = taskDrafts.get(target);
  if (d) return d;
  d = newDraft('task', {
    linked: target || null,
    ...projectsFor(target),
    onCreated: id => {
      taskDrafts.delete(target);
      taskDraft(target).created = { id, title: d.title.trim() };
      if (taskShown === d) renderTaskTab(true);
    },
  });
  taskDrafts.set(target, d);
  return d;
}

// Show the form of what the panel shows; the same draft's form stays as it
// is (and keeps the focus). `focus`: the title takes it.
function renderTaskTab(focus = false) {
  const d = taskDraft(contextTarget(store.getState()));
  if (d !== taskShown) {
    taskShown = d;
    taskTabEl.replaceChildren(...renderNewForm(d));
    renderNewChips(d);
    renderNewFoot(d);
  }
  if (focus) taskTabEl.querySelector('.new__title')?.focus({ preventScroll: true });
}

// An assistant reply's "+ Task from this" (features/assistant/assistant.js,
// the `task.with` action): the Task tab for
// what the panel shows, with `markdown` in its body, below what is typed
// there already; `personal` (a reply showing personal values) makes the
// task `masked`. The context column is open beside the chat already.
function openTaskWith(markdown, personal = false) {
  const d = taskDraft(contextTarget(store.getState()));
  d.body = d.body.trim() ? `${d.body.trimEnd()}\n\n${markdown}` : markdown;
  if (personal) d.classification = 'masked';
  taskShown = null; // the form is built again, with the body
  selectContextTab('task', true);
}
registerAction({ id: 'task.with', title: t('new.task_with'), run: openTaskWith });

store.subscribe(contextTarget, () => {
  if (shownContextTab() === 'task') renderTaskTab();
});

// Another vault opened (`forgetVault`): the drafts named the old vault's
// entities and projects.
rpcOn('vault', ({ state }) => {
  if (state !== 'indexing') return;
  taskDrafts.clear();
  if (shownContextTab() === 'task') renderTaskTab();
});

// ----------------------------------------------------------------------------
// creating

// Why the draft can't be created yet, or '' if it can.
function newBlocked(d) {
  if (!d.title.trim()) return d.type.missing;
  if (d.type.kind === 'event' && !d.start) return t('new.pick_start');
  if (d.type.kind === 'document' && !d.day) return t('new.pick_date');
  return '';
}

// The draft's role blocks, as the frontmatter has them (vault.md#well-known-roles):
// what is left empty is left out.
function newRoles(d) {
  switch (d.type.kind) {
    case 'document':
      return { document: { type: 'note' } };
    case 'task': {
      const task = { status: d.status };
      if (d.due) task.due = d.due;
      return { task };
    }
    case 'event':
      return { event: newEvent(d) };
    case 'contact': {
      const contact = { type: d.contactType };
      for (const key of ['email', 'phone', 'address']) {
        if (d[key].trim()) contact[key] = [d[key].trim()];
      }
      return { contact };
    }
    case 'list':
      return { list: { items: d.items.map(item => item.trim()).filter(Boolean) } };
    default:
      return { record: {} };
  }
}

// An event's block: its calendar and status; a stay's or a layover's
// days, the morning after the last night its end; one without a start time
// its days only, the day after the last its end (an all-day event, a planned
// journey); else its times, each in its zone (a trip's in its place's). A plain
// event's schedule, if it repeats. Then where it is, and the trip fields of
// its type.
function newEvent(d) {
  const type = d.eventType;
  const event = {};
  if (d.calendar) event.calendar = d.calendar;
  if (type || d.eventStatus === 'tentative') event.status = d.eventStatus;
  const nightly = byNights({ type });
  if (nightly || !d.startTime) {
    event.start = d.start;
    const last = nightly ? d.end : newNextDay(d.end || d.start);
    event.end = last > d.start ? last : newNextDay(d.start);
  } else {
    const zone = d.fromZone;
    const endZone = type === 'journey' ? d.toZone : zone;
    event.start = isoInstant(zonedInstant(d.start, d.startTime, zone));
    if (d.end) event.end = isoInstant(zonedInstant(d.end, d.endTime, endZone));
    event.tz = zone;
    if (endZone !== zone) event['end-tz'] = endZone;
  }
  if (!type && d.schedule.period !== 'once') {
    const allDay = !d.startTime;
    const zone = allDay ? null : d.fromZone;
    const start = allDay ? parseDate(d.start) : zonedInstant(d.start, d.startTime, zone);
    event.schedule = eventScheduleLines(d.schedule, start, zone, allDay);
  }
  if (d.location.trim()) event.location = d.location.trim();
  if (type) event.type = type;
  const keep = keys => keys.forEach(key => { if (d[key].trim()) event[key] = d[key].trim(); });
  if (type === 'journey') {
    event.by = d.by;
    keep(['from', 'to', 'booking']);
  } else if (type) {
    keep(['place']);
  }
  return event;
}

// Create the draft `d` (Create, Ctrl+Enter); the host pushes the new
// entity, and its host goes on (`onCreated`). A failure stays in the form,
// as it was.
function createNew(d) {
  if (d.busy || newBlocked(d)) return;
  const args = { roles: newRoles(d), title: d.title.trim(), projects: d.projects, labels: d.labels };
  if (d.linked) args.linked_to = d.linked;
  if (d.classification) args.classification = d.classification;
  if (d.type.kind === 'document') args.date = isoInstant(zonedInstant(d.day, localTime(new Date())));
  if (!['list', 'record'].includes(d.type.kind) && d.body.trim()) args.content = d.body;
  d.busy = true;
  d.error = null;
  renderNewFoot(d);
  rpcCall('create_entity', args).then(({ id }) => d.onCreated(id), e => {
    d.busy = false;
    d.error = e.message;
    renderNewFoot(d);
  });
}

// ----------------------------------------------------------------------------
// dates: the form's are local days (YYYY-MM-DD) and times (hh:mm)

// An hour after `time`, at most 23:59.
function newHourLater(time) {
  const [h, m] = time.split(':').map(Number);
  return h >= 23 ? '23:59' : localTime(new Date(2000, 0, 1, h + 1, m));
}

// The day after `day`.
function newNextDay(day) {
  const [y, m, d] = day.split('-').map(Number);
  return localDay(new Date(y, m - 1, d + 1));
}

// ----------------------------------------------------------------------------
// rendering

// The form of `d`: its title, the folder (on the New page), its rows, its
// body, the note of the one made before, the error and the buttons.
function renderNewForm(d) {
  const title = accEl('input', 'input new__title');
  title.type = 'text';
  title.placeholder = d.type.hint;
  title.value = d.title;
  title.dataset.focusKey = 'new:title';
  title.addEventListener('input', () => {
    d.title = title.value;
    renderNewFoot(d);
  });
  const parts = [accEl('span', 'label new__title-label', d.type.label), title];

  if (d.page) {
    const path = accEl('span', 'mono mono--small truncate-start new__path', `${d.type.folder}\\`);
    path.title = t('new.path_title');
    parts.push(path);
  }

  const [rows, row] = watchFormGrid();
  rows.classList.add('new__rows');
  renderNewTypeRows(d, row);
  d.els.chips = row(t('new.projects_labels'));
  d.els.linked = d.linked ? row(t('new.linked')) : null;
  parts.push(rows, ...renderNewBody(d));

  if (d.created) {
    parts.push(createdNote(t('new.created', { title: d.created.title }), d.created.id, t('new.show_task')));
  }
  d.els.error = accEl('div', 'error-note new__error');
  d.els.foot = accEl('div', 'new__foot');
  parts.push(d.els.error, d.els.foot);
  return parts;
}

// The rows of the type's own fields.
function renderNewTypeRows(d, row) {
  switch (d.type.kind) {
    case 'document': {
      row(t('props.type'), accEl('span', '', 'note'));
      const note = accEl('span', 'meta', relativeDay(parseDate(d.day)));
      const day = newDateInput(d.day, value => {
        d.day = value;
        note.textContent = relativeDay(parseDate(value));
        renderNewFoot(d);
      });
      row(t('docs.date'), day, note);
      break;
    }
    case 'task': {
      newSegRow(row(t('props.status')), TASK_STATUSES, () => d.status, value => { d.status = value; });
      const note = accEl('span', 'meta');
      const due = newDateInput(d.due, value => {
        d.due = value;
        showDue();
      });
      const clear = newClearButton(t('new.no_due'), () => {
        d.due = '';
        due.value = '';
        showDue();
      });
      const showDue = () => {
        note.textContent = d.due ? relativeDay(parseDate(d.due)) : t('new.none');
        clear.hidden = !d.due;
      };
      row(t('props.due'), due, note, clear);
      showDue();
      break;
    }
    case 'event':
      renderNewEventRows(d, row);
      break;
    case 'contact':
      newSegRow(row(t('new.type')), NEW_CONTACT_TYPES, () => d.contactType, value => { d.contactType = value; });
      row(t('new.email_address'), newTextInput(d, 'email', '', { type: 'email' }));
      row(t('new.phone'), newTextInput(d, 'phone', '', { type: 'tel' }));
      row(t('accounts.address'), newTextInput(d, 'address', t('new.street_town')));
      break;
    default:
      break;
  }
}

// An event's rows: its Type (another builds the sheet again), planned or
// booked, its calendar; then by its type a journey's means, places and
// times, a stay's or a layover's place and nights, an activity's place
// and times, a plain event's times; and where it is.
function renderNewEventRows(d, row) {
  d.els.zones = [];
  const type = row(t('props.type'));
  type.append(accSeg(NEW_EVENT_TYPES, d.eventType, value => {
    d.eventType = value;
    if (byNights({ type: value }) && d.end <= d.start) d.end = newNextDay(d.start);
    renderNewSheet(d);
    d.els.type.querySelector('[aria-pressed="true"]')?.focus();
  }));
  d.els.type = type;
  if (d.eventType) newSegRow(row(t('props.state')), STATES, () => d.eventStatus, value => { d.eventStatus = value; });
  else newSegRow(row(t('props.status')), EVENT_STATUSES, () => d.eventStatus, value => { d.eventStatus = value; });
  row(t('props.calendar'), newCalendarButton(d));
  if (d.eventType === 'journey') {
    const means = MEANS.map(([value, label]) => [value, label, { icon: MEANS_ICONS[value] }]);
    newSegRow(row(t('props.by')), means, () => d.by, value => {
      d.by = value;
      d.els.icon.firstChild.setAttribute('href', newDraftIcon(d));
    });
    newPlaceRow(d, row(t('props.from')), 'from', 'fromZone');
    newTimeRows(d, row, [t('props.leaves'), t('props.arrives')], ['fromZone', 'toZone'],
      () => newPlaceRow(d, row(t('props.to')), 'to', 'toZone'));
    row(t('props.booking'), newTextInput(d, 'booking', t('new.its_reference'), { className: 'form-input mono' }));
  } else if (byNights({ type: d.eventType })) {
    newPlaceRow(d, row(t('props.place')), 'place', null);
    newNightsRow(d, row(t('props.nights')));
  } else if (d.eventType === 'activity') {
    newPlaceRow(d, row(t('props.place')), 'place', 'fromZone');
    newTimeRows(d, row, [t('props.starts'), t('props.ends')], ['fromZone', null]);
  } else {
    newTimeRows(d, row, [t('props.starts'), t('props.ends')], ['fromZone', null]);
    newScheduleRow(d, row(t('props.schedule')));
  }
  row(t('new.location'), newTextInput(d, 'location', t('new.address_or_place')));
}

// A plain event's schedule (`eventScheduleField`): once at first, drawn
// again on each pick; a week starts on the start's weekday.
function newScheduleRow(d, field) {
  const render = () => field.replaceChildren(eventScheduleField(d.schedule, {
    from: parseDate(d.start) ?? new Date(), key: 'new:schedule',
    bind: (input, set) => input.addEventListener('change', () => set(input.value)),
    onChange: rule => {
      d.schedule = rule;
      render();
    },
  }));
  render();
}

// The draft's calendar: a button naming it, which opens the vault's.
function newCalendarButton(d) {
  const button = accEl('button', 'btn btn--ghost new__calendar');
  button.type = 'button';
  const show = () => { button.textContent = entities.get(d.calendar)?.title || t('watch.none'); };
  button.addEventListener('click', () => calendarMenu(button, d.calendar, id => {
    d.calendar = id;
    show();
  }, undefined, { writable: true }));
  show();
  return button;
}

// A place of the draft (`key`): typed in, the vault's trip places offered;
// one named that had a zone (`placeZone`) sets the draft's `zoneKey`.
function newPlaceRow(d, field, key, zoneKey) {
  field.append(placeInput(newTextInput(d, key, t('props.place'), {
    onInput: () => {
      const zone = zoneKey && placeZone(d[key]);
      if (!zone || zone === d[zoneKey]) return;
      d[zoneKey] = zone;
      renderNewZones(d);
    },
  })));
}

// The start's and end's rows (`labels`): a day and a time each, the end
// moving with the start while it lies before it -- the same day, an hour
// later. With `zones` (the draft's keys), each in its zone, a button naming
// it that opens the zones' menu; `between()` adds the rows between them (a
// journey's To). Without a start time it is all day: its end a day, the
// end's time off; All day, a check, clears the time or gives it 9:00 to
// 10:00 again.
function newTimeRows(d, row, [startLabel, endLabel], zones = [null, null], between = null) {
  const note = accEl('span', 'meta', relativeDay(parseDate(d.start)));
  const allDay = accEl('button', 'check');
  allDay.type = 'button';
  titled(allDay, t('event.all_day'));
  const dayOnly = () => {
    allDay.textContent = d.startTime ? '' : '\u2713';
    allDay.setAttribute('aria-pressed', String(!d.startTime));
    endTime.disabled = !d.startTime;
  };
  allDay.addEventListener('click', () => {
    [d.startTime, d.endTime] = d.startTime ? ['', d.endTime] : ['09:00', '10:00'];
    startTime.value = d.startTime;
    endTime.value = d.endTime;
    dayOnly();
  });
  const keepEnd = () => {
    if (!d.start) return;
    if (!d.end || d.end < d.start) d.end = d.start;
    if (d.end === d.start && d.startTime && d.endTime <= d.startTime) d.endTime = newHourLater(d.startTime);
    endDay.value = d.end;
    endTime.value = d.endTime;
  };
  const startDay = newDateInput(d.start, value => {
    d.start = value;
    note.textContent = relativeDay(parseDate(value));
    keepEnd();
    renderNewFoot(d);
  });
  const startTime = newTimeInput(d.startTime, value => {
    d.startTime = value;
    keepEnd();
    dayOnly();
  });
  const endDay = newDateInput(d.end, value => { d.end = value; });
  const endTime = newTimeInput(d.endTime, value => { d.endTime = value; });
  dayOnly();
  const zone = i => (zones[i] ? [newZoneSlot(d, zones[i])] : []);
  row(startLabel, startDay, startTime, ...zone(0), note, allDay, accEl('span', 'meta', t('event.all_day')));
  between?.();
  row(endLabel, endDay, endTime, ...zone(1));
}

// The zone `zoneKey` of the draft: kept to be built again when a place
// takes its zone or another is picked (`renderNewZones`).
function newZoneSlot(d, zoneKey) {
  const slot = accEl('span', 'new__zone');
  d.els.zones.push({ slot, zoneKey });
  renderNewZones(d);
  return slot;
}

function renderNewZones(d) {
  for (const { slot, zoneKey } of d.els.zones ?? []) {
    slot.replaceChildren(zoneButton(null, d[zoneKey], zone => {
      d[zoneKey] = zone;
      renderNewZones(d);
    }));
  }
}

// A stay's nights: the first night's day and the day it ends, the morning
// after the last, and how many.
function newNightsRow(d, field) {
  const count = accEl('span', 'meta');
  const show = () => {
    const nights = d.start && d.end ? Math.round((parseDate(d.end) - parseDate(d.start)) / 864e5) : 0;
    count.textContent = nights > 0 ? t('trip.nights', { n: nights }) : t('new.ends_before_start');
  };
  const from = newDateInput(d.start, value => {
    d.start = value;
    if (d.end <= value) {
      d.end = newNextDay(value);
      until.value = d.end;
    }
    show();
    renderNewFoot(d);
  });
  const until = newDateInput(d.end, value => {
    d.end = value;
    show();
  });
  field.append(from, accEl('span', 'meta', t('new.until')), until, count);
  show();
}

// The list's items, a record's note, or the markdown body of the others.
function renderNewBody(d) {
  if (d.type.kind === 'list') {
    const head = accEl('div', 'section-head new__body-head', t('new.items'));
    head.append(accEl('span', 'section-head__note', t('new.enter_adds')));
    d.els.items = accEl('div', 'new__items');
    const add = accEl('button', 'tool new__add-item', t('new.add_item'));
    add.type = 'button';
    add.addEventListener('click', () => newAddItem(d, d.items.length));
    renderNewItems(d);
    return [head, d.els.items, add];
  }
  if (d.type.kind === 'record') {
    return [accEl('div', 'note new__body-head', t('new.record_note'))];
  }
  const head = accEl('div', 'section-head new__body-head', t('new.content'));
  head.append(accEl('span', 'section-head__note', 'Markdown'));
  const body = accEl('textarea', 'input new__body');
  body.placeholder = 'Markdown';
  body.value = d.body;
  body.addEventListener('input', () => { d.body = body.value; });
  return [head, body];
}

// A list's items, one input each: Enter goes to the next (a new one after
// the last), Backspace in an empty one removes it. `focus`: the index of
// the item to focus.
function renderNewItems(d, focus = null) {
  const rows = d.items.map((text, i) => {
    const item = accEl('div', 'new__item');
    const input = accEl('input', 'form-input');
    input.type = 'text';
    input.placeholder = t('new.item');
    input.value = text;
    input.addEventListener('input', () => { d.items[i] = input.value; });
    input.addEventListener('keydown', e => {
      if (e.key === 'Enter' && !e.ctrlKey) {
        e.preventDefault();
        if (i + 1 < d.items.length) renderNewItems(d, i + 1);
        else newAddItem(d, i + 1);
      } else if (e.key === 'Backspace' && !input.value && d.items.length > 1) {
        e.preventDefault();
        d.items.splice(i, 1);
        renderNewItems(d, Math.max(0, i - 1));
      }
    });
    const remove = accEl('button', 'tool new__item-remove');
    remove.type = 'button';
    remove.append(iconEl('#i-dismiss'));
    remove.title = t('new.remove_item');
    remove.addEventListener('click', () => {
      d.items.splice(i, 1);
      if (!d.items.length) d.items.push('');
      renderNewItems(d);
    });
    item.append(input, remove);
    return item;
  });
  d.els.items.replaceChildren(...rows);
  if (focus !== null) rows[focus]?.querySelector('input').focus();
}

function newAddItem(d, at) {
  d.items.splice(at, 0, '');
  renderNewItems(d, at);
}

// The projects and labels on one row, as on a card, and the link.
function renderNewChips(d) {
  draftChips(d, d.els.chips, () => renderNewChips(d));
  if (d.scope && d.projects.length === 1 && d.projects[0] === d.scope) {
    d.els.chips.append(accEl('span', 'meta push', t('new.from_scope')));
  }

  if (d.els.linked) {
    const unlink = () => {
      d.linked = null;
      renderNewChips(d);
    };
    d.els.linked.replaceChildren(d.linked && linkTarget(d.linked)
      ? linkChip(d.linked, { onRemove: unlink })
      : accEl('span', 'meta', t('new.none')));
  }
}

// Create (and on the New page Cancel), and the error of a failed Create.
function renderNewFoot(d) {
  const blocked = newBlocked(d);
  const create = accEl('button', 'btn btn--primary', d.busy ? t('new.creating') : d.type.create);
  create.type = 'button';
  create.disabled = !!blocked || d.busy;
  create.title = blocked || d.type.createTitle;
  create.addEventListener('click', () => createNew(d));
  d.els.foot.replaceChildren(create, accEl('span', 'meta', shortcutText('Ctrl+Enter')));
  if (d.page) {
    const cancel = accEl('button', 'btn btn--ghost push', t('field.cancel'));
    cancel.type = 'button';
    cancel.addEventListener('click', closeVaultArea);
    d.els.foot.append(cancel, accEl('span', 'meta', shortcutText('Escape')));
  }
  d.els.error.textContent = d.error ?? '';
  d.els.error.hidden = !d.error;
}

// ----------------------------------------------------------------------------
// fields

// An input of the draft `d`'s `key`, of `type`; `onInput` after each change.
// The New page's, and New recurring payment's (features/recurring-payment-new.js).
export function newTextInput(d, key, placeholder, { type = 'text', className = 'form-input', onInput = null } = {}) {
  const input = accEl('input', className);
  input.type = type;
  input.placeholder = placeholder;
  input.value = d[key];
  input.addEventListener('input', () => {
    d[key] = input.value;
    onInput?.();
  });
  return input;
}

// The projects and labels of the draft `d` into `el`: a chip each, with x,
// and "+" (`addChip`); `rerender` after a change.
export function draftChips(d, el, rerender) {
  const leave = project_id => {
    d.projects = d.projects.filter(p => p !== project_id);
    rerender();
  };
  const setLabels = labels => {
    d.labels = labels;
    rerender();
  };
  const add = addChip({ projects: d.projects, labels: d.labels }, {
    onJoin: project_id => {
      d.projects = [...d.projects, project_id];
      rerender();
    },
    onAddLabel: label => setLabels([...d.labels, label]),
  });
  el.replaceChildren(...projectChips(d.projects, leave), ...labelChips(d.labels, setLabels), add);
}

export function newDateInput(value, onChange) {
  const input = accEl('input', 'form-input new__date');
  input.type = 'date';
  input.value = value;
  input.addEventListener('change', () => onChange(input.value));
  return input;
}

function newTimeInput(value, onChange) {
  const input = accEl('input', 'form-input new__time');
  input.type = 'time';
  input.value = value;
  input.addEventListener('change', () => onChange(input.value));
  return input;
}

// A segmented control of `options` in `field`, built again on a pick.
function newSegRow(field, options, current, onPick) {
  const render = () => field.replaceChildren(accSeg(options, current(), value => {
    onPick(value);
    render();
  }));
  render();
}

function newClearButton(title, onClear) {
  const button = accEl('button', 'tool push');
  button.type = 'button';
  button.append(iconEl('#i-dismiss'));
  button.title = title;
  button.addEventListener('click', onClear);
  return button;
}
