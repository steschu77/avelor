// The blocks (editor.md#block-model): a node's block and root block, the
// structure and the trailing paragraph, lists, quotes and code blocks,
// merging, the rule, an element's ancestors, inline tags renamed, and the exit
// on Enter.

import { HEADING_TAGS, ROOT_BLOCK_TAGS } from './tags.js';

export function createBlocks(editor, {
  captureSelection, getSelectionRange, isCaretAtEndOf, isEditable, maxOffset, placeCaretAfterElement,
  placeCaretAtStart, restoreSelection, selectRange, selectionInside,
}) {
  // ---------------------------------------------------------------------------
  // Block model -- see editor.md#block-model for what each set means and why.
  const LEAF_BLOCK_TAGS = ['P', ...HEADING_TAGS, 'LI', 'PRE', 'TH', 'TD'];
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

  // A paragraph with nothing in it but the <br> that keeps it open for the
  // caret.
  function emptyParagraph() {
    const p = document.createElement('p');
    p.appendChild(document.createElement('br'));
    return p;
  }

  function isEmptyParagraph(node) {
    return Boolean(node) && node.tagName === 'P' && node.textContent === '';
  }

  function ensureTrailingParagraph() {
    if (!isEditable()) return;
    const last = editor.lastElementChild;
    if (!last || SELF_EXITING_TAGS.includes(last.tagName)) return;
    const p = emptyParagraph();
    editor.appendChild(p);
  }

  function removeTrailingParagraph() {
    const last = editor.lastElementChild;
    const prev = last?.previousElementSibling;
    if (isEmptyParagraph(last) && prev && !SELF_EXITING_TAGS.includes(prev.tagName)) last.remove();
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
      selectRange(r);
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

  function followingSiblings(node) {
    const following = [];
    for (let sib = node.nextSibling; sib; sib = sib.nextSibling) following.push(sib);
    return following;
  }

  // A new blockquote holding `nodes`, moved into it; null for none: the rest
  // of a quote split at a block.
  function quoteOf(nodes) {
    if (!nodes.length) return null;
    const quote = document.createElement('blockquote');
    nodes.forEach(n => quote.appendChild(n));
    return quote;
  }

  // Pull `block` out of its blockquote, splitting the quote around it.
  function unwrapFromQuote(block) {
    const quote = block.parentNode;
    const following = followingSiblings(block);
    const saved = captureSelection();
    quote.parentNode.insertBefore(block, quote.nextSibling);
    const rest = quoteOf(following);
    if (rest) block.parentNode.insertBefore(rest, block.nextSibling);
    if (!quote.firstChild) quote.remove();
    restoreSelection(saved);
    if (!selectionInside(block)) placeCaretAtStart(block);
  }

  // Replace `block` with a rule followed by a fresh paragraph holding the caret.
  function replaceBlockWithRule(block) {
    const hr = document.createElement('hr');
    const p = emptyParagraph();
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
    const p = emptyParagraph();
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

  // ---------------------------------------------------------------------------
  // execCommand inserts <b>/<i>/<strike>; renamed in place for consistency
  // with the typed shortcuts (editor.md#toolbar).
  function normalizeInlineTags() {
    editor.querySelectorAll('b').forEach(el => renameElement(el, 'strong'));
    editor.querySelectorAll('i').forEach(el => renameElement(el, 'em'));
    editor.querySelectorAll('strike, s').forEach(el => renameElement(el, 'del'));
  }

  // execCommand('strikeThrough') takes back only <strike>/<s>, not the
  // editor's <del>: renamed for it, and back by normalizeInlineTags.
  function strikeForCommand() {
    editor.querySelectorAll('del').forEach(el => renameElement(el, 'strike'));
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
      p = emptyParagraph();
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
      const following = followingSiblings(block);
      quote.removeChild(block);

      const rest = quoteOf(following);

      if (!rest && quote.firstChild) {
        exitBlock(quote); // nothing to split off; may reuse the trailing paragraph
      } else {
        // A real split: the paragraph must sit exactly between the two
        // halves, so it's always fresh, never reused from elsewhere.
        const p = emptyParagraph();
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

  return {
    CONVERTIBLE_TAGS, MERGEABLE_TAGS, convertListItemToParagraph, emptyParagraph, ensureBlockStructure,
    ensureTrailingParagraph, getAncestorTag, getBlockNode, getCurrentBlockTag, getEnclosingListItem,
    getRootBlock, getSelectionBlock, hasAncestorTag, indentListItem, indentQuotedBlock, insertRule,
    isEmptyBlock, mergeBlockIntoPrevious, normalizeInlineTags, outdentListItem, removeTrailingParagraph,
    replaceBlockTag, replaceBlockWithRule, strikeForCommand, tryExitBlockOnEnter, unwrapElement, unwrapFromQuote,
    wrapBlockAsCode, wrapBlockAsListItem, wrapBlockAsQuote,
  };
}
