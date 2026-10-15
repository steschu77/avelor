// The DOM helpers every module uses.

export const $ = id => document.getElementById(id);

// A new `tag` element with `className` and the text `text`, if given.
export function accEl(tag, className, text) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}

// The tooltip `text` of a control that shows a glyph (an x, a +, an arrow,
// a check mark) or an icon alone, and its name too: a screen reader reads
// the words, not the glyph. Attributes, since an SVG has no `title`.
export function titled(el, text) {
  el.setAttribute('title', text);
  el.setAttribute('aria-label', text);
  return el;
}
