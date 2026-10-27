// A vault for the page's tests: its entities and projects, put into the model
// as the host's events do (model/events.js), without a filter.

import { parseDate } from '../../static/core/format.js';
import { entities, projects } from '../../static/model/entities.js';
import { store } from '../../static/model/state.js';

const UNFILTERED = { selected_project: null, selected_source: null, selected_labels: [], word_filter: null };

// The vault of `metas`, each an entity as its event brings it
// (api.md#events), in place of the one before.
export function vault(...metas) {
  entities.clear();
  projects.clear();
  store.setState(UNFILTERED);
  for (const meta of metas) {
    if (meta.kind === 'project') projects.set(meta.id, { name: meta.title, starred: false, archived: false, ...meta });
    else entities.set(meta.id, entity(meta));
  }
}

// An entity as the page keeps it.
export function entity(meta) {
  const date = parseDate(meta.date);
  return {
    ...meta,
    doc_id: meta.id,
    date,
    end: parseDate(meta.event?.end),
    due: parseDate(meta.task?.due),
    done: parseDate(meta.task?.done),
    all_day: date && meta.date.length === 10,
  };
}
