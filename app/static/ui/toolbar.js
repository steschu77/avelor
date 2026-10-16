// The editors' toolbars, cloned from index.html's templates for a card's
// editor and preview (features/panel/panel.js).

import { $ } from '../core/dom.js';

// Clones the shared toolbar markup (index.html #editor-toolbar-template),
// so each editor (a card's) gets its own live
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
