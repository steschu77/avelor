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

// A boolean ARIA state: present as "true" when `on`, absent otherwise.
export function setAria(el, name, on) {
  if (on) el.setAttribute(name, 'true');
  else el.removeAttribute(name);
}
