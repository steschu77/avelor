// The lenses (ui.md#lenses): the main view's tabs, one lens shown at a
// time (`selected_lens`, `selectLens`), the one shown last again on start
// (`restoreLens`). A lens registers its element and, if it shows entities,
// its render (`registerLens`).

import { oncePerFrame } from '../../core/dom.js';
import { savePrefs, store } from '../../model/state.js';
import { selectLens } from '../selection.js';

const lensViewEls = {}; // lens id -> its element in the main view

// The lenses showing entities render at most once per frame, and only the
// visible one: `invalidateLenses` marks the named lenses (all by default)
// stale, and a hidden one renders when its tab is selected. A scan sends an
// `entity` event per file.
const lensRenders = {}; // lens id -> its render
const staleLenses = new Set();

// `el`: the lens's part of the main view, shown while its tab is selected;
// `render()`: the render of a lens that shows entities.
export function registerLens(id, el, render) {
  lensViewEls[id] = el;
  if (render) lensRenders[id] = render;
  el.hidden = id !== store.getState().selected_lens;
}

const scheduleRenderLens = oncePerFrame(() => {
  const lens = store.getState().selected_lens;
  if (staleLenses.delete(lens)) lensRenders[lens]();
});

export function invalidateLenses(...lenses) {
  for (const lens of lenses.length ? lenses : Object.keys(lensRenders)) staleLenses.add(lens);
  scheduleRenderLens();
}

store.subscribe(state => state.selected_lens,
  selected_lens => {
    for (const tabEl of document.querySelectorAll('.tabs > [data-lens]')) {
      tabEl.setAttribute('aria-selected', String(tabEl.dataset.lens === selected_lens));
    }

    for (const [lens, el] of Object.entries(lensViewEls)) {
      el.hidden = lens !== selected_lens;
    }
    scheduleRenderLens();
  });

// Show the lens shown last (app.js, once the lenses are registered); one
// that no longer exists leaves the first, the Dashboard. From then on the
// lens shown is remembered (the `lens` preference).
export function restoreLens() {
  const { lens } = store.getState().prefs;
  if (lens && lensViewEls[lens]) selectLens(lens);
  store.subscribe(state => state.selected_lens, selected_lens => {
    if (store.getState().prefs.lens !== selected_lens) savePrefs({ lens: selected_lens });
  });
}

for (const tabEl of document.querySelectorAll('.tabs > [data-lens]')) {
  tabEl.addEventListener('click', () => selectLens(tabEl.dataset.lens));
}
