// Pure message-tree helpers: no DOM, no globals. Callers pass in a Map of
// messages (or an already-filtered array) and a childrenIdx built by
// `childrenByParent`. Kept framework-free like markdown.js/cache.js.
//
// Threads are flat, date-ordered lists (vault.md#thread). The
// parent tree, active leaf, and sibling navigation belong to conversations
// ("Conversation"): the assistant (features/assistant/assistant.js) uses these helpers, with
// messages keyed by `id`.

// ----------------------------------------------------------------------------
// Group one conversation's messages by their `parent` id (a missing parent
// is keyed as undefined).
export function childrenByParent(threadMessages) {
  const index = new Map(); // parent_id|undefined -> Message[]
  for (const msg of threadMessages) {
    const key = msg.parent ?? undefined;
    let list = index.get(key);
    if (!list) {
      list = [];
      index.set(key, list);
    }
    list.push(msg);
  }
  return index;
}

// ----------------------------------------------------------------------------
// Walk `.parent` upward from `activeId` until a message with no parent is
// reached (inclusive), then reverse so index 0 is the root. Defensive
// against cycles/missing ids: stops and returns the partial path rather
// than looping forever or throwing.
export function pathToActive(messages, activeId) {
  const path = [];
  const seen = new Set();
  let id = activeId;

  while (id !== undefined && id !== null && !seen.has(id)) {
    const msg = messages.get(id);
    if (!msg) break;
    seen.add(id);
    path.push(msg);
    id = msg.parent;
  }

  return path.reverse();
}

// ----------------------------------------------------------------------------
// The list of `message`'s siblings (all messages sharing its parent, itself
// included) and its own index within that list.
export function siblingsOf(message, childrenIdx) {
  const list = childrenIdx.get(message.parent ?? undefined) ?? [message];
  const index = list.indexOf(message);
  return { list, index };
}

// ----------------------------------------------------------------------------
// Walk down from `message` following the most-recently-created child at
// each branch (lexicographically greatest id -- `Id` is UUIDv7 and thus
// time-ordered, so this is equivalent to "most recent" with no extra data)
// until a leaf (no children) is reached.
export function firstLeafFrom(message, childrenIdx) {
  let current = message;
  const seen = new Set();
  while (!seen.has(current.id)) {
    seen.add(current.id);
    const children = childrenIdx.get(current.id);
    if (!children || children.length === 0) return current;
    current = children.reduce((a, b) => (b.id > a.id ? b : a));
  }
  return current;
}
