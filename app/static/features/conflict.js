// The conflict dialog (index.html #conflict-dialog, saving.md#the-dialog):
// a file changed outside the app, so the host didn't write it. The
// store's `conflicts` (oldest first) are shown one at a time in a modal
// dialog over the shaded app. A conflict isn't dismissed, only answered:
// - Use theirs reads the file again (`reloadEntity`), dropping the edits
//   made here;
// - Use ours -- offered when a document's content was refused -- saves the
//   version edited here over it (`overwriteDocContent`).
// It ends with `endConflict` once the host has done it, and the dialog
// moves on to the next. model/documents.js keeps the conflicts
// (`showConflict`, `endConflict`) and has `overwriteDocContent`.

import { $ } from '../core/dom.js';
import { DATE_FORMAT, TIME_FORMAT } from '../core/format.js';
import { t } from '../core/text.js';
import { store } from '../model/state.js';
import { entities, projects, threads } from '../model/entities.js';
import { overwriteDocContent } from '../model/documents.js';
import { reloadEntity } from '../model/structure.js';
import { docIcon, kindName } from '../ui/icons.js';

const conflictDialogEl = $('conflict-dialog');
const conflictTheirsEl = $('conflict-theirs');
const conflictOursEl = $('conflict-ours');

// "at 14:32" today, else "on <date> at 14:32".
function conflictTime(iso) {
  const date = new Date(iso);
  const time = TIME_FORMAT.format(date);
  if (date.toDateString() === new Date().toDateString()) return t('conflict.when_today', { time });
  return t('conflict.when_day', { date: DATE_FORMAT.format(date), time });
}

function conflictText({ content, changed }) {
  if (content && changed) return t('conflict.changed_editing', { when: conflictTime(changed) });
  if (content) return t('conflict.deleted_editing');
  if (changed) return t('conflict.changed_unsaved', { when: conflictTime(changed) });
  return t('conflict.deleted_unsaved');
}

function renderConflict(conflicts) {
  const conflict = conflicts[0];
  if (!conflict) {
    if (conflictDialogEl.open) conflictDialogEl.close();
    return;
  }

  const { id, content, changed } = conflict;
  const entity = entities.get(id);
  const meta = entity ?? projects.get(id) ?? threads.get(id);
  const kind = entity?.kind ?? (projects.has(id) ? 'project' : 'thread');

  $('conflict-icon').firstChild.setAttribute('href', docIcon(entity ?? { kind }));
  $('conflict-kind').textContent = kindName(kind);
  const title = meta?.title || meta?.name || t('untitled');
  $('conflict-title').textContent = changed
    ? t('conflict.title_changed', { title }) : t('conflict.title_deleted', { title });
  const pathEl = $('conflict-path');
  pathEl.textContent = meta?.path ?? '';
  pathEl.title = meta?.path ?? '';
  $('conflict-text').textContent = conflictText(conflict);

  conflictTheirsEl.title = changed
    ? (content ? t('conflict.theirs_load_drop') : t('conflict.theirs_load'))
    : (content ? t('conflict.theirs_remove_drop') : t('conflict.theirs_remove'));
  conflictOursEl.hidden = !content;
  conflictOursEl.title = changed ? t('conflict.ours_over') : t('conflict.ours_again');

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

conflictOursEl.addEventListener('click', () => {
  const conflict = store.getState().conflicts[0];
  if (conflict) overwriteDocContent(conflict.id);
});

// The dialog has to be answered: Escape doesn't close it, and should the
// browser close it anyway, it opens again while a conflict waits.
conflictDialogEl.addEventListener('cancel', ev => ev.preventDefault());
conflictDialogEl.addEventListener('close', () => renderConflict(store.getState().conflicts));

store.subscribe(state => state.conflicts, renderConflict);
