// Editing in place: a title where it shows (its `onEditEnd` also ends the
// sidebar's renames, features/sidebar.js).

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
// host's push renames it. The project header's, a card's and the thread
// header's title.
export function editTextInPlace(el, current, commit) {
  const text = current();
  el.textContent = text;
  el.setAttribute('contenteditable', 'plaintext-only');
  onEditEnd(el, save => {
    const title = el.textContent.replace(/\s+/g, ' ').trim();
    el.removeAttribute('contenteditable');
    el.textContent = current();
    if (save && title !== text) commit(title);
  });
  el.focus();
  window.getSelection().selectAllChildren(el);
}
