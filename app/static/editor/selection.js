// The selection and the caret (editor.md#selection, editor.md#caret): the
// range inside the editor, a selection kept across a structural edit, the
// caret placed and tested.

export function createSelection(editor) {
  function getSelectionRange() {
    const sel = window.getSelection();
    if (!sel.rangeCount) return null;
    const range = sel.getRangeAt(0);
    return editor.contains(range.commonAncestorContainer) ? range : null;
  }

  // ---------------------------------------------------------------------------
  // Selection helpers -- see editor.md#selection
  // for why every structural edit below captures the selection before
  // mutating and restores it after.
  function captureSelection() {
    const sel = window.getSelection();
    const saved = [];
    for (let i = 0; i < sel.rangeCount; i++) {
      const r = sel.getRangeAt(i);
      saved.push({
        startContainer: r.startContainer, startOffset: r.startOffset,
        endContainer: r.endContainer, endOffset: r.endOffset,
      });
    }
    return saved;
  }

  function restoreSelection(saved, remap = n => n) {
    if (!saved.length) return false;
    const sel = window.getSelection();
    sel.removeAllRanges();
    saved.forEach(({ startContainer, startOffset, endContainer, endOffset }) => {
      try {
        const r = document.createRange();
        const sc = remap(startContainer);
        const ec = remap(endContainer);
        r.setStart(sc, Math.min(startOffset, maxOffset(sc)));
        r.setEnd(ec, Math.min(endOffset, maxOffset(ec)));
        sel.addRange(r);
      } catch (err) { /* best effort */ }
    });
    return sel.rangeCount > 0;
  }

  function maxOffset(node) {
    return node.nodeType === Node.TEXT_NODE ? node.data.length : node.childNodes.length;
  }

  function selectionInside(el) {
    const sel = window.getSelection();
    return sel.rangeCount > 0 && el.contains(sel.getRangeAt(0).startContainer);
  }

  function selectRange(range) {
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  }

  function placeCaretAtStart(el) {
    const r = document.createRange();
    r.selectNodeContents(el);
    r.collapse(true);
    selectRange(r);
  }

  function placeCaretAfterElement(el) {
    const r = document.createRange();
    r.setStartAfter(el);
    r.collapse(true);
    selectRange(r);
  }

  function isCaretAtStartOf(el) {
    const sel = window.getSelection();
    if (!sel.rangeCount || !sel.isCollapsed) return false;
    const range = sel.getRangeAt(0);
    const preRange = document.createRange();
    preRange.selectNodeContents(el);
    preRange.setEnd(range.startContainer, range.startOffset);
    return preRange.toString() === '';
  }

  function isCaretAtEndOf(el) {
    const sel = window.getSelection();
    if (!sel.rangeCount || !sel.isCollapsed) return false;
    const range = sel.getRangeAt(0);
    const afterRange = document.createRange();
    afterRange.selectNodeContents(el);
    afterRange.setStart(range.startContainer, range.startOffset);
    return afterRange.toString() === '';
  }

  return {
    captureSelection, getSelectionRange, isCaretAtEndOf, isCaretAtStartOf, maxOffset, placeCaretAfterElement,
    placeCaretAtStart, restoreSelection, selectRange, selectionInside,
  };
}
