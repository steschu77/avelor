// The Dashboard lens (ui.md#dashboard): the cards of
// features/dashboard/cards.js in a grid, in the order and with the closed
// ones the preferences keep (`dashboard_cards`, `dashboard_closed`); a
// card's head drags it before or after another, its x closes it; the lens
// header's Cards menu shows and hides them and resets the layout. The lens
// registers itself (features/lenses/registry.js) and is drawn again when
// the entities, the filter, a project's name, the layout or the day
// change; the selected entity is marked on its rows.

import { $, accEl, placeChildren } from '../../core/dom.js';
import { LONG_DATE, dayStart, everyMidnight } from '../../core/format.js';
import { savePrefs, store } from '../../model/state.js';
import { iconEl } from '../../ui/icons.js';
import { showMenu } from '../../ui/menu.js';
import { clearDropMark, dropTarget } from '../../ui/dropzone.js';
import { markSelection } from '../selection.js';
import { invalidateLenses, registerLens } from '../lenses/registry.js';
import { CARDS } from './cards.js';

// A card dragged by its head carries its id as this type; only the
// dashboard's cards take it.
const CARD_DRAG = 'application/x-avelor-card';
const DEFAULT_ORDER = CARDS.map(card => card.id);

const lensEl = $('lens-dashboard');
const gridEl = $('dash-grid');
const emptyEl = $('dash-empty');
const todayEl = $('dash-today');
const cardsBtn = $('dash-cards-btn');
const cardsClosedEl = $('dash-cards-closed');

// ----------------------------------------------------------------------------
// The layout as the preferences keep it: the cards' order (an id it
// doesn't know dropped, a card it doesn't name at the end) and the closed
// ones.
function layout() {
  const { dashboard_cards = '', dashboard_closed = '' } = store.getState().prefs;
  const known = id => DEFAULT_ORDER.includes(id);
  const saved = [...new Set(dashboard_cards.split(',').filter(known))];
  return {
    order: [...saved, ...DEFAULT_ORDER.filter(id => !saved.includes(id))],
    closed: new Set(dashboard_closed.split(',').filter(known)),
  };
}

function saveLayout(order, closed) {
  savePrefs({ dashboard_cards: order.join(','), dashboard_closed: [...closed].join(',') });
}

function isDefaultLayout({ order, closed }) {
  return !closed.size && order.join() === DEFAULT_ORDER.join();
}

function showCard(id, shown) {
  const { order, closed } = layout();
  if (shown) closed.delete(id);
  else closed.add(id);
  saveLayout(order, closed);
}

function moveCard(from, to, before) {
  const { order, closed } = layout();
  const next = order.filter(id => id !== from);
  next.splice(next.indexOf(to) + (before ? 0 : 1), 0, from);
  if (next.join() !== order.join()) saveLayout(next, closed);
}

// ----------------------------------------------------------------------------
// A section per card, kept across draws: its head (the title, what the card
// puts there, the x) is what drags it.
const sections = new Map(); // card id -> { section, head, title, close }
let draggedCard = null; // the id of the card being dragged

const SECTION_CLASSES = {
  full: 'dash__section dash__section--full',
  wide: 'dash__section dash__section--wide',
};
const DROP_CLASSES = { left: 'is-drop-left', right: 'is-drop-right', top: 'is-drop-top', bottom: 'is-drop-bottom' };

for (const card of CARDS) {
  const section = accEl('section', SECTION_CLASSES[card.size] ?? 'dash__section');
  const head = accEl('div', 'section-head dash__head');
  head.draggable = true;
  head.title = 'Drag to move';
  const title = accEl('span', '', card.title);
  const close = accEl('button', 'tool dash__close');
  close.type = 'button';
  close.append(iconEl('#i-dismiss'));
  close.title = 'Close card';
  close.addEventListener('click', () => showCard(card.id, false));
  sections.set(card.id, { section, head, title, close });

  // A drag starting on a button of the head is none.
  let pressedOnButton = false;
  head.addEventListener('pointerdown', e => {
    pressedOnButton = !!e.target.closest('button');
  });
  head.addEventListener('dragstart', e => {
    if (pressedOnButton) {
      e.preventDefault();
      return;
    }
    const r = section.getBoundingClientRect();
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData(CARD_DRAG, card.id);
    e.dataTransfer.setDragImage(section, e.clientX - r.left, e.clientY - r.top);
    draggedCard = card.id;
    // marked after the drag image is taken
    setTimeout(() => section.classList.add('is-dragging'), 0);
  });
  head.addEventListener('dragend', () => {
    section.classList.remove('is-dragging');
    draggedCard = null;
    clearDropMark();
  });

  // Over another card, the side it would go to is marked: above or below a
  // card as wide as the grid, else left or right.
  dropTarget(section, {
    over: e => {
      if (!draggedCard || !e.dataTransfer.types.includes(CARD_DRAG)) return null;
      if (draggedCard === card.id) return { el: null };
      const r = section.getBoundingClientRect();
      const full = r.width > gridEl.clientWidth * 0.8;
      const side = full
        ? (e.clientY < r.top + r.height / 2 ? 'top' : 'bottom')
        : (e.clientX < r.left + r.width / 2 ? 'left' : 'right');
      return { el: section, cls: DROP_CLASSES[side], side };
    },
    drop: ({ el, side }) => {
      if (el) moveCard(draggedCard, card.id, side === 'left' || side === 'top');
    },
    effect: 'move',
  });
}

// ----------------------------------------------------------------------------
const refresh = () => invalidateLenses('dashboard');

// Draw the shown cards in their order; with every card closed, the lens
// says so.
function renderDashboard() {
  const today = dayStart(new Date());
  const { order, closed } = layout();
  const ctx = { today, shown: id => !closed.has(id), refresh };
  for (const card of CARDS) {
    const { section, head, title, close } = sections.get(card.id);
    section.hidden = closed.has(card.id);
    if (section.hidden) continue;
    const drawn = card.render(ctx);
    placeChildren(head, [title, ...drawn.head, close]);
    placeChildren(section, [head, ...drawn.body]);
  }
  placeChildren(gridEl, order.map(id => sections.get(id).section));
  emptyEl.hidden = closed.size < CARDS.length;
  todayEl.textContent = LONG_DATE.format(today);
  markSelected();
}

// The rows of the selected entity are marked.
const markSelected = markSelection(lensEl);

// ----------------------------------------------------------------------------
// The lens header: the date and the Cards menu, while the dashboard is
// shown; the menu's button counts the closed cards.
function showHeader(selected_lens) {
  const shown = selected_lens === 'dashboard';
  todayEl.hidden = !shown;
  cardsBtn.hidden = !shown;
}

function countClosed() {
  const { closed } = layout();
  cardsClosedEl.hidden = !closed.size;
  cardsClosedEl.textContent = `${closed.size} closed`;
}

cardsBtn.addEventListener('click', () => {
  const current = layout();
  const { order, closed } = current;
  const titles = Object.fromEntries(CARDS.map(card => [card.id, card.title]));
  const isDefault = isDefaultLayout(current);
  showMenu(cardsBtn, [
    ...order.map(id => ({ label: titles[id], checked: !closed.has(id), run: () => showCard(id, closed.has(id)) })),
    {
      label: 'Reset layout', separator: true, dim: isDefault,
      run: isDefault ? undefined : () => saveLayout(DEFAULT_ORDER, new Set()),
    },
  ]);
});

$('dash-show-all').addEventListener('click', () => saveLayout(layout().order, new Set()));

store.subscribe(state => state.selected_lens, showHeader);
store.subscribe(
  [({ prefs }) => prefs.dashboard_cards, ({ prefs }) => prefs.dashboard_closed, ({ prefs }) => prefs.week_start],
  () => {
    countClosed();
    refresh();
  });
// a project's name shows on the tasks and the chats
store.subscribe(state => state.projects_rev, refresh);
everyMidnight(refresh);

showHeader(store.getState().selected_lens);
countClosed();
registerLens('dashboard', lensEl, renderDashboard);
refresh();
