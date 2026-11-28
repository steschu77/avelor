// Editing in place: the pen, a title where it shows (its `onEditEnd` also
// ends the sidebar's renames, features/sidebar.js), and the Workspace pages'
// editable field with the edits under way.

import { accEl } from '../core/dom.js';
import { titled } from '../core/dom.js';
import { t } from '../core/text.js';
import { iconEl } from './icons.js';
import { setPageFocus } from './page.js';

// The pen (ui.md#the-pen): it edits the text next to it, a name or a
// content, and nothing else, and looks the same everywhere; `title` says
// what it changes.
export function penButton(title, onClick = null) {
  const pen = accEl('button', 'icon-btn icon-btn--quiet pen');
  pen.type = 'button';
  titled(pen, title);
  pen.append(iconEl('#i-edit'));
  if (onClick) pen.addEventListener('click', onClick);
  return pen;
}

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
// An input of a card's details block (features/panel/details.js), of
// `type`, holding `value`; `key` finds it again once the block is drawn
// anew.
export function fieldInput(type, key, value, className) {
  const input = accEl('input', className);
  input.type = type;
  input.value = value;
  input.dataset.focusKey = key;
  return input;
}

// A field of a card's details block: Enter or leaving it writes what it
// holds, if that changed (`write(value)`); Esc puts back what it held
// (`onEditEnd`). A date input's change fires for every digit typed, so it
// isn't what writes.
export function editField(input, write) {
  const arm = () => {
    const before = input.value;
    onEditEnd(input, save => {
      if (!save) input.value = before;
      else if (input.value !== before) write(input.value);
      if (document.activeElement === input) arm();
    });
  };
  input.addEventListener('focus', arm);
}

// ----------------------------------------------------------------------------
// A block of fields drawn anew on every change (a card's details and its
// parts, a project's blueprint): while one of its inputs has the focus,
// that waits until the block is left, so what is typed stays; while a
// press on it is under way, until its click has run, so the pressed button
// isn't replaced before it (the host's answer to a field written as the
// button took the focus). A radio or a checkbox is no typing.
const heldRenders = new WeakMap(); // block -> its latest render
const watchedBlocks = new WeakSet(); // the blocks listening for that
const pressedBlocks = new WeakSet();

function busy(block) {
  if (pressedBlocks.has(block)) return true;
  const el = document.activeElement;
  return block.contains(el) && el.tagName === 'INPUT' && el.type !== 'radio' && el.type !== 'checkbox';
}

// Hold `render` of `block` while it is typed in or pressed; whether it did.
export function heldWhileTyping(block, render) {
  watch(block);
  if (!busy(block)) return false;
  heldRenders.set(block, render);
  return true;
}

function watch(block) {
  if (watchedBlocks.has(block)) return;
  watchedBlocks.add(block);
  // where the focus went, and whether a click followed the mouseup, is
  // known only after the event
  const release = () => setTimeout(() => {
    const held = heldRenders.get(block);
    if (!held || busy(block)) return;
    heldRenders.delete(block);
    held();
  });
  block.addEventListener('mousedown', () => {
    pressedBlocks.add(block);
    document.addEventListener('mouseup', () => {
      pressedBlocks.delete(block);
      release();
    }, { capture: true, once: true });
  });
  block.addEventListener('focusout', release);
}

// Draw `block` anew with `draw()`: the field or button of it that had the
// focus gets it back, found by its focus key (`fieldInput`).
export function keepingFocus(block, draw) {
  const el = document.activeElement;
  const key = block.contains(el) ? el.dataset.focusKey : undefined;
  draw();
  const again = key && [...block.querySelectorAll('[data-focus-key]')].find(other => other.dataset.focusKey === key);
  again?.focus({ preventScroll: true });
}

// ----------------------------------------------------------------------------
// `${owner}\n${field}` -> { value, checking, error } while a field is
// edited; `owner` is what the field belongs to: an account or watch id, or
// 'new:<kind>' for one not saved yet. Absent: the saved value shows.
export const fieldEdits = new Map();

export function fieldEditKey(owner, field) {
  return `${owner}\n${field}`;
}

// A field shown as text with its pen, or as an input while it is edited
// -- and, if `required`, while nothing valid is saved. Enter or leaving the
// input saves it, as a card's field does, if its text changed (else it
// just closes), Esc cancels; a field the host checks first (`checked`: a
// key, an address, a folder) has Save and an x instead, and leaving it
// keeps it open. `opts`: { saved (text, or null if nothing valid is saved),
// empty (the text without a saved value), required, checked, value (the
// input's start), placeholder, type, mono, textClass (of the saved text),
// editTitle (the pen's title), byClick (a name: a click on it edits it
// too, ui.md#the-pen), onSave(value), rerender (the page's scheduler) }.
export function renderEditField(owner, field, opts) {
  const key = fieldEditKey(owner, field);
  const state = fieldEdits.get(key);
  const wrap = accEl('span', 'edit-field');
  if (!state && (opts.saved != null || !opts.required)) {
    const textClass = opts.textClass ?? (opts.mono ? 'edit-field__text mono' : 'subtitle');
    const start = () => {
      fieldEdits.set(key, { value: opts.value ?? '' });
      setPageFocus(key);
      opts.rerender();
    };
    const text = accEl('span', textClass, opts.saved ?? opts.empty ?? '');
    if (opts.byClick) {
      text.classList.add('renamable');
      text.addEventListener('click', start);
    }
    wrap.append(text, penButton(opts.editTitle ?? t('field.edit'), start));
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

  const save = accEl('button', 'btn btn--primary', t('field.save'));
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
  const done = () => {
    if (input.value.trim() === String(opts.value ?? '').trim() || !canSave()) cancel();
    else submit();
  };
  input.addEventListener('input', () => {
    const current = fieldEdits.get(key);
    fieldEdits.set(key, { ...current, value: input.value, error: current?.error ?? null });
    save.disabled = !canSave();
  });
  input.addEventListener('keydown', e => {
    if (e.key === 'Enter') {
      e.preventDefault();
      if (opts.checked) submit();
      else done();
    } else if (e.key === 'Escape' && cancelable) {
      e.preventDefault();
      cancel();
    }
  });
  wrap.classList.add('edit-field--edit');
  wrap.append(input);
  // a redraw that takes the input off the page is no leaving it
  if (!opts.checked) input.addEventListener('blur', () => {
    if (input.isConnected && fieldEdits.has(key)) done();
  });
  else {
    save.addEventListener('click', submit);
    wrap.append(save);
  }
  if (opts.checked && cancelable && !state?.checking) {
    const close = accEl('button', 'btn btn--ghost', '\u00d7');
    close.type = 'button';
    titled(close, t('field.cancel'));
    close.addEventListener('click', cancel);
    wrap.append(close);
  }

  let note = null;
  if (state?.checking) note = accEl('span', 'meta', t('field.checking'));
  else if (state?.error) note = accEl('span', 'error-note', state.error);
  return [wrap, note];
}

// ----------------------------------------------------------------------------
// File name patterns (`*.pdf`) of `owner`: a chip per pattern (click to
// remove it), then an input that adds what is typed on Enter (several
// separated by commas). `onChange(patterns)` gets the list as changed;
// `rerender` (the page's scheduler) runs for an Enter that adds nothing. A
// watch's file types, and the attachments an email filter saves.
export function renderPatternField(owner, patterns, onChange, rerender) {
  const chips = patterns.map(pattern => {
    const chip = accEl('button', 'chip chip--action chip--mono', `${pattern} \u00d7`);
    chip.type = 'button';
    chip.title = t('field.remove_pattern', { pattern });
    chip.addEventListener('click', () => onChange(patterns.filter(p => p !== pattern)));
    return chip;
  });

  const key = fieldEditKey(owner, 'pattern');
  const input = accEl('input', 'form-input mono mono--small');
  input.type = 'text';
  input.placeholder = patterns.length ? t('field.add') : t('field.add_pattern');
  input.spellcheck = false;
  input.autocomplete = 'off';
  input.dataset.focusKey = key;
  input.value = fieldEdits.get(key)?.value ?? '';
  input.addEventListener('input', () => fieldEdits.set(key, { value: input.value }));
  input.addEventListener('keydown', e => {
    if (e.key !== 'Enter') return;
    e.preventDefault();
    const added = input.value.split(/[,;]/).map(p => p.trim()).filter(p => p && !patterns.includes(p));
    fieldEdits.delete(key);
    if (added.length) onChange([...patterns, ...new Set(added)]);
    else rerender();
  });
  return [...chips, input];
}
