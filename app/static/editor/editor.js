// Self-contained WYSIWYG editor for markdown-shaped HTML. See
// editor.md for the full rationale, block model, and behavior
// reference.
//
//   const ed = createEditor(rootEl, { toolbar, onChange, onEscape, onSubmit, onLinkClick });
//   ed.setHTML(html); ed.setEditable(true); ed.getHTML(); ed.destroy();
//
// `onSubmit`, if given, takes Enter (Shift+Enter keeps its usual meaning),
// e.g. to send a chat message. `setEditable(true, false)` doesn't focus.
//
// Toolbar buttons are looked up by `data-cmd` (bold, italic, strike, code,
// ul, ol, quote, codeblock, hr, link, image, table, row-add, row-del,
// col-add, col-del, table-del) and `data-block` (H1..H6); `data-table-only`
// elements show only while the caret is inside a table.

import { oncePerFrame } from '../core/dom.js';

let defaultParagraphSeparatorSet = false;

export function createEditor(editor, { toolbar = null, onChange = null, onEscape = null, onSubmit = null, onLinkClick = null } = {}) {
  const cmdButtons = toolbar ? toolbar.querySelectorAll('button[data-cmd]') : [];
  const blockButtons = toolbar ? toolbar.querySelectorAll('button[data-block]') : [];

  // Chrome defaults new blocks (e.g. exiting a list) to <div>; use paragraphs
  // so the editor keeps a single block structure. Document-wide setting.
  if (!defaultParagraphSeparatorSet) {
    document.execCommand('defaultParagraphSeparator', false, 'p');
    defaultParagraphSeparatorSet = true;
  }

  // ---------------------------------------------------------------------------
  // Frame-coalesced toolbar refresh: one update per animation frame no matter
  // how many browser events one edit produces.
  const toolbarUpdate = oncePerFrame(updateToolbarState);

  function scheduleToolbarUpdate() {
    if (toolbar) toolbarUpdate();
  }

  function notifyChange() {
    if (onChange) onChange();
  }

  // Common tail of every edit.
  function afterEdit() {
    ensureBlockStructure();
    ensureTrailingParagraph();
    normalizeInlineTags();
    scheduleToolbarUpdate();
    notifyChange();
  }

  // ---------------------------------------------------------------------------
  // Block model -- see editor.md#block-model for what each set means and why.
  const HEADING_TAGS = ['H1', 'H2', 'H3', 'H4', 'H5', 'H6'];
  const LEAF_BLOCK_TAGS = ['P', ...HEADING_TAGS, 'LI', 'PRE', 'TH', 'TD'];
  const ROOT_BLOCK_TAGS = ['P', ...HEADING_TAGS, 'UL', 'OL', 'BLOCKQUOTE', 'PRE', 'HR', 'TABLE'];
  const CONVERTIBLE_TAGS = ['P', ...HEADING_TAGS];

  function getBlockNode(node) {
    if (!node) return null;
    if (node.nodeType === Node.TEXT_NODE) node = node.parentNode;
    while (node && node !== editor) {
      if (LEAF_BLOCK_TAGS.includes(node.tagName)) return node;
      if (node.parentNode === editor) return node;
      node = node.parentNode;
    }
    return null;
  }

  // The ancestor of `node` that sits directly under the editor root.
  function getRootBlock(node) {
    if (!node) return null;
    if (node.nodeType === Node.TEXT_NODE) node = node.parentNode;
    while (node && node.parentNode !== editor) node = node.parentNode;
    return node;
  }

  function getSelectionRange() {
    const sel = window.getSelection();
    if (!sel.rangeCount) return null;
    const range = sel.getRangeAt(0);
    return editor.contains(range.commonAncestorContainer) ? range : null;
  }

  function getSelectionBlock() {
    const range = getSelectionRange();
    return range ? getBlockNode(range.startContainer) : null;
  }

  function getCurrentBlockTag() {
    const block = getSelectionBlock();
    return block ? block.tagName : null;
  }

  // Chromium leaves text directly under the editable root after a document
  // is cleared; wrap root-level inline content in a paragraph.
  function ensureBlockStructure() {
    let paragraph = null;
    Array.from(editor.childNodes).forEach(node => {
      if (node.nodeType === Node.TEXT_NODE && /^\s*$/.test(node.data)) {
        node.remove();
        return;
      }
      if (node.nodeType === Node.ELEMENT_NODE && ROOT_BLOCK_TAGS.includes(node.tagName)) {
        paragraph = null;
        return;
      }
      if (!paragraph) {
        paragraph = document.createElement('p');
        editor.insertBefore(paragraph, node);
      }
      paragraph.appendChild(node);
    });
  }

  // See editor.md#trailing (the trailing-paragraph invariant).
  const SELF_EXITING_TAGS = ['P', ...HEADING_TAGS, 'UL', 'OL'];

  function isEmptyParagraph(node) {
    return Boolean(node) && node.tagName === 'P' && node.textContent === '';
  }

  function ensureTrailingParagraph() {
    if (!isEditable()) return;
    const last = editor.lastElementChild;
    if (!last || SELF_EXITING_TAGS.includes(last.tagName)) return;
    const p = document.createElement('p');
    p.appendChild(document.createElement('br'));
    editor.appendChild(p);
  }

  function removeTrailingParagraph() {
    const last = editor.lastElementChild;
    const prev = last?.previousElementSibling;
    if (isEmptyParagraph(last) && prev && !SELF_EXITING_TAGS.includes(prev.tagName)) last.remove();
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

  function placeCaretAtStart(el) {
    const r = document.createRange();
    r.selectNodeContents(el);
    r.collapse(true);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(r);
  }

  function placeCaretAfterElement(el) {
    const r = document.createRange();
    r.setStartAfter(el);
    r.collapse(true);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(r);
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

  // Heals typing that leaks into a just-closed inline element -- see
  // editor.md#caret.
  let justClosedInline = null; // { el, originalText }

  function markJustClosed(el) {
    justClosedInline = { el, originalText: el.textContent };
  }

  function healLeakedTyping() {
    if (!justClosedInline) return;
    const { el, originalText } = justClosedInline;
    justClosedInline = null;
    if (!editor.contains(el)) return;
    const currentText = el.textContent;
    if (currentText === originalText || !currentText.startsWith(originalText)) return;

    const leaked = currentText.slice(originalText.length);
    let lastText = el;
    while (lastText.lastChild) lastText = lastText.lastChild;
    if (lastText.nodeType !== Node.TEXT_NODE || !lastText.data.endsWith(leaked)) return;

    lastText.data = lastText.data.slice(0, lastText.data.length - leaked.length);
    const leakedNode = document.createTextNode(leaked);
    el.parentNode.insertBefore(leakedNode, el.nextSibling);

    const r = document.createRange();
    r.setStart(leakedNode, leaked.length);
    r.collapse(true);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(r);
  }

  // Chrome won't place a typing caret inside a container that holds only a
  // zero-length text node -- it needs either real text or a <br>.
  function ensureCaretable(el) {
    Array.from(el.childNodes).forEach(n => {
      if (n.nodeType === Node.TEXT_NODE && n.data === '') el.removeChild(n);
    });
    if (!el.firstChild) el.appendChild(document.createElement('br'));
  }

  // ---------------------------------------------------------------------------
  // Block conversions (manual DOM surgery, never execCommand). `transplant`
  // moves `block`'s children into `leaf`, puts `root` where `block` was, and
  // preserves the selection throughout (editor.md#selection).
  function transplant(block, leaf, root = leaf) {
    const saved = captureSelection();
    while (block.firstChild) leaf.appendChild(block.firstChild);
    block.parentNode.replaceChild(root, block);
    ensureCaretable(leaf);
    restoreSelection(saved, n => (n === block ? leaf : n));
    if (!selectionInside(leaf)) placeCaretAtStart(leaf);
    return leaf;
  }

  function replaceBlockTag(block, tagName) {
    return transplant(block, document.createElement(tagName));
  }

  function wrapBlockAsListItem(block, listTag = 'ul', start = 1) {
    const list = document.createElement(listTag);
    if (listTag === 'ol' && start !== 1) list.setAttribute('start', String(start));
    const li = document.createElement('li');
    list.appendChild(li);
    return transplant(block, li, list);
  }

  // See editor.md#blockquotes. The insertion point is captured once before
  // the loop, not re-read each iteration, so children land in their
  // original order instead of reversed.
  function absorbBlockquote(quote, sibling, atStart) {
    const saved = captureSelection();
    const anchor = atStart ? quote.firstChild : null;
    while (sibling.firstChild) quote.insertBefore(sibling.firstChild, anchor);
    sibling.remove();
    restoreSelection(saved, n => (n === sibling ? quote : n));
  }

  // Absorbs a blockquote sibling on either side of `quote` -- both, if
  // present -- so it never ends up directly adjacent to another one.
  function absorbAdjacentQuotes(quote) {
    const prev = quote.previousElementSibling;
    if (prev && prev.tagName === 'BLOCKQUOTE') absorbBlockquote(quote, prev, true);
    const next = quote.nextElementSibling;
    if (next && next.tagName === 'BLOCKQUOTE') absorbBlockquote(quote, next, false);
  }

  // See editor.md#blockquotes.
  function wrapBlockAsQuote(block) {
    const quote = document.createElement('blockquote');
    const leaf = document.createElement(block.tagName.toLowerCase());
    quote.appendChild(leaf);
    transplant(block, leaf, quote);
    absorbAdjacentQuotes(quote);
    if (!selectionInside(leaf)) placeCaretAtStart(leaf);
    return leaf;
  }

  function wrapBlockAsCode(block) {
    return transplant(block, document.createElement('pre'));
  }

  function getEnclosingListItem(node) {
    if (!node) return null;
    if (node.nodeType === Node.TEXT_NODE) node = node.parentNode;
    const li = node.closest && node.closest('li');
    return li && editor.contains(li) ? li : null;
  }

  // Backspace at the very start of a list item un-indents it to a plain
  // paragraph first, rather than immediately merging into the previous line
  // (the list splits around it if it wasn't the first/last item).
  function convertListItemToParagraph(li) {
    const list = li.parentNode;
    const followingItems = [];
    let sib = li.nextElementSibling;
    while (sib) { followingItems.push(sib); sib = sib.nextElementSibling; }

    const saved = captureSelection();
    const p = document.createElement('p');
    while (li.firstChild) p.appendChild(li.firstChild);
    ensureCaretable(p);
    list.removeChild(li);

    let newList = null;
    if (followingItems.length > 0) {
      newList = document.createElement(list.tagName);
      followingItems.forEach(item => newList.appendChild(item));
    }

    const parent = list.parentNode;
    const insertionPoint = list.nextSibling;
    parent.insertBefore(p, insertionPoint);
    if (newList) parent.insertBefore(newList, insertionPoint);
    if (!list.firstChild) parent.removeChild(list);

    restoreSelection(saved, n => (n === li ? p : n));
    if (!selectionInside(p)) placeCaretAtStart(p);
  }

  // See editor.md#merging: Backspace/Delete merge P/heading pairs
  // explicitly, keeping the surviving block's own tag.
  const MERGEABLE_TAGS = ['P', ...HEADING_TAGS];

  function mergeBlockIntoPrevious(block) {
    const prev = block.previousElementSibling;
    if (!prev || !MERGEABLE_TAGS.includes(prev.tagName)) return false;

    const saved = captureSelection();
    // A lone <br> is only ever the empty-block caret placeholder (see
    // ensureCaretable) -- drop it from both sides, or it becomes a stray
    // line break once real content is joined around it.
    if (prev.childNodes.length === 1 && prev.firstChild.tagName === 'BR') {
      prev.removeChild(prev.firstChild);
    }
    if (block.childNodes.length === 1 && block.firstChild.tagName === 'BR') {
      block.removeChild(block.firstChild);
    }
    const caretNode = prev.lastChild;
    const caretOffset = caretNode ? maxOffset(caretNode) : 0;
    while (block.firstChild) prev.appendChild(block.firstChild);
    block.remove();
    ensureCaretable(prev);
    restoreSelection(saved, n => (n === block ? prev : n));
    if (!selectionInside(prev) && caretNode) {
      const r = document.createRange();
      r.setStart(caretNode, Math.min(caretOffset, maxOffset(caretNode)));
      r.collapse(true);
      const sel = window.getSelection();
      sel.removeAllRanges();
      sel.addRange(r);
    }
    return true;
  }

  // Tab in a list item: nest it under the previous item.
  function indentListItem(li) {
    const prev = li.previousElementSibling;
    if (!prev) return false;
    const saved = captureSelection();
    let nested = prev.lastElementChild;
    if (!nested || (nested.tagName !== 'UL' && nested.tagName !== 'OL')) {
      nested = document.createElement(li.parentNode.tagName);
      prev.appendChild(nested);
    }
    nested.appendChild(li);
    restoreSelection(saved);
    if (!selectionInside(li)) placeCaretAtStart(li);
    return true;
  }

  // Shift+Tab in a nested list item: move it up after its parent item, taking
  // the items that followed it along as its own nested list. At top level
  // the item becomes a paragraph.
  function outdentListItem(li) {
    const list = li.parentNode;
    const parentLi = list.parentNode;
    if (!parentLi || parentLi.tagName !== 'LI') {
      convertListItemToParagraph(li);
      return true;
    }
    const saved = captureSelection();
    const following = [];
    let sib = li.nextElementSibling;
    while (sib) { following.push(sib); sib = sib.nextElementSibling; }
    parentLi.parentNode.insertBefore(li, parentLi.nextSibling);
    if (following.length) {
      const nested = document.createElement(list.tagName);
      following.forEach(item => nested.appendChild(item));
      li.appendChild(nested);
    }
    if (!list.firstChild) list.remove();
    restoreSelection(saved);
    if (!selectionInside(li)) placeCaretAtStart(li);
    return true;
  }

  function indentQuotedBlock(block) {
    const saved = captureSelection();
    const quote = document.createElement('blockquote');
    block.parentNode.replaceChild(quote, block);
    quote.appendChild(block);
    absorbAdjacentQuotes(quote);
    restoreSelection(saved);
    if (!selectionInside(block)) placeCaretAtStart(block);
  }

  // Pull `block` out of its blockquote, splitting the quote around it.
  function unwrapFromQuote(block) {
    const quote = block.parentNode;
    const following = [];
    let sib = block.nextSibling;
    while (sib) { following.push(sib); sib = sib.nextSibling; }
    const saved = captureSelection();
    quote.parentNode.insertBefore(block, quote.nextSibling);
    if (following.length) {
      const rest = document.createElement('blockquote');
      following.forEach(n => rest.appendChild(n));
      block.parentNode.insertBefore(rest, block.nextSibling);
    }
    if (!quote.firstChild) quote.remove();
    restoreSelection(saved);
    if (!selectionInside(block)) placeCaretAtStart(block);
  }

  // Replace `block` with a rule followed by a fresh paragraph holding the caret.
  function replaceBlockWithRule(block) {
    const hr = document.createElement('hr');
    const p = document.createElement('p');
    p.appendChild(document.createElement('br'));
    block.parentNode.insertBefore(hr, block);
    block.parentNode.replaceChild(p, block);
    placeCaretAtStart(p);
    return p;
  }

  // Insert a rule after the current root block, caret in a new paragraph.
  function insertRule() {
    const range = getSelectionRange();
    const root = range ? getRootBlock(range.startContainer) : null;
    const hr = document.createElement('hr');
    const p = document.createElement('p');
    p.appendChild(document.createElement('br'));
    if (root) {
      editor.insertBefore(hr, root.nextSibling);
    } else {
      editor.appendChild(hr);
    }
    editor.insertBefore(p, hr.nextSibling);
    placeCaretAtStart(p);
  }

  // ---------------------------------------------------------------------------
  // Inline conversions.
  function hasAncestorTag(node, tagName) {
    return Boolean(getAncestorTag(node, tagName));
  }

  function getAncestorTag(node, tagName) {
    let el = node.nodeType === Node.TEXT_NODE ? node.parentNode : node;
    while (el && el !== editor) {
      if (el.tagName === tagName) return el;
      el = el.parentNode;
    }
    return null;
  }

  // Replace an inline element by its children, keeping the selection.
  function unwrapElement(el) {
    const saved = captureSelection();
    const parent = el.parentNode;
    const first = el.firstChild;
    while (el.firstChild) parent.insertBefore(el.firstChild, el);
    parent.removeChild(el);
    restoreSelection(saved, n => (n === el ? parent : n));
    if (first && !selectionInside(parent)) placeCaretAfterElement(first);
  }

  // Wrap the current (non-collapsed, single-block) selection in `el`.
  // `flatten` replaces the wrapped content by its plain text (for <code>).
  function wrapSelection(el, { flatten = false } = {}) {
    const range = getSelectionRange();
    if (!range || range.collapsed) return false;
    if (getBlockNode(range.startContainer) !== getBlockNode(range.endContainer)) return false;
    const fragment = range.extractContents();
    if (flatten) el.textContent = fragment.textContent;
    else el.appendChild(fragment);
    range.insertNode(el);
    const r = document.createRange();
    r.selectNodeContents(el);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(r);
    return true;
  }

  function toggleInlineCode() {
    const range = getSelectionRange();
    if (!range) return;
    const code = getAncestorTag(range.startContainer, 'CODE');
    if (code) {
      unwrapElement(code);
      return;
    }
    if (hasAncestorTag(range.startContainer, 'PRE')) return;
    wrapSelection(document.createElement('code'), { flatten: true });
  }

  function toggleLink() {
    const range = getSelectionRange();
    if (!range) return;
    const a = getAncestorTag(range.startContainer, 'A');
    if (a) {
      unwrapElement(a);
      return;
    }
    if (hasAncestorTag(range.startContainer, 'PRE')) return;
    const href = window.prompt('Link URL');
    if (!href || !isSafeUrl(href)) return;
    const link = document.createElement('a');
    link.setAttribute('href', href);
    if (range.collapsed) {
      link.textContent = href;
      range.insertNode(link);
      placeCaretAfterElement(link);
      markJustClosed(link);
    } else if (!wrapSelection(link)) {
      return;
    }
  }

  function insertImage() {
    const range = getSelectionRange();
    if (!range || hasAncestorTag(range.startContainer, 'PRE')) return;
    const src = window.prompt('Image URL');
    if (!src || !isSafeUrl(src)) return;
    const img = document.createElement('img');
    img.setAttribute('src', src);
    img.setAttribute('alt', '');
    range.deleteContents();
    range.insertNode(img);
    placeCaretAfterElement(img);
  }

  // Relative URLs, the usual web schemes and links into the vault (`id:`,
  // vault.md#links-in-content) only -- never `javascript:`.
  function isSafeUrl(url) {
    const m = /^([a-zA-Z][a-zA-Z0-9+.-]*):/.exec(url.trim());
    return !m || ['http', 'https', 'mailto', 'tel', 'id'].includes(m[1].toLowerCase());
  }

  // ---------------------------------------------------------------------------
  // Tables -- see editor.md#tables for the shape and the editing model.
  function getEnclosingCell(node) {
    if (!node) return null;
    if (node.nodeType === Node.TEXT_NODE) node = node.parentNode;
    const cell = node.closest && node.closest('td, th');
    return cell && editor.contains(cell) ? cell : null;
  }

  function tableRows(table) {
    return Array.from(table.querySelectorAll(':scope > thead > tr, :scope > tbody > tr'));
  }

  function tableCells(table) {
    return tableRows(table).flatMap(row => Array.from(row.children));
  }

  function cellIndex(cell) {
    return Array.prototype.indexOf.call(cell.parentNode.children, cell);
  }

  function isHeaderRow(row) {
    return row.parentNode.tagName === 'THEAD';
  }

  function createCell(tagName) {
    const cell = document.createElement(tagName);
    cell.appendChild(document.createElement('br'));
    return cell;
  }

  function createRow(cols, cellTag) {
    const tr = document.createElement('tr');
    for (let c = 0; c < cols; c++) tr.appendChild(createCell(cellTag));
    return tr;
  }

  // Select a cell's content so typing replaces it (spreadsheet-style).
  function moveToCell(cell) {
    const r = document.createRange();
    r.selectNodeContents(cell);
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(r);
  }

  // The block after `el`, creating a paragraph if there is none or the next
  // block can't hold a caret (rule, table).
  function blockAfter(el) {
    let next = el.nextSibling;
    if (!next || !CONVERTIBLE_TAGS.includes(next.tagName)) {
      // (the trailing-paragraph invariant makes this rare at document end)
      const p = document.createElement('p');
      p.appendChild(document.createElement('br'));
      el.parentNode.insertBefore(p, next);
      next = p;
    }
    return next;
  }

  function insertTable(cols = 2, bodyRows = 2) {
    const table = document.createElement('table');
    const thead = document.createElement('thead');
    thead.appendChild(createRow(cols, 'th'));
    const tbody = document.createElement('tbody');
    for (let r = 0; r < bodyRows; r++) tbody.appendChild(createRow(cols, 'td'));
    table.appendChild(thead);
    table.appendChild(tbody);

    const range = getSelectionRange();
    const block = range ? getBlockNode(range.startContainer) : null;
    const root = range ? getRootBlock(range.startContainer) : null;
    if (block && block.parentNode === editor && block.tagName === 'P' && isEmptyBlock(block)) {
      block.replaceWith(table);
    } else if (root) {
      editor.insertBefore(table, root.nextSibling);
    } else {
      editor.appendChild(table);
    }
    blockAfter(table);
    moveToCell(thead.firstChild.firstChild);
  }

  function insertRowBelow(cell) {
    const row = cell.parentNode;
    const table = row.closest('table');
    const tr = createRow(row.children.length, 'td');
    if (isHeaderRow(row)) {
      let tbody = table.querySelector(':scope > tbody');
      if (!tbody) {
        tbody = document.createElement('tbody');
        table.appendChild(tbody);
      }
      tbody.insertBefore(tr, tbody.firstChild);
    } else {
      row.parentNode.insertBefore(tr, row.nextSibling);
    }
    moveToCell(tr.children[Math.min(cellIndex(cell), tr.children.length - 1)]);
  }

  // The header row can't be deleted (markdown needs one); delete the table.
  function deleteRow(cell) {
    const row = cell.parentNode;
    if (isHeaderRow(row)) return;
    const table = row.closest('table');
    const rows = tableRows(table);
    const idx = rows.indexOf(row);
    const col = cellIndex(cell);
    row.remove();
    const target = rows[idx + 1] || rows[idx - 1];
    moveToCell(target.children[Math.min(col, target.children.length - 1)]);
  }

  function insertColumnRight(cell) {
    const idx = cellIndex(cell);
    const table = cell.closest('table');
    let target = null;
    for (const row of tableRows(table)) {
      const ref = row.children[idx];
      const fresh = createCell(isHeaderRow(row) ? 'th' : 'td');
      row.insertBefore(fresh, ref ? ref.nextSibling : null);
      if (row === cell.parentNode) target = fresh;
    }
    moveToCell(target);
  }

  function deleteColumn(cell) {
    const table = cell.closest('table');
    const row = cell.parentNode;
    if (row.children.length <= 1) {
      deleteTable(table);
      return;
    }
    const idx = cellIndex(cell);
    for (const r of tableRows(table)) r.children[idx]?.remove();
    moveToCell(row.children[Math.min(idx, row.children.length - 1)]);
  }

  function deleteTable(table) {
    const next = blockAfter(table);
    table.remove();
    placeCaretAtStart(next);
  }

  // Tab: next cell, appending a row after the last one. Shift+Tab: previous.
  function tabInTable(cell, backwards) {
    const cells = tableCells(cell.closest('table'));
    const i = cells.indexOf(cell);
    if (backwards) {
      if (i > 0) moveToCell(cells[i - 1]);
    } else if (i < cells.length - 1) {
      moveToCell(cells[i + 1]);
    } else {
      insertRowBelow(cell);
    }
  }

  // Enter: the cell below; from the last row, the block after the table.
  function enterInTable(cell) {
    const row = cell.parentNode;
    const table = row.closest('table');
    const rows = tableRows(table);
    const below = rows[rows.indexOf(row) + 1];
    if (below) {
      moveToCell(below.children[Math.min(cellIndex(cell), below.children.length - 1)]);
    } else {
      placeCaretAtStart(blockAfter(table));
    }
  }

  // ---------------------------------------------------------------------------
  // Typed markdown shortcuts -- see editor.md#shortcuts for the full table.
  function tryBlockAutoformat() {
    const range = getSelectionRange();
    if (!range || !range.collapsed) return false;
    const container = range.startContainer;
    if (container.nodeType !== Node.TEXT_NODE) return false;
    if (range.startOffset > 8) return false;
    const block = getBlockNode(container);
    if (!block || !CONVERTIBLE_TAGS.includes(block.tagName)) return false;

    const preRange = document.createRange();
    preRange.selectNodeContents(block);
    preRange.setEnd(container, range.startOffset);
    // Chrome turns a trailing space into &nbsp; inside contenteditable.
    const textBefore = preRange.toString().replace(/\u00a0/g, ' ');

    const headingMatch = textBefore.match(/^(#{1,6}) $/);
    const bulletMatch = /^[-*] $/.test(textBefore);
    const numberMatch = textBefore.match(/^(\d{1,3})\. $/);
    const quoteMatch = /^> $/.test(textBefore);
    if (!headingMatch && !bulletMatch && !numberMatch && !quoteMatch) return false;

    preRange.deleteContents();

    let target;
    if (bulletMatch) target = wrapBlockAsListItem(block, 'ul');
    else if (numberMatch) target = wrapBlockAsListItem(block, 'ol', Number(numberMatch[1]));
    else if (quoteMatch) target = wrapBlockAsQuote(block);
    else target = replaceBlockTag(block, 'h' + headingMatch[1].length);
    placeCaretAtStart(target);
    return true;
  }

  // "```" -> fenced code block, triggered as soon as the third backtick is typed.
  function tryCodeBlockAutoformat() {
    const range = getSelectionRange();
    if (!range || !range.collapsed) return false;
    const container = range.startContainer;
    if (container.nodeType !== Node.TEXT_NODE) return false;
    if (range.startOffset > 3) return false;
    const block = getBlockNode(container);
    if (!block || !CONVERTIBLE_TAGS.includes(block.tagName)) return false;

    const preRange = document.createRange();
    preRange.selectNodeContents(block);
    preRange.setEnd(container, range.startOffset);
    if (preRange.toString() !== '```') return false;

    preRange.deleteContents();
    placeCaretAtStart(wrapBlockAsCode(block));
    return true;
  }

  // "---" / "***" / "___" alone in a paragraph -> horizontal rule.
  function tryRuleAutoformat() {
    const block = getSelectionBlock();
    if (!block || block.tagName !== 'P') return false;
    const text = block.textContent.replace(/\u00a0/g, ' ');
    if (!/^(---|\*\*\*|___)$/.test(text)) return false;
    replaceBlockWithRule(block);
    return true;
  }

  // `text` -> inline code span, triggered as soon as the closing backtick is
  // typed. Unlike emphasis, CommonMark code spans don't require a
  // whitespace boundary (`a`b`c` is valid), so there's no "before" check.
  function matchInlineCode(text) {
    if (!text.endsWith('`')) return null;
    for (let i = text.length - 2; i >= 0; i--) {
      if (text[i] !== '`') continue;
      const inner = text.slice(i + 1, text.length - 1);
      if (inner.length > 0 && !inner.includes('`')) {
        return { start: i, inner };
      }
      return null;
    }
    return null;
  }

  function tryInlineCodeAutoformat() {
    const range = getSelectionRange();
    if (!range || !range.collapsed) return false;
    const node = range.startContainer;
    if (node.nodeType !== Node.TEXT_NODE) return false;
    // Backticks typed inside a code block, or an existing code span, are
    // literal characters, not a new span.
    if (hasAncestorTag(node, 'PRE') || hasAncestorTag(node, 'CODE')) return false;

    const text = node.textContent.slice(0, range.startOffset);
    const match = matchInlineCode(text);
    if (!match) return false;

    const replaceRange = document.createRange();
    replaceRange.setStart(node, match.start);
    replaceRange.setEnd(node, range.startOffset);
    replaceRange.deleteContents();

    const code = document.createElement('code');
    code.textContent = match.inner;
    replaceRange.insertNode(code);
    placeCaretAfterElement(code);
    markJustClosed(code);
    return true;
  }

  function isValidEmphasisContent(inner, delim) {
    return inner.length > 0 &&
      !inner.includes(delim) &&
      !/\s/.test(inner[0]) &&
      !/\s/.test(inner[inner.length - 1]);
  }

  // The triple case is checked first since "***x***" also trivially ends
  // with "**" and with "*".
  function matchEmphasis(text, delim) {
    const triple = delim + delim + delim;
    if (text.endsWith(triple) && text.length >= 7) {
      const openIdx = text.lastIndexOf(triple, text.length - 4);
      if (openIdx !== -1) {
        const before = openIdx === 0 ? null : text[openIdx - 1];
        const inner = text.slice(openIdx + 3, text.length - 3);
        if ((before === null || /\s/.test(before)) && isValidEmphasisContent(inner, delim)) {
          return { combined: true, start: openIdx, inner };
        }
      }
      return null;
    }
    const double = delim + delim;
    if (text.endsWith(double) && text.length >= 5) {
      const openIdx = text.lastIndexOf(double, text.length - 3);
      if (openIdx !== -1) {
        const before = openIdx === 0 ? null : text[openIdx - 1];
        const inner = text.slice(openIdx + 2, text.length - 2);
        if ((before === null || /\s/.test(before)) && isValidEmphasisContent(inner, delim)) {
          return { bold: true, start: openIdx, inner };
        }
      }
      return null;
    }
    if (text.endsWith(delim)) {
      for (let i = text.length - 2; i >= 0; i--) {
        if (text[i] !== delim) continue;
        if (i > 0 && text[i - 1] === delim) return null; // second char of a "**"/"__" opener
        const before = i === 0 ? null : text[i - 1];
        const inner = text.slice(i + 1, text.length - 1);
        if ((before === null || /\s/.test(before)) && isValidEmphasisContent(inner, delim)) {
          return { bold: false, start: i, inner };
        }
        return null;
      }
    }
    return null;
  }

  function tryInlineAutoformat(delim) {
    const range = getSelectionRange();
    if (!range || !range.collapsed) return false;
    const node = range.startContainer;
    if (node.nodeType !== Node.TEXT_NODE) return false;
    if (hasAncestorTag(node, 'PRE') || hasAncestorTag(node, 'CODE')) return false;
    const text = node.textContent.slice(0, range.startOffset);
    if (text[text.length - 1] !== delim) return false;

    const match = matchEmphasis(text, delim);
    if (!match) return false;
    // "~" only has the double form (~~strike~~).
    if (delim === '~' && !match.bold) return false;

    // Skip whichever wrapper an ancestor already satisfies (editor.md#shortcuts).
    const alreadyStrong = hasAncestorTag(node, 'STRONG');
    const alreadyEm = hasAncestorTag(node, 'EM');
    const alreadyDel = hasAncestorTag(node, 'DEL');

    const replaceRange = document.createRange();
    replaceRange.setStart(node, match.start);
    replaceRange.setEnd(node, range.startOffset);
    replaceRange.deleteContents();

    let content;
    if (delim === '~') {
      if (alreadyDel) {
        content = document.createTextNode(match.inner);
      } else {
        content = document.createElement('del');
        content.textContent = match.inner;
      }
    } else {
      const wantStrong = (match.bold || match.combined) && !alreadyStrong;
      const wantEm = (!match.bold || match.combined) && !alreadyEm;
      if (wantStrong && wantEm) {
        content = document.createElement('strong');
        const em = document.createElement('em');
        em.textContent = match.inner;
        content.appendChild(em);
      } else if (wantStrong) {
        content = document.createElement('strong');
        content.textContent = match.inner;
      } else if (wantEm) {
        content = document.createElement('em');
        content.textContent = match.inner;
      } else {
        content = document.createTextNode(match.inner);
      }
    }
    replaceRange.insertNode(content);

    if (content.nodeType === Node.ELEMENT_NODE) {
      placeCaretAfterElement(content);
      markJustClosed(content);
    } else {
      placeCaretAfterElement(content);
    }
    return true;
  }

  // "[text](url)" -> link, "![alt](url)" -> image, triggered on the closing
  // parenthesis.
  function tryLinkAutoformat() {
    const range = getSelectionRange();
    if (!range || !range.collapsed) return false;
    const node = range.startContainer;
    if (node.nodeType !== Node.TEXT_NODE) return false;
    if (hasAncestorTag(node, 'PRE') || hasAncestorTag(node, 'CODE') || hasAncestorTag(node, 'A')) return false;

    const text = node.textContent.slice(0, range.startOffset);
    const match = /(!?)\[([^[\]]+)\]\(([^()\s]+)\)$/.exec(text);
    if (!match || !isSafeUrl(match[3])) return false;

    const replaceRange = document.createRange();
    replaceRange.setStart(node, match.index);
    replaceRange.setEnd(node, range.startOffset);
    replaceRange.deleteContents();

    let el;
    if (match[1] === '!') {
      el = document.createElement('img');
      el.setAttribute('src', match[3]);
      el.setAttribute('alt', match[2]);
    } else {
      el = document.createElement('a');
      el.setAttribute('href', match[3]);
      el.textContent = match[2];
    }
    replaceRange.insertNode(el);
    placeCaretAfterElement(el);
    if (el.tagName === 'A') markJustClosed(el);
    return true;
  }

  // ---------------------------------------------------------------------------
  // execCommand inserts <b>/<i>/<strike>; renamed in place for consistency
  // with the typed shortcuts (editor.md#toolbar).
  function normalizeInlineTags() {
    editor.querySelectorAll('b').forEach(el => renameElement(el, 'strong'));
    editor.querySelectorAll('i').forEach(el => renameElement(el, 'em'));
    editor.querySelectorAll('strike, s').forEach(el => renameElement(el, 'del'));
  }

  function renameElement(el, tagName) {
    const saved = captureSelection();
    const replacement = document.createElement(tagName);
    while (el.firstChild) replacement.appendChild(el.firstChild);
    el.parentNode.replaceChild(replacement, el);
    if (document.activeElement === editor) {
      // execCommand often anchors the selection on the element itself
      // (not a text node inside it); remap that to the new element.
      restoreSelection(saved, n => (n === el ? replacement : n));
    }
    return replacement;
  }

  // ---------------------------------------------------------------------------
  // Explicit exit from blockquotes and code blocks on a second Enter --
  // see editor.md#blockquotes, editor.md#code-blocks.
  function isEmptyBlock(block) {
    return block.textContent === '';
  }

  // Reuse an empty paragraph that already follows the block (e.g. the
  // trailing one) instead of stacking a second.
  function exitBlock(block) {
    let p = block.nextSibling;
    if (!isEmptyParagraph(p)) {
      p = document.createElement('p');
      p.appendChild(document.createElement('br'));
      block.parentNode.insertBefore(p, block.nextSibling);
    }
    placeCaretAtStart(p);
  }

  function tryExitBlockOnEnter() {
    const range = getSelectionRange();
    if (!range || !range.collapsed) return false;
    const block = getBlockNode(range.startContainer);
    if (!block) return false;
    if (CONVERTIBLE_TAGS.includes(block.tagName) && block.parentNode.tagName === 'BLOCKQUOTE' && isEmptyBlock(block)) {
      // Splits the quote at `block` (editor.md#blockquotes, exiting on Enter) rather
      // than always exiting after the quote's last child.
      const quote = block.parentNode;
      const following = [];
      let sib = block.nextSibling;
      while (sib) { following.push(sib); sib = sib.nextSibling; }
      quote.removeChild(block);

      const rest = following.length ? document.createElement('blockquote') : null;
      if (rest) following.forEach(n => rest.appendChild(n));

      if (!rest && quote.firstChild) {
        exitBlock(quote); // nothing to split off; may reuse the trailing paragraph
      } else {
        // A real split: the paragraph must sit exactly between the two
        // halves, so it's always fresh, never reused from elsewhere.
        const p = document.createElement('p');
        p.appendChild(document.createElement('br'));
        if (quote.firstChild) {
          quote.parentNode.insertBefore(p, quote.nextSibling);
        } else {
          quote.parentNode.replaceChild(p, quote);
        }
        if (rest) p.parentNode.insertBefore(rest, p.nextSibling);
        placeCaretAtStart(p);
      }
      return true;
    }
    if (block.tagName === 'PRE' && block.lastChild?.tagName === 'BR' && isCaretAtEndOf(block)) {
      while (block.lastChild?.tagName === 'BR') block.lastChild.remove();
      exitBlock(block);
      return true;
    }
    return false;
  }

  // ---------------------------------------------------------------------------
  // Paste / drop sanitization -- see editor.md#paste.
  const ALLOWED_PASTE_TAGS = new Map([
    ['P', []], ['H1', []], ['H2', []], ['H3', []], ['H4', []], ['H5', []], ['H6', []],
    ['UL', []], ['OL', ['start']], ['LI', []], ['BLOCKQUOTE', []], ['PRE', []], ['HR', []],
    ['TABLE', []], ['THEAD', []], ['TBODY', []], ['TR', []], ['TH', ['align']], ['TD', ['align']],
    ['STRONG', []], ['EM', []], ['DEL', []], ['CODE', []], ['BR', []],
    ['A', ['href', 'title']], ['IMG', ['src', 'alt', 'title']],
  ]);
  const RENAMED_PASTE_TAGS = new Map([['B', 'STRONG'], ['I', 'EM'], ['S', 'DEL'], ['STRIKE', 'DEL']]);
  const DISCARDED_PASTE_TAGS = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'SVG', 'IFRAME', 'OBJECT', 'EMBED', 'VIDEO', 'AUDIO']);

  function sanitizePastedHTML(html) {
    const source = document.createElement('template');
    source.innerHTML = html;
    const output = document.createDocumentFragment();

    function appendSanitized(parent, node, inPre) {
      if (node.nodeType === Node.TEXT_NODE) {
        parent.appendChild(document.createTextNode(node.data));
        return;
      }
      if (node.nodeType !== Node.ELEMENT_NODE || DISCARDED_PASTE_TAGS.has(node.tagName)) return;
      const tagName = RENAMED_PASTE_TAGS.get(node.tagName) || node.tagName;
      // Code blocks are a bare <pre>; a nested <code> is flattened away.
      const allowed = ALLOWED_PASTE_TAGS.get(tagName);
      if (allowed && !(inPre && tagName === 'CODE')) {
        const clean = document.createElement(tagName.toLowerCase());
        for (const name of allowed) {
          const value = node.getAttribute(name);
          if (value === null) continue;
          if ((name === 'href' || name === 'src') && !isSafeUrl(value)) {
            if (tagName === 'A') break; // becomes plain text below
            return; // an image without a safe source is dropped
          }
          clean.setAttribute(name, value);
        }
        if (tagName === 'A' && !clean.hasAttribute('href')) {
          Array.from(node.childNodes).forEach(child => appendSanitized(parent, child, inPre));
          return;
        }
        parent.appendChild(clean);
        Array.from(node.childNodes).forEach(child => appendSanitized(clean, child, inPre || tagName === 'PRE'));
        return;
      }
      Array.from(node.childNodes).forEach(child => appendSanitized(parent, child, inPre));
    }

    Array.from(source.content.childNodes).forEach(node => appendSanitized(output, node, false));
    return output;
  }

  function insertPastedContent(dataTransfer) {
    const range = getSelectionRange();
    if (!range) return;
    const html = dataTransfer.getData('text/html');
    const plainText = dataTransfer.getData('text/plain');
    const fragment = html ? sanitizePastedHTML(html) : document.createTextNode(plainText);
    const currentBlock = getBlockNode(range.startContainer);
    const hasBlock = Array.from(fragment.childNodes).some(node =>
      node.nodeType === Node.ELEMENT_NODE && ROOT_BLOCK_TAGS.includes(node.tagName));
    range.deleteContents();
    const lastNode = fragment.lastChild;
    if (currentBlock?.tagName === 'P' && isEmptyBlock(currentBlock)) {
      if (hasBlock) {
        currentBlock.replaceWith(fragment);
      } else {
        currentBlock.replaceChildren();
        const insertionRange = document.createRange();
        insertionRange.selectNodeContents(currentBlock);
        insertionRange.collapse(true);
        insertionRange.insertNode(fragment);
      }
    } else {
      range.insertNode(fragment);
    }
    if (lastNode) placeCaretAfterElement(lastNode);
    afterEdit();
  }

  // ---------------------------------------------------------------------------
  // Commands (toolbar + keyboard).
  function toggleList(listTag) {
    const range = getSelectionRange();
    if (!range) return;
    const li = getEnclosingListItem(range.startContainer);
    if (li) {
      if (li.parentNode.tagName === listTag.toUpperCase()) {
        convertListItemToParagraph(li);
      } else {
        // Switch the whole list's kind.
        const saved = captureSelection();
        const list = li.parentNode;
        const replacement = document.createElement(listTag);
        while (list.firstChild) replacement.appendChild(list.firstChild);
        list.parentNode.replaceChild(replacement, list);
        restoreSelection(saved, n => (n === list ? replacement : n));
      }
      return;
    }
    const block = getBlockNode(range.startContainer);
    if (!block || !CONVERTIBLE_TAGS.includes(block.tagName)) return;
    wrapBlockAsListItem(block, listTag);
  }

  function toggleQuote() {
    const block = getSelectionBlock();
    if (!block) return;
    if (block.parentNode.tagName === 'BLOCKQUOTE') {
      unwrapFromQuote(block);
    } else if (CONVERTIBLE_TAGS.includes(block.tagName)) {
      wrapBlockAsQuote(block);
    }
  }

  function toggleCodeBlock() {
    const block = getSelectionBlock();
    if (!block) return;
    if (block.tagName === 'PRE') {
      replaceBlockTag(block, 'p');
    } else if (CONVERTIBLE_TAGS.includes(block.tagName)) {
      wrapBlockAsCode(block);
    }
  }

  function setBlockTag(tagName) {
    const block = getSelectionBlock();
    if (!block || !CONVERTIBLE_TAGS.includes(block.tagName)) return;
    replaceBlockTag(block, block.tagName === tagName.toUpperCase() ? 'p' : tagName.toLowerCase());
  }

  function runCommand(cmd) {
    switch (cmd) {
      case 'bold': document.execCommand('bold', false, null); break;
      case 'italic': document.execCommand('italic', false, null); break;
      case 'strike': document.execCommand('strikeThrough', false, null); break;
      case 'code': toggleInlineCode(); break;
      case 'ul': toggleList('ul'); break;
      case 'ol': toggleList('ol'); break;
      case 'quote': toggleQuote(); break;
      case 'codeblock': toggleCodeBlock(); break;
      case 'hr': insertRule(); break;
      case 'link': toggleLink(); break;
      case 'image': insertImage(); break;
      case 'table': insertTable(); break;
      default: return runTableCommand(cmd);
    }
    return true;
  }

  function runTableCommand(cmd) {
    const range = getSelectionRange();
    const cell = range ? getEnclosingCell(range.startContainer) : null;
    if (!cell) return false;
    switch (cmd) {
      case 'row-add': insertRowBelow(cell); break;
      case 'row-del': deleteRow(cell); break;
      case 'col-add': insertColumnRight(cell); break;
      case 'col-del': deleteColumn(cell); break;
      case 'table-del': deleteTable(cell.closest('table')); break;
      default: return false;
    }
    return true;
  }

  // ---------------------------------------------------------------------------
  // Toolbar -- see editor.md#toolbar. State comes from the element
  // structure, not computed style, so heading weight doesn't make Bold
  // appear active.
  function getToolbarCommandState(command) {
    const range = getSelectionRange();
    if (!range) return false;
    const node = range.startContainer;
    const li = getEnclosingListItem(node);
    const block = getBlockNode(node);
    switch (command) {
      case 'bold': return hasAncestorTag(node, 'STRONG');
      case 'italic': return hasAncestorTag(node, 'EM');
      case 'strike': return hasAncestorTag(node, 'DEL');
      case 'code': return hasAncestorTag(node, 'CODE');
      case 'link': return hasAncestorTag(node, 'A');
      case 'ul': return Boolean(li && li.parentNode.tagName === 'UL');
      case 'ol': return Boolean(li && li.parentNode.tagName === 'OL');
      case 'quote': return hasAncestorTag(node, 'BLOCKQUOTE');
      case 'codeblock': return Boolean(block && block.tagName === 'PRE');
      default: return false;
    }
  }

  function updateToolbarState() {
    const focused = document.activeElement === editor;
    const range = focused ? getSelectionRange() : null;
    const inTable = Boolean(range && getEnclosingCell(range.startContainer));
    cmdButtons.forEach(btn => {
      btn.setAttribute('aria-pressed', String(focused && getToolbarCommandState(btn.dataset.cmd)));
    });
    // Table tools only while in a table; no nested tables.
    toolbar.querySelectorAll('[data-table-only]').forEach(el => { el.hidden = !inTable; });
    toolbar.querySelectorAll('[data-cmd="table"]').forEach(el => { el.hidden = inTable; });
    const currentBlock = focused ? getCurrentBlockTag() : null;
    blockButtons.forEach(btn => {
      btn.setAttribute('aria-pressed', String(btn.dataset.block === currentBlock));
    });
  }

  // mousedown (not click) so the editor keeps focus and its selection.
  function onToolbarMouseDown(e) {
    const btn = e.target.closest('button[data-cmd], button[data-block]');
    if (!btn || !isEditable()) return;
    e.preventDefault();
    editor.focus();
    if (btn.dataset.cmd) {
      runCommand(btn.dataset.cmd);
    } else {
      setBlockTag(btn.dataset.block);
    }
    afterEdit();
  }

  // ---------------------------------------------------------------------------
  // Event handlers.
  function onKeyDown(e) {
    if (e.key === 'Escape') {
      if (onEscape) {
        e.preventDefault();
        onEscape();
      }
      return;
    }
    if (e.key === 'Tab') {
      // Never let Tab leave the editor while editing.
      e.preventDefault();
      const range = getSelectionRange();
      if (!range) return;
      const cell = getEnclosingCell(range.startContainer);
      const li = getEnclosingListItem(range.startContainer);
      const block = getBlockNode(range.startContainer);
      if (cell) {
        tabInTable(cell, e.shiftKey);
        afterEdit();
      } else if (li) {
        if (e.shiftKey) outdentListItem(li); else indentListItem(li);
        afterEdit();
      } else if (block?.parentNode.tagName === 'BLOCKQUOTE') {
        if (e.shiftKey) unwrapFromQuote(block); else indentQuotedBlock(block);
        afterEdit();
      } else if (hasAncestorTag(range.startContainer, 'PRE') && !e.shiftKey) {
        document.execCommand('insertText', false, '\t');
      }
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      // No Ctrl+Alt combinations -- see editor.md#toolbar.
      const key = e.key.toLowerCase();
      let cmd = null;
      if (!e.altKey && !e.shiftKey && key === 'b') cmd = 'bold';
      else if (!e.altKey && !e.shiftKey && key === 'i') cmd = 'italic';
      else if (!e.altKey && !e.shiftKey && key === 'e') cmd = 'code';
      else if (!e.altKey && !e.shiftKey && key === 'k') cmd = 'link';
      else if (!e.altKey && e.shiftKey && key === 'x') cmd = 'strike';
      else if (!e.altKey && e.shiftKey && e.code === 'Digit7') cmd = 'ol';
      else if (!e.altKey && e.shiftKey && e.code === 'Digit8') cmd = 'ul';
      else if (!e.altKey && e.shiftKey && e.code === 'Digit9') cmd = 'quote';
      if (cmd) {
        e.preventDefault();
        runCommand(cmd);
        afterEdit();
      }
      return;
    }
    if (e.key === 'Backspace') {
      const range = getSelectionRange();
      const li = range ? getEnclosingListItem(range.startContainer) : null;
      if (li && isCaretAtStartOf(li)) {
        e.preventDefault();
        convertListItemToParagraph(li);
        afterEdit();
        return;
      }
      const block = range ? getBlockNode(range.startContainer) : null;
      if (block && MERGEABLE_TAGS.includes(block.tagName) && isCaretAtStartOf(block) &&
        mergeBlockIntoPrevious(block)) {
        e.preventDefault();
        afterEdit();
      }
      return;
    }
    if (e.key === 'Delete') {
      const range = getSelectionRange();
      const block = range ? getBlockNode(range.startContainer) : null;
      const next = block ? block.nextElementSibling : null;
      if (block && next && MERGEABLE_TAGS.includes(next.tagName) &&
        isCaretAtEndOf(block) && mergeBlockIntoPrevious(next)) {
        e.preventDefault();
        afterEdit();
      }
      return;
    }
    if (e.key === 'Enter' && onSubmit && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      onSubmit();
      return;
    }
    if (e.key === 'Enter') {
      const range = getSelectionRange();
      const cell = range ? getEnclosingCell(range.startContainer) : null;
      if (cell) {
        e.preventDefault();
        enterInTable(cell);
        afterEdit();
      } else if (tryExitBlockOnEnter()) {
        e.preventDefault();
        afterEdit();
      }
    }
  }

  function onPaste(e) {
    e.preventDefault();
    insertPastedContent(e.clipboardData);
  }

  function onDrop(e) {
    e.preventDefault();
    insertPastedContent(e.dataTransfer);
  }

  function onInput(e) {
    healLeakedTyping();
    ensureBlockStructure();
    ensureTrailingParagraph();
    if (!e.isComposing) {
      switch (e.data) {
        case ' ': tryBlockAutoformat(); break;
        case '*': case '_': tryInlineAutoformat(e.data) || tryRuleAutoformat(); break;
        case '~': tryInlineAutoformat('~'); break;
        case '-': tryRuleAutoformat(); break;
        case '`': tryCodeBlockAutoformat() || tryInlineCodeAutoformat(); break;
        case ')': tryLinkAutoformat(); break;
        default: break;
      }
      normalizeInlineTags();
    }
    scheduleToolbarUpdate();
    notifyChange();
  }

  // Links never navigate the app away -- see editor.md#links.
  function onClick(e) {
    const a = e.target.closest('a');
    if (!a || !editor.contains(a)) return;
    e.preventDefault();
    if (!isEditable() && onLinkClick) onLinkClick(a.getAttribute('href'));
  }

  function onSelectionChange() {
    if (document.activeElement === editor) scheduleToolbarUpdate();
  }

  function onBlur() {
    scheduleToolbarUpdate();
  }

  editor.addEventListener('keydown', onKeyDown);
  editor.addEventListener('paste', onPaste);
  editor.addEventListener('drop', onDrop);
  editor.addEventListener('input', onInput);
  editor.addEventListener('click', onClick);
  editor.addEventListener('blur', onBlur);
  document.addEventListener('selectionchange', onSelectionChange);
  if (toolbar) toolbar.addEventListener('mousedown', onToolbarMouseDown);

  // ---------------------------------------------------------------------------
  // Public API.
  function isEditable() {
    return editor.getAttribute('contenteditable') === 'true';
  }

  function setEditable(flag, focus = true) {
    editor.setAttribute('contenteditable', flag ? 'true' : 'false');
    if (toolbar) toolbar.hidden = !flag;
    if (flag) {
      ensureTrailingParagraph();
      if (focus) editor.focus();
    } else {
      justClosedInline = null;
      if (document.activeElement === editor) editor.blur();
      removeTrailingParagraph();
    }
    scheduleToolbarUpdate();
  }

  // Replace the whole content. `html` must be in the shape described in
  // av-vault's markdown module (no whitespace between blocks etc.).
  function setHTML(html) {
    justClosedInline = null;
    editor.innerHTML = html;
    normalizeInlineTags();
    ensureBlockStructure();
    scheduleToolbarUpdate();
  }

  function getHTML() {
    return editor.innerHTML;
  }

  function destroy() {
    editor.removeEventListener('keydown', onKeyDown);
    editor.removeEventListener('paste', onPaste);
    editor.removeEventListener('drop', onDrop);
    editor.removeEventListener('input', onInput);
    editor.removeEventListener('click', onClick);
    editor.removeEventListener('blur', onBlur);
    document.removeEventListener('selectionchange', onSelectionChange);
    if (toolbar) toolbar.removeEventListener('mousedown', onToolbarMouseDown);
    toolbarUpdate.cancel();
  }

  setEditable(false);

  return { setHTML, getHTML, setEditable, isEditable, focus: options => editor.focus(options), destroy };
}
