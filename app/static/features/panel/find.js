// The panel's find strip (panel.md#find): while the vault panel's words
// filter (ui.md#words), the cards shown mark them -- CSS highlights over
// their text, so the editor's DOM stays the file's -- and the strip counts
// them and steps through them, F3 and Shift+F3. Only where a search reads:
// a card's title, its details, its labels and its text.

import { $, oncePerFrame } from '../../core/dom.js';
import { registerAction } from '../../core/actions.js';
import { store } from '../../model/state.js';
import { termsRegex } from '../../ui/marks.js';

const READ = '.msg .doc-title, .msg .props, .msg .chip--tag, .msg .editor';
const VIEWS = ['panel-doc-view', 'panel-thread-view', 'panel-day-view'].map($);

const findEl = $('find');
const queryEl = $('find-q');
const countEl = $('find-n');
const stepBtns = [$('find-prev'), $('find-next')];

// An older web view (WebKitGTK before 2.42) draws no highlights; the strip
// still counts and steps.
const highlights = globalThis.CSS?.highlights;

let ranges = [];
let current = 0;
// A new selection or new words start at the first mark of the selected card.
let startOver = true;

const scheduleMark = oncePerFrame(mark);
const observer = new MutationObserver(scheduleMark);
for (const view of VIEWS) observer.observe(view, { childList: true, subtree: true, characterData: true });

const restart = () => {
  startOver = true;
  scheduleMark();
};
store.subscribe(state => state.word_filter, restart);
store.subscribe([
  state => state.selected_entity, state => state.selected_thread, state => state.selected_day?.getTime(),
], restart);

stepBtns[0].addEventListener('click', () => step(-1));
stepBtns[1].addEventListener('click', () => step(1));
const stepping = () => !findEl.hidden && store.getState().panel_size !== 'closed';
registerAction({ id: 'find.next', title: 'Next', shortcut: 'F3', when: stepping, run: () => step(1) });
registerAction({ id: 'find.previous', title: 'Previous', shortcut: 'Shift+F3', when: stepping, run: () => step(-1) });

// The parts of the shown cards a search reads, none inside another.
function readParts() {
  const parts = VIEWS.filter(view => !view.hidden).flatMap(view => [...view.querySelectorAll(READ)]);
  return parts.filter(part => !parts.some(other => other !== part && other.contains(part)));
}

function mark() {
  const { word_filter } = store.getState();
  if (!word_filter && !ranges.length && findEl.hidden) return;
  ranges = [];
  const parts = readParts();
  if (word_filter) {
    const pattern = termsRegex(word_filter.terms);
    for (const part of parts) {
      const walker = document.createTreeWalker(part, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        for (const match of node.data.matchAll(pattern)) {
          const range = new Range();
          range.setStart(node, match.index);
          range.setEnd(node, match.index + match[0].length);
          ranges.push(range);
        }
      }
    }
  }
  findEl.hidden = !word_filter || !parts.length;
  if (startOver) {
    const card = VIEWS.find(view => !view.hidden)?.querySelector('.msg[aria-selected="true"]');
    current = Math.max(0, card ? ranges.findIndex(range => card.contains(range.startContainer)) : 0);
    startOver = false;
    show(true);
  } else {
    current = Math.min(current, Math.max(0, ranges.length - 1));
    show(false);
  }
}

// Draw the marks and the strip; `scroll` brings the current mark into
// view if it is out of it.
function show(scroll) {
  const shown = !findEl.hidden && ranges.length > 0;
  highlights?.set('words', new Highlight(...(shown ? ranges : [])));
  highlights?.set('word-current', new Highlight(...(shown ? [ranges[current]] : [])));
  if (findEl.hidden) return;
  queryEl.textContent = store.getState().word_filter.text.trim();
  countEl.textContent = ranges.length ? `${current + 1} of ${ranges.length}` : 'not in this card';
  for (const btn of stepBtns) btn.setAttribute('aria-disabled', String(ranges.length < 2));
  if (scroll && shown) scrollToCurrent();
}

function scrollToCurrent() {
  const range = ranges[current];
  const list = range.startContainer.parentElement?.closest('.msg-list');
  if (!list) return;
  const at = range.getBoundingClientRect();
  const box = list.getBoundingClientRect();
  if (at.top < box.top || at.bottom > box.bottom) {
    range.startContainer.parentElement.scrollIntoView({ block: 'center', behavior: 'instant' });
  }
}

function step(by) {
  if (ranges.length < 2) return;
  current = (current + by + ranges.length) % ranges.length;
  show(true);
}
