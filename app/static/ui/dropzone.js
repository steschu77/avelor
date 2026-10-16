// Dragging on the page (panel.md#structural-edits): what takes a drop and
// marks where it would land (one mark page-wide); the drag preview chip a
// lens's row and a card's head both show; the type a card dragged out of its
// thread carries and the lens that takes it; and the panel's drop zones.

import { accEl } from '../core/dom.js';
import { iconEl } from './icons.js';

// A card dragged out of its thread by its head carries the thread it
// leaves as this type, beside its id as `text/plain`: the Documents lens
// takes it, the panel's drop zones don't.
export const MEMBER_DRAG = 'application/x-avelor-thread';

// ----------------------------------------------------------------------------
// Where a drop would land, marked: { owner, el, cls } (`dropTarget`), or
// null. One at a time, page-wide.
let mark = null;

function setMark(next) {
  if (mark?.el !== next?.el || mark?.cls !== next?.cls) {
    mark?.el.classList.remove(mark.cls);
    next?.el.classList.add(next.cls);
  }
  mark = next;
}

// Make `target` take drops. `over(ev)` says where a drag over it would
// land, or null when `target` doesn't take it: `{ el, cls }`, the element
// marked while the drag is there by the class `cls` ('is-drop-target'
// unless given; `el` null marks nothing), with anything else the drop
// needs. A drop calls `drop(at, ev)` with what `over` says there; `effect`
// is the drop effect shown. A target clears its own mark when the drag
// leaves it or doesn't take it.
export function dropTarget(target, { over, drop, effect = 'copy' }) {
  const own = () => mark?.owner === target;

  target.addEventListener('dragover', ev => {
    const at = over(ev);
    if (!at) {
      if (own()) setMark(null);
      return;
    }
    ev.preventDefault(); // required to become a valid drop target
    ev.dataTransfer.dropEffect = effect;
    setMark(at.el ? { owner: target, el: at.el, cls: at.cls ?? 'is-drop-target' } : null);
  });

  target.addEventListener('dragleave', ev => {
    if (own() && !target.contains(ev.relatedTarget)) setMark(null);
  });

  target.addEventListener('drop', ev => {
    const at = over(ev);
    if (own()) setMark(null);
    if (!at) return;
    ev.preventDefault();
    drop(at, ev);
  });
}

// ----------------------------------------------------------------------------
// Shared drag-preview chip, used for both a row dragged out of a lens
// (features/lenses/documents.js) and a card dragged out of its thread by
// its head (features/panel/panel.js) -- the same small icon+title pill,
// since both carry an entity's id. `icon` is the symbol of its kind
// (ui/icons.js). `effectAllowed`: 'copy' for a row, which joins what it is
// dropped on; 'copyMove' for a card, which also moves out of its thread.
let dragPreviewEl = null;

export function startDragPreview(ev, icon, title, effectAllowed = 'copy') {
  dragPreviewEl = accEl('span', 'chip drag-preview');

  dragPreviewEl.appendChild(iconEl(icon));
  dragPreviewEl.appendChild(document.createTextNode(' ' + title));

  document.body.appendChild(dragPreviewEl);

  ev.dataTransfer.effectAllowed = effectAllowed;
  ev.dataTransfer.setDragImage(dragPreviewEl, -12, 0);
}

export function endDragPreview() {
  dragPreviewEl?.remove();
  dragPreviewEl = null;
}

// ----------------------------------------------------------------------------
// A lens that takes a card dragged out of its thread: dropped on `lensEl`,
// it calls `onLeave(doc_id, thread_id)`; the lens is marked while one is
// over it. A drop target inside the lens that takes the card first (the
// Documents lens's project and thread rows) cancels the event, and then
// the lens leaves it alone.
export function takeMemberDrops(lensEl, onLeave) {
  dropTarget(lensEl, {
    over: ev => (ev.dataTransfer.types.includes(MEMBER_DRAG) && !ev.defaultPrevented ? { el: lensEl } : null),
    drop: (_at, ev) => onLeave(ev.dataTransfer.getData('text/plain'), ev.dataTransfer.getData(MEMBER_DRAG)),
    effect: 'move',
  });
}

// ----------------------------------------------------------------------------
// A drop zone of the panel: an entity dropped on `containerEl` calls
// `onDrop(doc_id)`; while one is dragged over it, its last card is
// marked. A card dragged out of its thread (`MEMBER_DRAG`) is no drop.
export function attachDropZone(containerEl, onDrop) {
  dropTarget(containerEl, {
    over: ev => (ev.dataTransfer.types.includes(MEMBER_DRAG)
      ? null : { el: containerEl.lastElementChild, cls: 'is-drop-after' }),
    drop: (_at, ev) => {
      const doc_id = ev.dataTransfer.getData('text/plain');
      if (doc_id) onDrop(doc_id);
    },
  });
}
