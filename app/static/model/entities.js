// The vault's data as the page knows it (architecture.md#page-state): the
// Maps of the entities, projects and threads, written only from the host's
// events (model/events.js); the selectors over them (a thread's members in
// thread order); and the selection, published minus what no longer exists
// (`commitExistingSelection`).

import { store } from './state.js';

export const projects = new Map(); // project_id -> project data
export const threads = new Map(); // thread_id -> thread data
export const entities = new Map(); // doc_id -> document data (every non-{project, thread} entity)

// ----------------------------------------------------------------------------
// The thread shown for `doc_id`: the first of the threads it's a member of
// (`message.threads`) that is in the vault, or undefined for an entity
// outside any thread. One it still names after a restore from the trash,
// dissolved meanwhile, is skipped (vault.md#trash).
export function threadOf(doc_id) {
  return knownThreads(entities.get(doc_id))[0];
}

// The threads `doc` is a member of that are in the vault, in the order of
// its `threads`; none for an unknown entity.
export function knownThreads(doc) {
  return (doc?.threads ?? []).filter(thread_id => threads.has(thread_id));
}

// ----------------------------------------------------------------------------
export function getMeta(doc_id) {
  return entities.get(doc_id);
}

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

// The projects of `doc` that are in the vault (a restored file may still
// name one deleted since, vault.md#trash).
export function projectsOf(doc) {
  return (doc.projects ?? []).filter(id => projects.has(id));
}

// The names of `doc`'s projects, joined, while no project is selected (the
// filter names it already): the dashboard's cards.
export function projectNames(doc, { selected_project } = store.getState()) {
  if (selected_project) return '';
  return projectsOf(doc).map(id => projects.get(id).name).filter(Boolean).join(' \u00b7 ');
}

// The project something new starts in, and the header shows: the selected
// one, null for All Projects and No project.
export function projectScope({ selected_project } = store.getState()) {
  return selected_project === NO_PROJECT ? null : selected_project;
}

// ----------------------------------------------------------------------------
// The vault panel's filter: `doc` is in `selected_project` (if one is set;
// in none for `NO_PROJECT`), and carries every one of `selected_labels`.
// The lenses show only matching entities; thread membership is never
// filtered.
export function matchesFilter(doc, { selected_project, selected_labels } = store.getState()) {
  if (selected_project === NO_PROJECT) {
    if (projectsOf(doc).length) return false;
  } else if (selected_project && !doc.projects?.includes(selected_project)) {
    return false;
  }
  return selected_labels.every(tag => doc.tags?.includes(tag));
}

// A thread is in the filter while one of its members is.
export function threadMatchesFilter(thread_id, state = store.getState()) {
  return threadMembers(thread_id).some(doc => matchesFilter(doc, state));
}

// ----------------------------------------------------------------------------
// The one exit of every thread-membership change: publishes the new
// selection and bumps `threads_rev`, so `renderPanel` rebuilds the cards
// even when neither selection changed (e.g. a member was added), and the
// table shows the threads as they are now (app.js).
function commitThreadChange({ selected_entity, selected_thread }) {
  const { threads_rev } = store.getState();
  store.setState({ selected_entity, selected_thread, threads_rev: threads_rev + 1 });
}

// ----------------------------------------------------------------------------
// Publish `selection`, minus anything that no longer exists.
export function commitExistingSelection({ selected_entity, selected_thread }) {
  commitThreadChange({
    selected_entity: entities.has(selected_entity) ? selected_entity : null,
    selected_thread: threads.has(selected_thread) ? selected_thread : null,
  });
}

// ----------------------------------------------------------------------------
// Thread order (vault.md#thread): by date, ties broken by id --
// UUIDv7, so string order is creation order. Members without a date first.
function compareMembers(a, b) {
  const ta = a.date?.getTime() ?? -Infinity;
  const tb = b.date?.getTime() ?? -Infinity;
  if (ta !== tb) return ta < tb ? -1 : 1;
  return a.doc_id < b.doc_id ? -1 : a.doc_id > b.doc_id ? 1 : 0;
}

// ----------------------------------------------------------------------------
// The members of the thread `threadId`, in thread order.
export function threadMembers(threadId) {
  return [...entities.values()]
    .filter(doc => doc.threads?.includes(threadId))
    .sort(compareMembers);
}

// A thread's projects are its members' (vault.md#thread): those in the
// vault, each once, in thread order.
export function threadProjects(threadId) {
  return [...new Set(threadMembers(threadId).flatMap(projectsOf))];
}

// ----------------------------------------------------------------------------
// The projects something new for `target` (an entity or thread id, or
// none) starts in: its own (a thread's, its members'), else the one
// selected in the sidebar (`projectScope`), which is then its `scope`. A
// task's in the Task tab.
export function projectsFor(target) {
  const doc = target ? getMeta(target) : null;
  const own = doc ? doc.projects ?? [] : threads.has(target) ? threadProjects(target) : [];
  if (own.length) return { projects: own, scope: null };
  const scope = projectScope();
  return { projects: scope ? [scope] : [], scope };
}

// ----------------------------------------------------------------------------
// `Source` serializes as a tagged object (e.g. `{ scan: {...} }`), not a
// bare string, or as null when an entity has none.
function sourceKind(source) {
  return typeof source === 'string' ? source : Object.keys(source ?? {})[0];
}

// ----------------------------------------------------------------------------
// Whether `doc` is a chat with an AI: a conversation from the `ai` source.
export function isAiConversation(doc) {
  return doc?.kind === 'conversation' && sourceKind(doc.source) === 'ai';
}
