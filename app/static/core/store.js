// The page's observable store (architecture.md#page-state): `setState`
// merges an update and calls each subscriber whose selected value changed
// (`Object.is`); `subscribe` also calls it once at once. A subscriber to
// several values selects them with an array of selectors: it is called when
// any of them changed, with all of them as an array.

export function createStore(initialState) {
  let state = initialState;

  const subscribers = new Set();

  function getState() {
    return state;
  }

  function setState(update) {
    state = {
      ...state,
      ...update
    };

    notify();
  }

  function notify() {
    subscribers.forEach(subscriber => {
      const newSelectedValue =
        subscriber.selector(state);

      // only notify if selected value changed
      if (subscriber.changed(
        subscriber.currentValue,
        newSelectedValue
      )) {
        subscriber.currentValue =
          newSelectedValue;

        subscriber.listener(
          newSelectedValue,
          state
        );
      }
    });
  }

  function subscribe(selector, listener) {
    const several = Array.isArray(selector);
    const select = several ? s => selector.map(one => one(s)) : selector;
    const subscriber = {
      selector: select,
      changed: several
        ? (before, now) => now.some((value, i) => !Object.is(before[i], value))
        : (before, now) => !Object.is(before, now),
      listener,
      currentValue: select(state)
    };

    subscribers.add(subscriber);

    // immediate first run
    listener(
      subscriber.currentValue,
      state
    );
  }

  return {
    getState,
    setState,
    subscribe
  };
}
