// The vault panel (ui.md#vault-panel): the projects (starred first, the
// archived folded) after No project, and the sources, with their counts,
// each a filter; renaming, adding, starring and dropping onto projects;
// the sources' sync buttons; the vault's name and menu. It renders at most
// once per frame (`scheduleRenderSidebar`), following the model (app.js).
// The labels filter from the lens head (features/header.js).

import { $ } from '../core/dom.js';
import { accEl, oncePerFrame, placeChildren, setAria } from '../core/dom.js';
import { rpcCall } from '../core/rpc.js';
import { runAction } from '../core/actions.js';
import { store } from '../model/state.js';
import { showError } from '../model/state.js';
import { NO_PROJECT, entities, projectsInOrder, projectsOf } from '../model/entities.js';
import { projects } from '../model/entities.js';
import { accounts, sourceKeyOf } from '../model/entities.js';
import { watchSourceKey, watches } from '../model/entities.js';
import { structureCall } from '../model/structure.js';
import { joinProjectLocal } from '../model/structure.js';
import { iconEl } from '../ui/icons.js';
import { KIND_ICONS } from '../ui/icons.js';
import { IMPORT_ICON } from '../ui/icons.js';
import { onEditEnd } from '../ui/field.js';
import { showMenu } from '../ui/menu.js';
import { dropTarget } from '../ui/dropzone.js';
import { selectFilter } from './selection.js';
import { watchType } from './workspace/folders.js';
import { openScanImport } from './import/scans.js';
import { openMailFetch } from './import/mail.js';
import { revealVault, switchVault } from './workspace/vault.js';

const projectsContainer = $('projects');
const sourcesContainer = $('sources');
const allProjectsEl = $('all-projects');
const noProjectEl = $('no-project');
const allSourcesEl = $('all-sources');

// ----------------------------------------------------------------------------
// The sidebar's per-project counts are derived, never sent (vault.md#thread:
// membership is always computed): the entities whose `projects` list the
// project, and those in none for No project. Re-rendered at most once per
// frame, since every `entity` event of a scan may change it.
export const scheduleRenderSidebar = oncePerFrame(renderSidebar);

// A sidebar row without its count and highlight, which `updateNavItem` sets.
// `action` ({ icon, title, onClick }) becomes a circular icon button at the
// row's right end, shown while the row is hovered; `onClick` gets the row
// and the button.
function sidebarNavItem(className, name, classIcon, icon, onClick, action) {
  const el = accEl('a', className);
  const elName = accEl('span', 'nav-item__name', name);
  const elCount = accEl('span', 'meta nav-item__count');
  const elIcon = iconEl(icon, classIcon);
  el.appendChild(elIcon);
  el.appendChild(elName);
  el.appendChild(elCount);
  if (action) {
    const btn = accEl('button', 'icon-btn icon-btn--quiet nav-item__action');
    btn.type = 'button';
    btn.title = action.title;
    btn.append(iconEl(action.icon));
    btn.addEventListener('click', e => {
      e.stopPropagation();
      action.onClick(el, btn);
    });
    el.appendChild(btn);
  }
  el.addEventListener('click', onClick);
  return el;
}

// `current`: the row is the selected project or source; `starred`: a
// project is starred.
function updateNavItem(row, count, current, starred) {
  const countEl = row.querySelector('.nav-item__count');
  if (countEl.textContent !== String(count)) countEl.textContent = count;
  setAria(row, 'aria-current', current);
  setAria(row, 'aria-selected', starred);
}

// The row being renamed in place (see `editSidebarName`): while it is,
// `renderSidebar` leaves the sidebar alone, and runs once the edit ends.
let sidebarEditing = false;

// Swap `row`'s name for a text input: Enter or leaving it commits (calling
// `commit` with the trimmed text if it changed), Esc cancels.
function editSidebarName(row, name, commit) {
  const nameEl = row.querySelector('.nav-item__name');
  const input = accEl('input', 'nav-item__edit');
  input.value = name;
  nameEl.replaceWith(input);
  row.classList.add('nav-item--editing');
  sidebarEditing = true;

  onEditEnd(input, save => {
    sidebarEditing = false;
    input.replaceWith(nameEl);
    row.classList.remove('nav-item--editing');
    const title = input.value.trim();
    if (save && title !== name) commit(title);
    scheduleRenderSidebar();
  });
  input.focus();
  input.select();
}

// The sidebar's rows by key (`project:<id>`, `source:<key>`), kept across
// renders: `renderSidebar` patches counts and highlights in place and
// rebuilds a row only when its name changed. A scan re-renders the sidebar every frame,
// and a replaced row would lose its hover (flickering its action button)
// and its drop-target highlight.
let sidebarRows = new Map();

function renderSidebar() {
  if (sidebarEditing) return;
  const projectCounts = new Map();
  let noProjectCount = 0;
  const sourceCounts = new Map();
  let sourceTotal = 0;
  for (const doc of entities.values()) {
    for (const id of doc.projects ?? []) projectCounts.set(id, (projectCounts.get(id) ?? 0) + 1);
    if (!projectsOf(doc).length) noProjectCount++;
    const key = sourceKeyOf(doc);
    if (key) {
      sourceCounts.set(key, (sourceCounts.get(key) ?? 0) + 1);
      sourceTotal++;
    }
  }

  const { selected_project, selected_source } = store.getState();

  // The row for `key`: the last render's if it was built for `name`,
  // otherwise a new one from `build`. Rows not asked for are dropped.
  const rows = new Map();
  const rowFor = (key, name, build) => {
    let row = sidebarRows.get(key);
    if (row?.dataset.name !== name) {
      row = build();
      row.dataset.name = name;
    }
    rows.set(key, row);
    return row;
  };

  // Starred projects first; otherwise in the order they arrived, so a new
  // project's pill stays last while it is renamed in place. The archived
  // ones follow, folded under an "Archived" row (`archivedUnfolded`).
  const projectRows = [];
  const archivedRows = [];
  let editRow = null;
  for (const { id, name, starred, archived } of projectsInOrder()) {
    const row = rowFor(`project:${id}`, name, () => {
      const row = sidebarNavItem('nav-item nav-item--project', name, 'nav-item__icon nav-item__icon--star', '#i-star', () => selectProject(id),
        { icon: '#i-edit', title: 'Rename project', onClick: row => editProject(id, row) });
      row.dataset.id = id; // the drop target of `joinProjectLocal`, see below
      row.querySelector('.nav-item__icon').addEventListener('click', e => {
        e.stopPropagation();
        toggleStar(id);
      });
      return row;
    });
    row.classList.toggle('nav-item--archived', archived);
    updateNavItem(row, projectCounts.get(id) ?? 0, id === selected_project, starred);
    // The star's symbol follows `starred`: an SVG's can't change in CSS.
    row.querySelector('.nav-item__icon use').setAttribute('href', starred ? '#i-star-filled' : '#i-star');
    (archived ? archivedRows : projectRows).push(row);
    if (id === editProjectId) editRow = row;
  }
  if (archivedRows.length) {
    const fold = rowFor('archived', 'Archived', () =>
      sidebarNavItem('nav-item nav-item--compact', 'Archived', 'nav-item__icon disclosure', '#i-chevron-down', toggleArchived));
    fold.setAttribute('aria-expanded', String(archivedUnfolded));
    updateNavItem(fold, archivedRows.length, false, false);
    projectRows.push(fold);
    if (archivedUnfolded) projectRows.push(...archivedRows);
  }
  placeChildren(projectsContainer, projectRows);
  if (editRow) {
    editProjectId = null;
    editProject(editRow.dataset.id, editRow);
  }

  const sourceRows = [];

  // The entities created in the app; its button opens the New menu.
  const createdRow = rowFor('source:created', 'Created Here', () =>
    sidebarNavItem('nav-item nav-item--compact', 'Created Here', 'nav-item__icon', '#i-home', () => selectSource('created'),
      { icon: '#i-add', title: 'New (Ctrl+N)', onClick: (_row, button) => runAction('new', button) }));
  updateNavItem(createdRow, sourceCounts.get('created') ?? 0, selected_source === 'created');
  sourceRows.push(createdRow);

  // Per enabled folder watch, under its name: what it imported. Its button
  // opens the import page of its type.
  for (const watch of watches.values()) {
    if (!watch.enabled) continue;
    const key = watchSourceKey(watch.id);
    const action = {
      icon: IMPORT_ICON,
      title: `Import from ${watch.name}`,
      onClick: () => openScanImport(watch.id),
    };
    const row = rowFor(`source:${key}`, watch.name, () =>
      sidebarNavItem('nav-item nav-item--compact', watch.name, 'nav-item__icon', watchType(watch.type).icon,
        () => selectSource(key), action));
    updateNavItem(row, sourceCounts.get(key) ?? 0, key === selected_source);
    sourceRows.push(row);
  }

  // Per connected account, under its email: mail, the checked calendars. A
  // row click filters by it; its button syncs it.
  for (const account of accounts.values()) {
    for (const source of accountSources(account)) {
      const row = rowFor(`source:${source.key}`, source.name, () =>
        sidebarNavItem('nav-item nav-item--compact', source.name, 'nav-item__icon', source.icon, () => selectSource(source.key),
          { icon: '#i-arrow-sync', title: `Sync ${source.name}`, onClick: () => syncSource(source) }));
      updateNavItem(row, sourceCounts.get(source.key) ?? 0, source.key === selected_source);
      // A source that syncs on its own shows its button pressed.
      const button = row.querySelector('.nav-item__action');
      setAria(button, 'aria-pressed', source.autoSync);
      const action = source.kind === 'email' ? 'Fetch mail' : `Sync ${source.name}`;
      button.title = source.autoSync ? `${action} \u00b7 auto-sync on` : `Sync ${source.name}`;
      sourceRows.push(row);
    }
  }

  placeChildren(sourcesContainer, sourceRows);

  sidebarRows = rows;

  setAria(allProjectsEl, 'aria-current', !selected_project);
  updateNavItem(noProjectEl, noProjectCount, selected_project === NO_PROJECT);
  updateNavItem(allSourcesEl, sourceTotal, !selected_source);
}

// Filter actions: one project, No project or all, one source or all
// (clicking the selected one again, or its "All" row, selects all). Every
// filter row is a non-page item: a click on it closes the open vault page
// (`selectFilter`, features/selection.js), even if the filter stays the
// same.
function selectProject(project_id) {
  const current = store.getState().selected_project;
  selectFilter({ selected_project: project_id === current ? null : project_id });
}

function selectSource(key) {
  const current = store.getState().selected_source;
  selectFilter({ selected_source: key === current ? null : key });
}

// Rename a project in place; the host writes the new title (an empty one
// removes it) and pushes the project back as `entity`.
function editProject(project_id, row) {
  const name = projects.get(project_id)?.name ?? '';
  editSidebarName(row, name, title =>
    structureCall('set_title', { id: project_id, title }, () => store.getState()));
}

// The project `addProject` created, to be renamed in place as soon as
// `renderSidebar` has built its pill.
let editProjectId = null;

// The "Add project" button: the host writes a new project and pushes it as
// `entity`; its pill then opens in rename mode. The selection stays.
function addProject() {
  structureCall('create_project', { title: 'New project' }, project_id => {
    editProjectId = project_id;
    scheduleRenderSidebar();
    return store.getState();
  });
}

$('add-project-btn').addEventListener('click', addProject);

// Dropping an entity (a table row, which drags its doc_id as text/plain; see
// features/lenses/documents.js) on a project pill adds it to that project. The
// listeners sit on the container, since `renderSidebar` replaces a renamed
// pill, possibly mid-drag. Only a pill is a valid drop target: over
// the gaps between them `dragover` isn't cancelled, so a drop there is a no-op.
dropTarget(projectsContainer, {
  over: ev => {
    const row = ev.target.closest('.nav-item--project');
    return row && { el: row };
  },
  drop: ({ el: row }, ev) => {
    const doc_id = ev.dataTransfer.getData('text/plain');
    if (doc_id) joinProjectLocal(doc_id, row.dataset.id);
  },
});

allProjectsEl.addEventListener('click', () => selectFilter({ selected_project: null }));
noProjectEl.addEventListener('click', () => selectProject(NO_PROJECT));
allSourcesEl.addEventListener('click', () => selectFilter({ selected_source: null }));

// The star on a project pill: the host writes `starred` (unstarring removes
// it) and pushes the project back as `entity`, which re-sorts the pills.
// The selection, the filter and an open page stay.
function toggleStar(project_id) {
  const starred = !projects.get(project_id)?.starred;
  structureCall('set_starred', { id: project_id, starred }, () => store.getState());
}

// The "Archived" row folds and unfolds the archived projects; not kept
// across restarts.
let archivedUnfolded = false;

function toggleArchived() {
  archivedUnfolded = !archivedUnfolded;
  scheduleRenderSidebar();
}

// Sources: what a connected account feeds into the vault. Mail follows the
// granted access; calendars are the ones checked on the Accounts page
// (`account.calendars`). `autoSync`: the host syncs it on its own.
function accountSources(account) {
  if (account.kind !== 'google' || account.status !== 'connected') return [];
  const { id, scope = {}, email = 'Mail' } = account;
  const sources = [];
  if (scope.email && scope.email !== 'off') {
    sources.push({
      key: `${id}\nemail`, kind: 'email', account: id, name: email, icon: KIND_ICONS.email.icon,
      autoSync: !!account.mail?.auto_sync?.enabled,
    });
  }
  if (scope.calendar && scope.calendar !== 'off') {
    for (const calendar of account.calendars ?? []) {
      sources.push({
        key: `${id}\ncalendar\n${calendar.id}`, kind: 'calendar', account: id,
        calendar: calendar.id, name: calendar.name, icon: KIND_ICONS.calendar.icon,
        autoSync: !!calendar.auto_sync,
      });
    }
  }
  return sources;
}

// The sync button of a source: Mail opens the Fetch mail page
// (features/import/mail.js); a calendar syncs right away, its progress and
// summary in the jobs panel.
function syncSource(source) {
  if (source.kind === 'email') return openMailFetch(source.account);
  if (source.kind === 'calendar') return syncCalendar(source);
}

// The sync runs as a job (the jobs panel); a refused one is said in the
// status bar.
function syncCalendar(source) {
  rpcCall('calendar_sync', { account: source.account, calendar: source.calendar }).catch(e => {
    if (e.code !== 'cancelled') showError(`Could not sync ${source.name}: ${e.message}`);
  });
}

// ----------------------------------------------------------------------------
store.subscribe(state => state.vault_name,
  vault_name => { $('vault-name').textContent = vault_name || 'No vault'; });

store.subscribe(
  state => state.files_total,
  files_total => {
    $('all-documents-count').textContent = files_total;
  });

$('vault-menu-btn').addEventListener('click', ev => {
  ev.stopPropagation();
  const items = [
    { label: 'Switch vault\u2026', run: switchVault },
    { label: 'Show in folder', run: revealVault },
  ];
  showMenu(ev.currentTarget, items);
});
