// The vault's data as the page knows it (architecture.md#page-state): the
// Maps of the entities and projects, written only from the host's events
// (model/events.js); the selectors over them.

import { store } from './state.js';

export const projects = new Map(); // project_id -> project data
export const entities = new Map(); // doc_id -> document data (every non-{project, thread} entity)

// ----------------------------------------------------------------------------
// The projects as the vault panel lists them: the starred first, otherwise
// in the order they arrived, the archived ones after the others.
export function projectsInOrder() {
  const sorted = [...projects.values()].sort((a, b) => b.starred - a.starred);
  return [...sorted.filter(p => !p.archived), ...sorted.filter(p => p.archived)];
}

// `selected_project` while the vault panel's "No project" is selected: the
// entities in no project of the vault.
export const NO_PROJECT = 'none';

// The projects of `doc` that are in the vault.
export function projectsOf(doc) {
  return (doc.projects ?? []).filter(id => projects.has(id));
}

// ----------------------------------------------------------------------------
// The vault panel's filter: `doc` is in `selected_project` (if one is set;
// in none for `NO_PROJECT`). The lenses show only matching entities.
export function matchesFilter(doc, { selected_project } = store.getState()) {
  if (selected_project === NO_PROJECT) {
    if (projectsOf(doc).length) return false;
  } else if (selected_project && !doc.projects?.includes(selected_project)) {
    return false;
  }
  return true;
}
