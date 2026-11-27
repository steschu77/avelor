// A project's actions (panel.md#projects), the menu of its pill and of its
// header (features/sidebar.js, features/header.js): New here, Star,
// Archive and Delete, last; the header adds its own between them.

import { t } from '../core/text.js';
import { store } from '../model/state.js';
import { projects } from '../model/entities.js';
import { structureCall } from '../model/structure.js';
import { newTypeItems } from './new.js';

// The menu of the project `id`: `more`, the header's entries before
// Archive; `beforeDelete`, what goes first (the header saves the
// description). What the host may not do with its file is dim.
export function projectMenu(id, { more = [], beforeDelete = null } = {}) {
  const project = projects.get(id);
  if (!project) return [];
  const { starred, archived, writable, deletable } = project;
  return [
    {
      label: t('project.new_here'),
      items: newTypeItems({ project: id }),
    },
    { label: starred ? t('sidebar.unstar') : t('sidebar.star'), run: () => toggleStar(id) },
    ...more,
    { label: archived ? t('header.unarchive') : t('header.archive'), dim: !writable, run: writable ? () => toggleArchive(id) : undefined },
    {
      label: t('header.delete'), armed: t('project.delete_armed'), separator: true, dim: !deletable,
      run: deletable ? () => deleteProject(id, beforeDelete) : undefined,
    },
  ];
}

// The star: the host writes `starred` (unstarring removes it) and pushes
// the project back as `entity`, which re-sorts the pills. The selection,
// the filter and an open page stay.
export function toggleStar(id) {
  const starred = !projects.get(id)?.starred;
  structureCall('set_starred', { id, starred }, () => store.getState());
}

// Archive: the host writes `archived` (unarchiving removes it) and pushes
// the project back, which moves its pill between the active and the
// archived projects. The selection stays.
export function toggleArchive(id) {
  const archived = !projects.get(id)?.archived;
  structureCall('set_archived', { id, archived }, () => store.getState());
}

// Delete: the host takes the project out of its members and moves its file
// to `.trash`, and `onEntityDeleted` clears the filter. `before` runs
// first, so if the delete fails no edit is lost.
export function deleteProject(id, before = null) {
  before?.();
  structureCall('delete_entity', { id }, () => store.getState());
}
