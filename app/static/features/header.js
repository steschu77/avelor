// The lens header (panel.md#projects): the selected project's name with its
// pen, the words filtering (ui.md#words) and the active labels with the "+"
// that filters by another one (ui.md#vault-panel), its menu at the end (the
// pill's, features/projects.js, with Blueprint and Link to), and the
// project's details -- its own labels and link, and the description, an
// editor bound like any document's (saving.md) with its pen. Also
// `leaveEditModes`, which takes every editor out of edit mode, the cards'
// and the description (closing the app, switching the vault).

import { $, accEl, oncePerFrame } from '../core/dom.js';
import { titled } from '../core/dom.js';
import { compareText } from '../core/format.js';
import { t } from '../core/text.js';
import { createEditor } from '../editor/editor.js';
import { serializeMarkdown } from '../editor/markdown.js';
import { NO_WORDS, savePrefs, store } from '../model/state.js';
import { NO_PROJECT, entities, matchesFilter, projects } from '../model/entities.js';
import { projectScope } from '../model/entities.js';
import {
  bindDocContent, editorRoots, onEdited, trySave,
} from '../model/documents.js';
import { beginEditSession, endEditSession, reportEditing } from '../model/documents.js';
import { setLabelsLocal, setLinkLocal, setProjectLocal, structureCall } from '../model/structure.js';
import { iconEl } from '../ui/icons.js';
import { showMenu } from '../ui/menu.js';
import { createEditorToolbar } from '../ui/toolbar.js';
import { editTextInPlace } from '../ui/field.js';
import { openLink } from './selection.js';
import { selectFilter } from './selection.js';
import { showBlueprint } from './blueprint.js';
import { BLUEPRINTS, checksOf, detailsOpen, detailsPrefs, factsOf, hasBlueprint, withBlueprint } from '../model/blueprint.js';
import { tripLine } from '../model/trip.js';
import { factText } from './facts.js';
import { leaveCardEditMode } from './panel/panel.js';
import { addChip, labelChips, linkChip } from './panel/panel.js';
import { linkMenuChip, showLinkMenu } from './linked.js';
import { vaultSources } from './sources.js';
import { projectMenu } from './projects.js';

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

// After the name, the header names the words filtering, the source and the
// active labels, each with an x that takes it out of the filter, and a "+"
// that adds a label, while any label is in use: looked at once per frame,
// not once per entity of an indexing. After more than one, Clear filters
// ends them all; the project, the head's name, stays.
const labelFilterEl = $('label-filter');
const labelAddBtn = $('label-add-btn');
const clearFiltersBtn = $('clear-filters-btn');
store.subscribe(
  [state => state.selected_labels, state => state.word_filter, state => state.selected_source,
    state => state.watches_rev, state => state.accounts_rev],
  ([selected_labels, word_filter, selected_source]) => {
    const chips = [
      ...(word_filter ? [wordsChip(word_filter.text)] : []),
      ...sourceChips(selected_source),
      ...labelChips(selected_labels,
        next => selectFilter({ selected_labels: next }),
        label => t('header.stop_label', { label })),
    ];
    labelFilterEl.replaceChildren(...chips);
    clearFiltersBtn.hidden = chips.length < 2;
  });
clearFiltersBtn.addEventListener('click', () => selectFilter({ selected_source: null, selected_labels: [], ...NO_WORDS }));

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

// The source's chip: its icon and name, and an x that ends the filter; none
// while the source isn't listed yet.
function sourceChips(selected_source) {
  const source = selected_source && vaultSources().find(s => s.key === selected_source);
  if (!source) return [];
  const chip = accEl('span', 'chip chip--source');
  const remove = accEl('button', 'chip__remove', '\u00d7');
  remove.type = 'button';
  titled(remove, t('header.stop_source', { source: source.name }));
  remove.addEventListener('click', () => selectFilter({ selected_source: null }));
  chip.append(iconEl(source.icon), accEl('span', '', source.name), remove);
  return [chip];
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
    .sort(compareText)
    .map(label => ({
      label: label,
      hint: String(counts.get(label)),
      dim: !counts.get(label),
      run: () => selectFilter({ selected_labels: [...store.getState().selected_labels, label] }),
    }));
  showMenu(labelAddBtn, items.length ? items : [{ label: t('header.no_other_labels'), dim: true }],
    { head: t('header.filter_label'), filter: { placeholder: t('header.filter_labels') } });
});

// The selected project's details under the header, shown while Details,
// the fold under the head, is unfolded: its own labels and link, the
// description -- its file's body in an editor, loaded and saved like any
// document (saving.md) -- and its blueprint (features/blueprint.js). The
// description's pen, at the end of its first line, toggles edit mode,
// Escape leaves it. While it is empty and not being edited, "+ Description"
// stands in its place, where the host can write the project.
const projectFoldBtn = $('project-fold-btn');
const projectAboutEl = $('project-about');
const projectDescBtn = $('project-desc-btn');
const projectDescAddBtn = $('project-desc-add');
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

// The fold is the project's own: as it was left for it, else unfolded for
// a project with a blueprint (`detailsOpen`, kept in the preferences).
function projectUnfolded() {
  const project = projects.get(descProjectId);
  return Boolean(project) && detailsOpen(store.getState().prefs, project.id, project.project);
}

function updateProjectDesc() {
  const shown = Boolean(descProjectId) && projectUnfolded();
  projectFoldBtn.setAttribute('aria-expanded', String(shown));
  scheduleFolded();
  const writable = Boolean(projects.get(descProjectId)?.writable);
  renderProjectProps(shown);
  const empty = !projectDescEditor.isEditable() && serializeMarkdown(projectDescEl).trim() === '';
  projectAboutEl.hidden = !shown || empty;
  projectDescBtn.hidden = !writable;
  projectDescAddBtn.hidden = !shown || !empty || !writable;
  showBlueprint(shown ? descProjectId : null);
}

// The project's own labels and link (vault.md#metadata) in its details,
// under the head: its labels as a card's chips with an x and "+" (labels
// only, `addChip`), its link as a chip with an x, or "+" that picks one
// (`linkMenuChip`, offering entities beyond the project itself), while the
// host can write the project (`set_labels`, `set_link`). The label chips
// after the name are the filter's.
const projectPropsEl = $('project-props');

function renderProjectProps(shown) {
  const project = shown ? projects.get(descProjectId) : null;
  const chips = [];
  if (project) {
    const { id, labels, linked_to: to, writable } = project;
    chips.push(...labelChips(labels, writable ? next => setLabelsLocal(id, next) : null));
    if (writable) chips.push(addChip({ projects: [], labels }, { onAddLabel: label => setLabelsLocal(id, [...labels, label]) }));
    if (to) chips.push(linkChip(to, { onRemove: writable ? () => setLinkLocal(id, null) : null }));
    else if (writable) chips.push(linkMenuChip(t('header.link_to'), linkPick(id)));
  }
  projectPropsEl.replaceChildren(...chips);
  projectPropsEl.hidden = !chips.length;
}

function setProjectDescEditMode(on) {
  if (!descProjectId) return;
  if (!on && projectDescEditor.isEditable()) {
    trySave(descProjectId);
    endEditSession(descProjectId);
  }
  if (on && !projectDescEditor.isEditable()) beginEditSession(descProjectId);
  projectDescEditor.setEditable(on);
  projectDescBtn.setAttribute('aria-pressed', String(on));
  updateProjectDesc();
  reportEditing();
}

// Every editor leaves edit mode -- the card being edited, the project
// description -- saving as it does (closing the app, switching the vault,
// an undo).
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
  if (!descProjectId) return;
  if (!unfold) setProjectDescEditMode(false);
  savePrefs(detailsPrefs(store.getState().prefs, descProjectId, unfold));
  updateProjectDesc();
}

projectFoldBtn.addEventListener('click', () => foldProjectDetails(!projectUnfolded()));

// While the details are folded, what matters stands outside them: Missing
// and Next beside the name, each while it has any, a click unfolding the
// details with that list open; and after Details what the project is, a
// trip's days, nights and route (`tripLine`), another's first two facts
// with a value as their rows say them. Looked at once per frame.
const projectChecksEl = $('project-checks');
const projectSummaryEl = $('project-summary');
const scheduleFolded = oncePerFrame(() => {
  const project = projects.get(descProjectId);
  const folded = Boolean(project) && !projectUnfolded();
  const chips = [];
  if (folded && hasBlueprint(project.project)) {
    const { missing, next } = checksOf(project.id, project.project);
    if (missing.length) chips.push(checkChip(t('bp.missing'), missing.length, 'missing'));
    if (next.length) chips.push(checkChip(t('props.next'), next.length, 'next'));
  }
  projectChecksEl.replaceChildren(...chips);
  projectChecksEl.hidden = !chips.length;
  const summary = folded ? projectSummary(project) : '';
  projectSummaryEl.textContent = summary ? `\u00b7 ${summary}` : '';
});
// The details follow the project, its members and the preferences, once per
// frame, not once per entity of an indexing.
const scheduleProjectDesc = oncePerFrame(updateProjectDesc);
store.subscribe([state => state.projects_rev, state => state.entities_rev, state => state.prefs], () => scheduleProjectDesc());

function checkChip(label, n, list) {
  const chip = accEl('button', 'tool label', label);
  chip.type = 'button';
  chip.append(accEl('span', 'meta', String(n)));
  chip.addEventListener('click', () => {
    foldProjectDetails(true);
    showBlueprint(descProjectId, list);
  });
  return chip;
}

function projectSummary({ id, project }) {
  const trip = tripLine(id, project);
  if (trip !== null) return trip;
  const set = value => value != null && value !== '' && !(Array.isArray(value) && !value.length);
  return factsOf(project).filter(fact => set(fact.value)).slice(0, 2)
    .map(fact => factText(fact, project)).join(' \u00b7 ');
}

// A project New project just made from a blueprint, selected: its details
// unfold to be filled in, and its name opens to be edited in place
// (features/sidebar.js).
export function openNewProject() {
  foldProjectDetails(true);
  editProjectName();
}

const projectRenameBtn = $('project-rename-btn');
projectRenameBtn.addEventListener('click', editProjectName);
projectDescBtn.addEventListener('click',
  () => setProjectDescEditMode(!projectDescEditor.isEditable()));
projectDescAddBtn.addEventListener('click', () => {
  setProjectDescEditMode(true);
  projectDescEl.focus();
});

// The project's menu: its pill's (`projectMenu`), the description saved
// before a delete, with two of the header's own. Blueprint: a project
// without one gets a shipped blueprint, from a submenu as New project's
// (features/sidebar.js); its name, and the checks and labels it brings, go
// into the project's block (`withBlueprint`), which keeps what it has. A
// blueprint stays: once set, the entry is gone. Link to: a project without
// a link picks one, as its "+" among its details does.
const projectMoreBtn = $('project-more-btn');
projectMoreBtn.addEventListener('click', () => {
  const project = projects.get(descProjectId);
  if (!project) return;
  const { id, writable, linked_to } = project;
  const more = [];
  if (writable && !project.project.blueprint && BLUEPRINTS.length) {
    more.push({
      label: t('project.blueprint'),
      items: BLUEPRINTS.map(blueprint => ({
        label: blueprint.title, icon: blueprint.icon, desc: blueprint.note,
        run: () => setProjectLocal(id, withBlueprint(project.project, blueprint)),
      })),
    });
  }
  if (writable && !linked_to) {
    more.push({
      label: t('project.link_to'),
      run: () => showLinkMenu(projectMoreBtn, linkPick(id)),
    });
  }
  showMenu(projectMoreBtn, projectMenu(id, { more, beforeDelete: () => setProjectDescEditMode(false) }));
});

// What a project may link to: the entities beyond the project itself.
function linkPick(id) {
  return { target: id, scope: state => ({ ...state, selected_project: null }), onPick: other => setLinkLocal(id, other) };
}

// The pen and the menu follow the selected project: the pen while the host
// may write its file, the menu named after it.
store.subscribe(
  [state => state.selected_project, state => state.projects_rev],
  () => {
    const project = projects.get(projectScope());
    projectRenameBtn.hidden = !project?.writable;
    projectMoreBtn.hidden = !project;
    if (project) titled(projectMoreBtn, t('menu.actions', { name: project.name || t('untitled') }));
  });

// Another project selected: the outgoing one is saved first; its fold is
// its own. All Projects and No project have no details.
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
