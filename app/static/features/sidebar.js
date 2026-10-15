// The vault panel (ui.md#vault-panel): the projects (starred first, the
// archived folded) after No project, with their counts, each a filter;
// the vault's name and menu. It renders at most once per frame
// (`scheduleRenderSidebar`), following the model (app.js).

import { $ } from '../core/dom.js';
import { accEl, oncePerFrame, placeChildren, setAria } from '../core/dom.js';
import { store } from '../model/state.js';
import { NO_PROJECT, entities, projectsInOrder, projectsOf } from '../model/entities.js';
import { iconEl } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import { revealVault, switchVault } from './workspace/vault.js';

const projectsContainer = $('projects');
const allProjectsEl = $('all-projects');
const noProjectEl = $('no-project');

// ----------------------------------------------------------------------------
// The sidebar's per-project counts are derived, never sent (vault.md#thread:
// membership is always computed): the entities whose `projects` list the
// project, and those in none for No project. Re-rendered at most once per
// frame, since every `entity` event of a scan may change it.
export const scheduleRenderSidebar = oncePerFrame(renderSidebar);

// A sidebar row without its count and highlight, which `updateNavItem` sets.
function sidebarNavItem(className, name, classIcon, icon, onClick, action) {
  const el = accEl('a', className);
  const elName = accEl('span', 'nav-item__name', name);
  const elCount = accEl('span', 'meta nav-item__count');
  const elIcon = iconEl(icon, classIcon);
  el.appendChild(elIcon);
  el.appendChild(elName);
  el.appendChild(elCount);
  el.addEventListener('click', onClick);
  return el;
}

// `current`: the row is the selected project; `starred`: a project is
// starred.
function updateNavItem(row, count, current, starred) {
  const countEl = row.querySelector('.nav-item__count');
  if (countEl.textContent !== String(count)) countEl.textContent = count;
  setAria(row, 'aria-current', current);
  setAria(row, 'aria-selected', starred);
}

// The sidebar's rows by key (`project:<id>`), kept across renders:
// `renderSidebar` patches counts and highlights in place and rebuilds a row
// only when its name changed. A scan re-renders the sidebar every frame,
// and a replaced row would lose its hover.
let sidebarRows = new Map();

function renderSidebar() {
  const projectCounts = new Map();
  let noProjectCount = 0;
  for (const doc of entities.values()) {
    for (const id of doc.projects ?? []) projectCounts.set(id, (projectCounts.get(id) ?? 0) + 1);
    if (!projectsOf(doc).length) noProjectCount++;
  }

  const { selected_project } = store.getState();

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

  // Starred projects first; otherwise in the order they arrived. The
  // archived ones follow, folded under an "Archived" row (`archivedUnfolded`).
  const projectRows = [];
  const archivedRows = [];
  for (const { id, name, starred, archived } of projectsInOrder()) {
    const row = rowFor(`project:${id}`, name, () =>
      sidebarNavItem('nav-item nav-item--project', name, 'nav-item__icon nav-item__icon--star', '#i-star-filled', () => selectProject(id)));
    row.classList.toggle('nav-item--archived', archived);
    updateNavItem(row, projectCounts.get(id) ?? 0, id === selected_project, starred);
    (archived ? archivedRows : projectRows).push(row);
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

  sidebarRows = rows;

  setAria(allProjectsEl, 'aria-current', !selected_project);
  updateNavItem(noProjectEl, noProjectCount, selected_project === NO_PROJECT);
}

// Filter actions: one project, No project or all (clicking the selected one
// again, or its "All" row, selects all).
function selectProject(project_id) {
  const current = store.getState().selected_project;
  store.setState({ selected_project: project_id === current ? null : project_id });
}

allProjectsEl.addEventListener('click', () => store.setState({ selected_project: null }));
noProjectEl.addEventListener('click', () => selectProject(NO_PROJECT));

// The "Archived" row folds and unfolds the archived projects; not kept
// across restarts.
let archivedUnfolded = false;

function toggleArchived() {
  archivedUnfolded = !archivedUnfolded;
  scheduleRenderSidebar();
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
