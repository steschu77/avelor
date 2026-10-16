export function createCache() {
  const entries = new Map();
  const subscribers = new Map();
  const pending = new Set();

  return {
    subscribe(id, callback) {
      let subs = subscribers.get(id);

      if (!subs) {
        subs = new Set();
        subscribers.set(id, subs);
      }

      subs.add(callback);

      const doc = entries.get(id);
      if (doc !== undefined) {
        callback(doc);
      }

      // Return an unsubscribe function
      return () => {
        subs.delete(callback);
        if (subs.size === 0) {
          subscribers.delete(id);
        }
      };
    },

    request(id, callback) {
      if (pending.has(id)) {
        return false;
      }

      pending.add(id);
      callback();
      return true;
    },

    // The request for `id` failed: a later one is sent again.
    failed(id) {
      pending.delete(id);
    },

    // Forget every entry and request (another vault opened); the
    // subscriptions stay with their editors, which go with the panel.
    clear() {
      entries.clear();
      pending.clear();
    },

    update(id, content) {
      entries.set(id, content);
      pending.delete(id);

      for (const callback of subscribers.get(id) ?? []) {
        callback(content);
      }
    },
  };
}