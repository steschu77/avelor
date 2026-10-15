// The DOM helpers every module uses.

export const $ = id => document.getElementById(id);

// A new `tag` element with `className` and the text `text`, if given.
export function accEl(tag, className, text) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  if (text !== undefined) el.textContent = text;
  return el;
}
