// The vault's data as the page knows it (architecture.md#page-state): the
// Map of the entities, written only from the host's events
// (model/events.js).

export const entities = new Map(); // doc_id -> document data (every non-{project, thread} entity)
