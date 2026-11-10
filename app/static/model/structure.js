// The structural edits (panel.md#saving-structure, api.md#structure):
// threads, projects, titles, labels, links, a type and a task's fields,
// deletes and reloads, each a host round trip that ends in a published
// selection (`commitExistingSelection`, model/entities.js); joining and
// leaving threads and projects, labels, links and the entity edits as
// requests of their own (`joinThreadLocal`, ..., `editEntityLocal`).
// The panel's drops and buttons (features/panel/panel.js), the table, the
// sidebar and the project header (features/sidebar.js, features/header.js)
// and the Linked tab send them.

import { rpcCall } from '../core/rpc.js';
import { t } from '../core/text.js';
import { store } from './state.js';
import { showError } from './state.js';
import { commitExistingSelection } from './entities.js';
import { entities } from './entities.js';
import { threadMembers, threads } from './entities.js';
import { showConflict } from './documents.js';
import { isDirty, startReload, trySave } from './documents.js';

// ----------------------------------------------------------------------------
// Structural edits are applied and saved by the host (panel.md#saving-structure),
// never here: each is a typed command (`structureCall`). The host pushes
// what it wrote -- `entity` for every written entity, `entity_deleted` for
// every trashed one -- before it answers, so once the answer arrives,
// `entities` and `threads` already show what is on disk, success or not.
// `select(id)` is the selection afterwards, `id` the answer's (a new thread
// or project, the id a reloaded file has now); after a failure or a
// conflict the selection stays. Each is applied minus anything that no
// longer exists; publishing it rebuilds the panel's cards (`threads_rev`).
// Without `select`, nothing is published: an entity's own fields, which the
// cards follow from its push (features/panel/panel.js's `refreshCardChips`),
// so a field keeps the focus. A conflict (a file changed outside the app,
// nothing written) opens the conflict dialog; unsaved edits of it are sent
// first, so their own conflict offers Use ours too. `edit_entities` names
// the entities it skipped instead: one changed outside the app is such a
// conflict, any other is reported. A failure shows in the status bar, in
// the host's words (core/rpc.js).
export function structureCall(cmd, args, select = null, step = undefined) {
  rpcCall(cmd, args, { step }).then(reply => {
    const skipped = reply?.skipped ?? [];
    const conflict = reply?.conflict ?? skipped.find(skip => skip.reason === 'changed_outside');
    if (conflict) {
      showConflict(conflict.id, false, conflict.changed);
      if (isDirty(conflict.id)) trySave(conflict.id);
    }
    for (const { id, reason } of skipped) {
      if (reason === 'changed_outside') continue;
      const title = entities.get(id)?.title || t('untitled');
      showError(t('structure.save_failed', { title, reason: SKIP_REASONS[reason] ?? reason }));
    }
    if (select) commitExistingSelection(conflict ? store.getState() : select(reply?.id ?? null));
  }, e => {
    if (select) commitExistingSelection(store.getState());
    throw e;
  });
}

// ----------------------------------------------------------------------------
// Reload: read `id`'s file again, dropping the unsaved changes. It ends
// like a structural edit (`structureCall`), so the panel is rebuilt --
// without the entity if its file is gone, with its new id if that was
// changed by hand. Saves stay held back until the content arrives
// (`onDocContent`), so none of the old content can follow the reload and
// overwrite the file after all.
export function reloadEntity(id) {
  startReload(id);
  const { selected_entity, selected_thread } = store.getState();
  structureCall('reload_entity', { id }, now => ({
    selected_entity: selected_entity === id ? now : selected_entity,
    selected_thread,
  }));
}

// ----------------------------------------------------------------------------
// The selection once `doc_id` is gone from `thread_id` -- it left, or was
// deleted -- computed from the thread as it is now, before the request:
// - thread dissolved: its remaining entity, shown standalone;
// - thread still there: it stays selected; if `doc_id` was the selected
//   entity, its neighbor in thread order (next, else previous) is selected
//   instead.
// For a standalone entity (`thread_id` null) the selection just drops it.
export function selectionWithout(doc_id, thread_id) {
  const members = thread_id ? threadMembers(thread_id) : [];
  const index = members.findIndex(m => m.doc_id === doc_id);
  const neighbor = members[index + 1] ?? members[index - 1];
  const remaining = members.find(m => m.doc_id !== doc_id);

  return () => {
    const { selected_entity, selected_thread } = store.getState();
    if (thread_id && !threads.has(thread_id)) {
      return {
        selected_entity: remaining?.doc_id ?? selected_entity,
        selected_thread: selected_thread === thread_id ? null : selected_thread,
      };
    }
    return {
      selected_entity: selected_entity === doc_id ? (neighbor?.doc_id ?? null) : selected_entity,
      selected_thread,
    };
  };
}

// ----------------------------------------------------------------------------
// Makes `doc_id` a member of `thread_id` and selects it there.
export function joinThreadLocal(doc_id, thread_id) {
  const doc = entities.get(doc_id);
  if (!doc || doc.threads?.includes(thread_id)) return;

  structureCall('join_thread', { id: doc_id, thread: thread_id },
    () => ({ selected_entity: doc_id, selected_thread: thread_id }));
}

// ----------------------------------------------------------------------------
// Removes `doc_id` from `thread_id` (the remove-from-thread button, or the
// card dragged onto the table). A thread left with fewer than two members
// is dissolved by the host. See `selectionWithout` for what is selected
// afterwards.
export function leaveThreadLocal(doc_id, thread_id) {
  const doc = entities.get(doc_id);
  if (!doc || !thread_id || !doc.threads?.includes(thread_id)) return;

  structureCall('leave_thread', { id: doc_id, thread: thread_id }, selectionWithout(doc_id, thread_id));
}

// ----------------------------------------------------------------------------
// Adds `doc_id` to the project `project_id` (dropped on its sidebar pill
// or its group row in the Documents lens); the selection stays.
export function joinProjectLocal(doc_id, project_id) {
  const doc = entities.get(doc_id);
  if (!doc || doc.projects?.includes(project_id)) return;

  structureCall('join_project', { id: doc_id, project: project_id }, () => store.getState());
}

// ----------------------------------------------------------------------------
// Removes `doc_id` from the project `project_id`; the selection stays.
export function leaveProjectLocal(doc_id, project_id) {
  const doc = entities.get(doc_id);
  if (!doc || !doc.projects?.includes(project_id)) return;

  structureCall('leave_project', { id: doc_id, project: project_id }, () => store.getState());
}

// ----------------------------------------------------------------------------
// Starts a new thread of the standalone entity `target_id` and the dropped
// `dropped_id`, titled after the target; its projects are its members'
// (vault.md#thread). The host writes the thread file next to the target.
// Selects the target in the new thread.
export function createThreadLocal(target_id, dropped_id) {
  const target = entities.get(target_id);
  const dropped = entities.get(dropped_id);
  if (!target || !dropped || target_id === dropped_id) return;

  const args = { title: target.title ?? '', members: [target_id, dropped_id] };
  structureCall('create_thread', args,
    thread_id => ({ selected_entity: target_id, selected_thread: thread_id }));
}

// ----------------------------------------------------------------------------
// Set `doc_id`'s labels to `labels`; the host writes them and pushes the
// entity back. The selection stays.
export function setLabelsLocal(doc_id, labels) {
  structureCall('set_labels', { id: doc_id, labels }, () => store.getState());
}

// Link `id` to `to`, null to remove its link; the host writes it and pushes
// the entity back. The selection stays.
export function setLinkLocal(id, to) {
  structureCall('set_link', { id, to }, () => store.getState());
}

// Link the transaction `transaction` of the statement `statement` to `to`, null to
// remove its link; the host writes the statement and pushes it back. The
// selection stays.
export function setTransactionLinkLocal(statement, transaction, to) {
  structureCall('set_transaction_link', { id: statement, transaction, to }, () => store.getState());
}

// Unpair the transfer whose half is the transaction `transaction` of the statement
// `statement`: both halves count in In and Out again. The host writes both
// statements and pushes them back. The selection stays.
export function unpairTransferLocal(statement, transaction) {
  structureCall('unpair_transfer', { id: statement, transaction }, () => store.getState());
}

// Pair the transaction `transaction` of the statement `statement` with `other`
// (`{ id: statement, transaction }`) as a transfer's two halves; the host writes
// both statements and pushes them back. The selection stays.
export function pairTransferLocal(statement, transaction, other) {
  structureCall('pair_transfer', { id: statement, transaction, other }, () => store.getState());
}

// Put the transactions `transactions` (`{ id: statement, transaction }` each) into the
// recurring payment `to`, null taking them out of theirs; the host writes
// each statement once and pushes it back. The selection stays. `step`: the
// group it is one step with (`newGroup`).
export function setTransactionRecurringPaymentLocal(transactions, to, step = undefined) {
  structureCall('set_transaction_recurring_payment', { transactions, to }, () => store.getState(), step);
}

// Write the recurring payment `id`'s role as `recurring` (vault.md#recurring);
// the host pushes the entity back. The selection stays.
export function setRecurringPaymentLocal(id, recurring) {
  structureCall('set_recurring_payment', { id, recurring_payment: recurring }, () => store.getState());
}

// Write `change` to the entity `id` (`edit_entities`, api.md#structure): a
// document's `{type}`, or a task's `{task: {status | due | assignee}}`,
// null removing it. The host pushes the entity back, which the cards, the
// lenses and the dashboard follow; the selection stays as it is.
export function editEntityLocal(id, change) {
  structureCall('edit_entities', { ids: [id], change });
}

// Why `edit_entities` skipped an entity, by the reason it names.
const SKIP_REASONS = {
  not_writable: t('structure.not_writable'),
  read_only_in_google: t('structure.read_only_in_google'),
  not_found: t('structure.not_found'),
  invalid_argument: t('structure.invalid_argument'),
};
