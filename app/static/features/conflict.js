// The conflict dialog (index.html #conflict-dialog, saving.md#the-dialog):
// a file changed outside the app, so the host didn't write it. The
// store's `conflicts` (oldest first) are shown one at a time in a modal
// dialog over the shaded app. A conflict isn't dismissed, only answered:
// Use theirs reads the file again (`reloadEntity`). It ends with
// `endConflict` once the host has done it, and the dialog moves on to the
// next. model/documents.js keeps the conflicts (`showConflict`,
// `endConflict`).

import { $ } from '../core/dom.js';
import { DATE_FORMAT, TIME_FORMAT } from '../core/format.js';
import { store } from '../model/state.js';
import { entities, projects, threads } from '../model/entities.js';
import { reloadEntity } from '../model/structure.js';
import { KIND_ICONS } from '../ui/icons.js';

const conflictDialogEl = $('conflict-dialog');
const conflictTheirsEl = $('conflict-theirs');

// "at 14:32" today, else "on <date> at 14:32".
function conflictTime(iso) {
  const date = new Date(iso);
  const time = TIME_FORMAT.format(date);
  if (date.toDateString() === new Date().toDateString()) return `at ${time}`;
  return `on ${DATE_FORMAT.format(date)} at ${time}`;
}

function conflictText({ changed }) {
  if (changed) {
    return `The file on disk changed ${conflictTime(changed)}, so your change wasn't ` +
      'saved. Load the version on disk, then make the change again.';
  }
  return 'The file was deleted outside the app, so your change wasn\'t saved.';
}

function renderConflict(conflicts) {
  const conflict = conflicts[0];
  if (!conflict) {
    if (conflictDialogEl.open) conflictDialogEl.close();
    return;
  }

  const { id, changed } = conflict;
  const entity = entities.get(id);
  const meta = entity ?? projects.get(id) ?? threads.get(id);
  const kind = entity?.kind ?? (projects.has(id) ? 'project' : 'thread');
  const outside = changed ? 'changed outside the app' : 'deleted outside the app';

  $('conflict-icon').firstChild.setAttribute('href', KIND_ICONS[kind]?.icon ?? '');
  $('conflict-kind').textContent = kind;
  $('conflict-title').textContent = `${meta?.title || meta?.name || 'Untitled'} was ${outside}`;
  const pathEl = $('conflict-path');
  pathEl.textContent = meta?.path ?? '';
  pathEl.title = meta?.path ?? '';
  $('conflict-text').textContent = conflictText(conflict);

  conflictTheirsEl.title = changed ? 'Load the version on disk' : 'Remove it here too';

  if (!conflictDialogEl.open) {
    conflictDialogEl.showModal();
    // Not a button: a stray Enter mustn't decide.
    conflictDialogEl.focus();
  }
}

conflictTheirsEl.addEventListener('click', () => {
  const conflict = store.getState().conflicts[0];
  if (conflict) reloadEntity(conflict.id);
});

// The dialog has to be answered: Escape doesn't close it, and should the
// browser close it anyway, it opens again while a conflict waits.
conflictDialogEl.addEventListener('cancel', ev => ev.preventDefault());
conflictDialogEl.addEventListener('close', () => renderConflict(store.getState().conflicts));

store.subscribe(state => state.conflicts, renderConflict);
