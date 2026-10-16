// The lens header (panel.md#projects): the selected project's name, the
// active labels with the "+" that filters by another one
// (ui.md#vault-panel), and the project's details -- a toolbar (rename,
// archive).

import { $, accEl, oncePerFrame } from '../core/dom.js';
import { store } from '../model/state.js';
import { NO_PROJECT, entities, matchesFilter, projects } from '../model/entities.js';
import { projectScope } from '../model/entities.js';
import { structureCall } from '../model/structure.js';
import { showMenu } from '../ui/menu.js';
import { editTextInPlace } from '../ui/field.js';

const projectNameEl = $('project-name');

// The header names the selected project, No project or All Projects;
// `projects_rev` catches a rename. A name being edited in place is left
// alone.
store.subscribe(
  [state => state.selected_project, state => state.projects_rev],
  ([selected_project]) => {
    if (projectNameEl.hasAttribute('contenteditable')) return;
    if (selected_project === NO_PROJECT) projectNameEl.textContent = 'No project';
    else if (selected_project) projectNameEl.textContent = projects.get(selected_project)?.name ?? '';
    else projectNameEl.textContent = 'All Projects';
  });

// After the name, the header names the active labels, each with an x that
// takes it out of the filter, and a "+" that adds one, while any label is
// in use: looked at once per frame, not once per entity of a vault scan.
const labelFilterEl = $('label-filter');
const labelAddBtn = $('label-add-btn');
store.subscribe(state => state.selected_labels, selected_labels =>
  labelFilterEl.replaceChildren(...tagChips(selected_labels,
    next => store.setState({ selected_labels: next }),
    tag => `Stop filtering by ${tag}`)));
const scheduleLabelAdd = oncePerFrame(() => {
  labelAddBtn.hidden = ![...entities.values()].some(doc => doc.tags?.length);
});
store.subscribe(state => state.entities_rev, scheduleLabelAdd);

// The "+" menu: every label in use but the active ones, filtered as typed,
// each with what the filter would show with it added; one that would show
// nothing is dim.
labelAddBtn.addEventListener('click', () => {
  const state = store.getState();
  const counts = new Map(); // label -> the entities in the filter carrying it
  for (const doc of entities.values()) {
    const shown = matchesFilter(doc, state);
    for (const tag of doc.tags ?? []) counts.set(tag, (counts.get(tag) ?? 0) + (shown ? 1 : 0));
  }
  const items = [...counts.keys()].filter(tag => !state.selected_labels.includes(tag))
    .sort((a, b) => a.localeCompare(b))
    .map(tag => ({
      label: tag,
      hint: String(counts.get(tag)),
      dim: !counts.get(tag),
      run: () => store.setState({ selected_labels: [...store.getState().selected_labels, tag] }),
    }));
  showMenu(labelAddBtn, items.length ? items : [{ label: 'No other labels', dim: true }],
    { head: 'Filter by a label', filter: { placeholder: 'Filter labels' } });
});

// The selected project's details under the header, shown while the fold
// button (the chevron) is unfolded: a toolbar (rename, archive).
const projectFoldBtn = $('project-fold-btn');
const projectToolsEl = $('project-tools');
let descProjectId = null; // the project the details are bound to

function projectUnfolded() {
  return projectFoldBtn.getAttribute('aria-expanded') === 'true';
}

function updateProjectDesc() {
  const shown = Boolean(descProjectId) && projectUnfolded();
  projectToolsEl.hidden = !shown;
}

// Rename the selected project in the header itself, like its sidebar pill
// (`editProject`): the host writes the new title and pushes the project
// back as `entity`, which renames the header.
function editProjectName() {
  const project_id = descProjectId;
  editTextInPlace(projectNameEl, () => projects.get(project_id)?.name ?? '',
    title => structureCall('set_title', { id: project_id, title }));
}

projectFoldBtn.addEventListener('click', () => {
  const unfold = !projectUnfolded();
  projectFoldBtn.setAttribute('aria-expanded', String(unfold));
  projectFoldBtn.title = unfold ? 'Hide project details' : 'Show project details';
  updateProjectDesc();
});

$('project-rename-btn').addEventListener('click', editProjectName);

// Archive toggles: an archived project's button is pressed and unarchives.
const projectArchiveBtn = $('project-archive-btn');
projectArchiveBtn.addEventListener('click', () => {
  if (projectArchiveBtn.getAttribute('aria-disabled') !== 'true') toggleArchive(descProjectId);
});

// The buttons follow the selected project: what the host may do with its
// file, and whether it is archived.
store.subscribe(
  [state => state.selected_project, state => state.projects_rev],
  ([selected_project]) => {
    const project = selected_project ? projects.get(selected_project) : null;
    const archived = Boolean(project?.archived);
    projectArchiveBtn.setAttribute('aria-disabled', String(!project?.writable));
    projectArchiveBtn.setAttribute('aria-pressed', String(archived));
    projectArchiveBtn.title = archived ? 'Unarchive project' : 'Archive project';
  });

// Another project selected: the fold state stays. All Projects and No
// project have no details.
store.subscribe(state => state.selected_project, () => {
  descProjectId = projectScope();
  projectFoldBtn.hidden = !descProjectId;
  updateProjectDesc();
});

// The project header's Archive: the host writes `archived` (unarchiving
// removes it) and pushes the project back, which moves its pill between
// the active and the archived projects. The selection stays.
function toggleArchive(project_id) {
  const archived = !projects.get(project_id)?.archived;
  structureCall('set_archived', { id: project_id, archived });
}

// ----------------------------------------------------------------------------
// Labels (`tags`) as chips, each with an x that calls `onChange(tags)` with
// the label removed; `removeTitle(tag)` titles the x.
function tagChips(tags, onChange, removeTitle) {
  return tags.map(tag => {
    const chip = accEl('span', 'chip chip--tag', tag);
    const remove = accEl('button', 'chip__remove', '\u00d7');
    remove.type = 'button';
    remove.title = removeTitle(tag);
    remove.addEventListener('click', () => onChange(tags.filter(t => t !== tag)));
    chip.append(remove);
    return chip;
  });
}
