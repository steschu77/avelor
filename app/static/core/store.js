// The page's observable store (architecture.md#page-state): `setState`
// merges an update and calls each subscriber whose selected value changed
// (`Object.is`); `subscribe` also calls it once at once.

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
      if (!Object.is(
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
    const subscriber = {
      selector,
      listener,
      currentValue: selector(state)
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
