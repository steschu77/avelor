// The DOM helpers every module uses.

export const $ = id => document.getElementById(id);

// A scheduler that runs `fn()` once in the next animation frame however often
// it is called before then. `fn` gets no arguments (not rAF's timestamp).
export function oncePerFrame(fn) {
  let frame = null;
  const schedule = () => {
    if (frame !== null) return;
    frame = requestAnimationFrame(() => {
      frame = null;
      fn();
    });
  };
  return schedule;
}

// Make `container`'s children exactly `nodes`, in order, moving only the
// ones out of place: a node that stays is never detached, so it keeps its
// hover, focus and drop-target state.
export function placeChildren(container, nodes) {
  const keep = new Set(nodes);
  for (let child = container.firstElementChild; child;) {
    const next = child.nextElementSibling;
    if (!keep.has(child)) child.remove();
    child = next;
  }
  let cursor = container.firstElementChild;
  for (const node of nodes) {
    if (node === cursor) cursor = cursor.nextElementSibling;
    else container.insertBefore(node, cursor);
  }
}

// A new `tag` element with `className` and the text `text`, if given.
export function accEl(tag, className, text) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}

// An element clicked like a button that can't be one, since it holds
// controls of its own (a row with its buttons): Tab reaches it, and Enter
// or Space clicks it (below). In the markup: `tabindex="0" role="button"`.
export function pressable(el) {
  el.tabIndex = 0;
  el.setAttribute('role', 'button');
  return el;
}

// A key on a control inside a pressable element is the control's; Ctrl+Enter
// is the action's (ui/page.js). An SVG has no `.click()`.
document.addEventListener('keydown', e => {
  if (e.defaultPrevented || (e.key !== 'Enter' && e.key !== ' ') || e.ctrlKey || e.altKey) return;
  const el = e.target;
  if (el.getAttribute?.('role') !== 'button' || el.localName === 'button') return;
  e.preventDefault();
  el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }));
});

// The tooltip `text` of a control that shows a glyph (an x, a +, an arrow,
// a check mark) or an icon alone, and its name too: a screen reader reads
// the words, not the glyph. Attributes, since an SVG has no `title`.
export function titled(el, text) {
  el.setAttribute('title', text);
  el.setAttribute('aria-label', text);
  return el;
}

// A boolean ARIA state: present as "true" when `on`, absent otherwise.
export function setAria(el, name, on) {
  if (on) el.setAttribute(name, 'true');
  else el.removeAttribute(name);
}
