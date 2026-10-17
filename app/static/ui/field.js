// Editing in place: a title where it shows (its `onEditEnd` also ends the
// sidebar's renames, features/sidebar.js), and the Workspace pages' editable
// field with the edits under way.

import { accEl } from '../core/dom.js';
import { iconEl } from './icons.js';
import { setPageFocus } from './page.js';

// End an in-place edit of `el` (an input or a contenteditable element):
// Enter or leaving it calls `finish(true)`, Esc `finish(false)`. Only the
// first of them counts; the listeners are removed before `finish` runs.
export function onEditEnd(el, finish) {
  const listening = new AbortController();
  const options = { signal: listening.signal };
  const end = save => {
    listening.abort();
    finish(save);
  };
  el.addEventListener('click', e => e.stopPropagation(), options);
  el.addEventListener('keydown', e => {
    e.stopPropagation();
    if (e.key === 'Enter') {
      e.preventDefault();
      end(true);
    } else if (e.key === 'Escape') {
      end(false);
    }
  }, options);
  el.addEventListener('blur', () => end(true), options);
}

// Edit `el`'s text in place, a title of one line: it shows `current()`,
// all selected; Enter or leaving it calls `commit(title)` with the text
// trimmed (whitespace runs, e.g. pasted line breaks, as one space) if it
// changed, Esc cancels. Either way `el` shows `current()` again, until the
// host's push renames it. A draggable ancestor (a thread card's head)
// stops dragging meanwhile, so the mouse selects text. The project
// header's, a card's and the thread header's title.
export function editTextInPlace(el, current, commit) {
  const text = current();
  const dragged = el.closest('[draggable="true"]');
  if (dragged) dragged.draggable = false;
  el.textContent = text;
  el.setAttribute('contenteditable', 'plaintext-only');
  onEditEnd(el, save => {
    const title = el.textContent.replace(/\s+/g, ' ').trim();
    el.removeAttribute('contenteditable');
    if (dragged) dragged.draggable = true;
    el.textContent = current();
    if (save && title !== text) commit(title);
  });
  el.focus();
  window.getSelection().selectAllChildren(el);
}

// ----------------------------------------------------------------------------
// `${owner}\n${field}` -> { value, checking, error } while a field is
// edited; `owner` is what the field belongs to: an account or watch id, or
// 'new:<kind>' for one not saved yet. Absent: the saved value shows.
export const fieldEdits = new Map();

export function fieldEditKey(owner, field) {
  return `${owner}\n${field}`;
}

// A field shown as text with a pencil, or as an input while it is edited
// -- and, if `required`, while nothing valid is saved. `opts`: { saved
// (text, or null if nothing valid is saved), empty (the text without a
// saved value), required, value (the input's start), placeholder, type,
// mono, editTitle (the pencil's title), onSave(value), rerender (the
// page's scheduler) }.
export function renderEditField(owner, field, opts) {
  const key = fieldEditKey(owner, field);
  const state = fieldEdits.get(key);
  const wrap = accEl('span', 'edit-field');
  if (!state && (opts.saved != null || !opts.required)) {
    const textClass = opts.mono ? 'edit-field__text mono' : 'subtitle';
    wrap.append(accEl('span', textClass, opts.saved ?? opts.empty ?? ''));
    const edit = accEl('button', 'icon-btn');
    edit.append(iconEl('#i-edit'));
    edit.type = 'button';
    edit.title = opts.editTitle ?? 'Edit';
    edit.addEventListener('click', () => {
      fieldEdits.set(key, { value: opts.value ?? '' });
      setPageFocus(key);
      opts.rerender();
    });
    wrap.append(edit);
    return [wrap, null];
  }

  const input = accEl('input', opts.mono ? 'input input--mono edit-field__input' : 'input edit-field__input');
  input.type = opts.type ?? 'text';
  input.value = state?.value ?? opts.value ?? '';
  input.placeholder = opts.placeholder ?? '';
  input.spellcheck = false;
  input.autocomplete = 'off';
  input.dataset.focusKey = key;
  input.disabled = !!state?.checking;

  const save = accEl('button', 'btn btn--primary', 'Save');
  save.type = 'button';
  const cancelable = opts.saved != null || !opts.required;
  const canSave = () => !state?.checking && (input.value.trim() !== '' || !opts.required);
  save.disabled = !canSave();
  const submit = () => {
    if (canSave()) opts.onSave(input.value.trim());
  };
  const cancel = () => {
    fieldEdits.delete(key);
    opts.rerender();
  };
  input.addEventListener('input', () => {
    const current = fieldEdits.get(key);
    fieldEdits.set(key, { ...current, value: input.value, error: current?.error ?? null });
    save.disabled = !canSave();
  });
  input.addEventListener('keydown', e => {
    if (e.key === 'Enter') {
      e.preventDefault();
      submit();
    } else if (e.key === 'Escape' && cancelable) {
      e.preventDefault();
      cancel();
    }
  });
  save.addEventListener('click', submit);
  wrap.classList.add('edit-field--edit');
  wrap.append(input, save);
  if (cancelable && !state?.checking) {
    const close = accEl('button', 'btn btn--ghost', '\u00d7');
    close.type = 'button';
    close.title = 'Cancel';
    close.addEventListener('click', cancel);
    wrap.append(close);
  }

  let note = null;
  if (state?.checking) note = accEl('span', 'meta', 'Checking\u2026');
  else if (state?.error) note = accEl('span', 'error-note', state.error);
  return [wrap, note];
}
