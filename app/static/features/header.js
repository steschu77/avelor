// The lens header (panel.md#projects): the selected project's name, the
// words filtering (ui.md#words) and the active labels with the "+" that
// filters by another one (ui.md#vault-panel), and the project's details -- a toolbar (rename, edit
// description, archive, delete) and the description, an editor bound like
// any document's (saving.md). Also `leaveEditModes`, which takes every
// editor out of edit mode, the cards' and the description (closing the
// app, switching the vault).

import { $, accEl, oncePerFrame } from '../core/dom.js';
import { titled } from '../core/dom.js';
import { t } from '../core/text.js';
import { createEditor } from '../editor/editor.js';
import { serializeMarkdown } from '../editor/markdown.js';
import { NO_WORDS, store } from '../model/state.js';
import { NO_PROJECT, entities, matchesFilter, projects } from '../model/entities.js';
import { projectScope } from '../model/entities.js';
import {
  bindDocContent, editorRoots, onEdited, trySave,
} from '../model/documents.js';
import { reportEditing } from '../model/documents.js';
import { structureCall } from '../model/structure.js';
import { armedButton } from '../ui/confirm.js';
import { iconEl } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import { createEditorToolbar } from '../ui/toolbar.js';
import { editTextInPlace } from '../ui/field.js';
import { openLink } from './selection.js';
import { selectFilter } from './selection.js';
import { leaveCardEditMode } from './panel/panel.js';
import { labelChips } from './panel/panel.js';

const projectNameEl = $('project-name');
const projectDescEl = $('project-desc');

// The header names the selected project, No project or All Projects;
// `projects_rev` catches a rename. A name being edited in place is left
// alone.
store.subscribe(
  [state => state.selected_project, state => state.projects_rev],
  ([selected_project]) => {
    if (projectNameEl.hasAttribute('contenteditable')) return;
    if (selected_project === NO_PROJECT) projectNameEl.textContent = t('sidebar.no_project');
    else if (selected_project) projectNameEl.textContent = projects.get(selected_project)?.name ?? '';
    else projectNameEl.textContent = t('sidebar.all_projects');
  });

// After the name, the header names the words filtering and the active
// labels, each with an x that takes it out of the filter, and a "+" that
// adds a label, while any label is in use: looked at once per frame, not
// once per entity of an indexing.
const labelFilterEl = $('label-filter');
const labelAddBtn = $('label-add-btn');
store.subscribe([state => state.selected_labels, state => state.word_filter],
  ([selected_labels, word_filter]) => labelFilterEl.replaceChildren(
    ...(word_filter ? [wordsChip(word_filter.text)] : []),
    ...labelChips(selected_labels,
      next => selectFilter({ selected_labels: next }),
      label => t('header.stop_label', { label }))));

// The words' chip: the words searched, and an x that empties the box.
function wordsChip(text) {
  const chip = accEl('span', 'chip chip--words');
  chip.title = t('header.every_word');
  const remove = accEl('button', 'chip__remove', '\u00d7');
  remove.type = 'button';
  titled(remove, t('header.clear_filter'));
  remove.addEventListener('click', () => selectFilter({ ...NO_WORDS }));
  chip.append(iconEl('#i-search'), accEl('span', '', text.trim()), remove);
  return chip;
}
const scheduleLabelAdd = oncePerFrame(() => {
  labelAddBtn.hidden = ![...entities.values()].some(doc => doc.labels?.length);
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
    for (const label of doc.labels ?? []) counts.set(label, (counts.get(label) ?? 0) + (shown ? 1 : 0));
  }
  const items = [...counts.keys()].filter(label => !state.selected_labels.includes(label))
    .sort((a, b) => a.localeCompare(b))
    .map(label => ({
      label: label,
      hint: String(counts.get(label)),
      dim: !counts.get(label),
      run: () => selectFilter({ selected_labels: [...store.getState().selected_labels, label] }),
    }));
  showMenu(labelAddBtn, items.length ? items : [{ label: t('header.no_other_labels'), dim: true }],
    { head: t('header.filter_label'), filter: { placeholder: t('header.filter_labels') } });
});

// The selected project's details under the header, shown while the fold
// button (the chevron) is unfolded: a toolbar (rename, edit description,
// archive, delete) and the description -- its file's body in an editor, loaded and
// saved like any document (saving.md). The description pen toggles edit
// mode, Escape leaves it. The description is hidden while empty and not
// being edited.
const projectFoldBtn = $('project-fold-btn');
const projectToolsEl = $('project-tools');
const projectDescBtn = $('project-desc-btn');
let descProjectId = null; // the project the details are bound to
let unbindProjectDesc = null;
const projectDescToolbar = createEditorToolbar();
projectDescEl.before(projectDescToolbar);
const projectDescEditor = createEditor(projectDescEl, {
  toolbar: projectDescToolbar,
  onEscape: () => {
    setProjectDescEditMode(false);
    projectDescBtn.focus();
  },
  onChange: () => onEdited(descProjectId),
  onLinkClick: openLink,
});

function projectUnfolded() {
  return projectFoldBtn.getAttribute('aria-expanded') === 'true';
}

function updateProjectDesc() {
  const shown = Boolean(descProjectId) && projectUnfolded();
  projectToolsEl.hidden = !shown;
  projectDescEl.hidden = !shown || (!projectDescEditor.isEditable()
    && serializeMarkdown(projectDescEl).trim() === '');
}

function setProjectDescEditMode(on) {
  if (!descProjectId) return;
  if (!on && projectDescEditor.isEditable()) trySave(descProjectId);
  projectDescEditor.setEditable(on);
  projectDescBtn.setAttribute('aria-pressed', String(on));
  updateProjectDesc();
  reportEditing();
}

// Every editor leaves edit mode -- the card being edited, the project
// description -- saving as it does (closing the app, switching the vault).
export function leaveEditModes() {
  leaveCardEditMode();
  if (projectDescEditor.isEditable()) setProjectDescEditMode(false);
}

// Rename the selected project in the header itself, like its sidebar pill
// (`editProject`): the host writes the new title and pushes the project
// back as `entity`, which renames the header.
function editProjectName() {
  const project_id = descProjectId;
  if (!project_id) return;
  editTextInPlace(projectNameEl, () => projects.get(project_id)?.name ?? '',
    title => structureCall('set_title', { id: project_id, title }, () => store.getState()));
}

// Folding while the description is edited saves it first.
function foldProjectDetails(unfold) {
  if (!unfold) setProjectDescEditMode(false);
  projectFoldBtn.setAttribute('aria-expanded', String(unfold));
  projectFoldBtn.title = unfold ? t('header.hide_details') : t('header.show_details');
  updateProjectDesc();
}

projectFoldBtn.addEventListener('click', () => foldProjectDetails(!projectUnfolded()));

// A project New project just made from a blueprint, selected: its details
// unfold to be filled in, and its name opens to be edited in place
// (features/sidebar.js).
export function openNewProject() {
  foldProjectDetails(true);
  editProjectName();
}

$('project-rename-btn').addEventListener('click', editProjectName);
projectDescBtn.addEventListener('click',
  () => setProjectDescEditMode(!projectDescEditor.isEditable()));

// Archive toggles: an archived project's button is pressed and unarchives.
const projectArchiveBtn = $('project-archive-btn');
projectArchiveBtn.addEventListener('click', () => {
  if (projectArchiveBtn.getAttribute('aria-disabled') !== 'true') toggleArchive(descProjectId);
});

// Delete: two clicks (`armedButton`). The description is saved first, so
// if the delete fails no edit is lost; the host takes the project out of
// its members and moves its file to `.trash`, and `onEntityDeleted` clears
// the filter.
const projectDeleteBtn = $('project-delete-btn');
armedButton(projectDeleteBtn, t('header.delete_armed'), () => {
  const project_id = descProjectId;
  setProjectDescEditMode(false);
  structureCall('delete_entity', { id: project_id }, () => store.getState());
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
    projectArchiveBtn.title = archived ? t('header.unarchive') : t('header.archive');
    projectDeleteBtn.setAttribute('aria-disabled', String(!project?.deletable));
  });

// Another project selected: the outgoing one is saved first. The fold state
// stays. All Projects and No project have no details.
store.subscribe(state => state.selected_project, () => {
  if (descProjectId) {
    setProjectDescEditMode(false);
    unbindProjectDesc();
    editorRoots.delete(descProjectId);
  }
  descProjectId = projectScope();
  projectFoldBtn.hidden = !descProjectId;
  projectDescEditor.setHTML('');
  if (descProjectId) {
    editorRoots.set(descProjectId, projectDescEl);
    unbindProjectDesc = bindDocContent(
      descProjectId, projectDescEditor, projectDescEl, updateProjectDesc);
  }
  updateProjectDesc();
});

// The project header's Archive: the host writes `archived` (unarchiving
// removes it) and pushes the project back, which moves its pill between
// the active and the archived projects. The selection stays.
function toggleArchive(project_id) {
  const archived = !projects.get(project_id)?.archived;
  structureCall('set_archived', { id: project_id, archived }, () => store.getState());
}
