// The editors' toolbars, cloned from index.html's templates for a card's
// editor and preview (features/panel/panel.js) and the project
// description (features/header.js); a tool button with a label
// (`toolButton`).

import { $, accEl } from '../core/dom.js';

// Clones the shared toolbar markup (index.html #editor-toolbar-template),
// so each editor (a card's, the project description's) gets its own live
// set of buttons rather than sharing one DOM node across instances. Returns
// a detached element -- the caller appends it wherever the editor needs it.
export function createEditorToolbar() {
  const template = $('editor-toolbar-template');
  const toolbar = template.content.firstElementChild.cloneNode(true);
  toolbar.hidden = false;
  return toolbar;
}

export function createDocPreviewToolbar() {
  const template = $('preview-toolbar-template');
  const toolbar = template.content.firstElementChild.cloneNode(true);
  toolbar.hidden = false;
  return toolbar;
}

// A tool button labelled `label`, `title` its tooltip: `run(btn)` on a
// click.
export function toolButton(label, title, run) {
  const btn = accEl('button', 'tool label', label);
  btn.type = 'button';
  btn.title = title;
  btn.addEventListener('click', () => run(btn));
  return btn;
}
