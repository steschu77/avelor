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
import { t } from '../core/text.js';
import { createSelection } from './selection.js';
import { createBlocks } from './blocks.js';
import { createTables } from './tables.js';
import { createShortcuts } from './shortcuts.js';
import { createPaste } from './paste.js';
import { isSafeUrl } from './links.js';

// The commands with a key of their own. No Ctrl+Alt combinations -- see
// editor.md#toolbar.
export const EDITOR_KEYS = [
  { command: 'bold', shortcut: 'Ctrl+B' },
  { command: 'italic', shortcut: 'Ctrl+I' },
  { command: 'strike', shortcut: 'Ctrl+Shift+X' },
  { command: 'code', shortcut: 'Ctrl+E' },
  { command: 'link', shortcut: 'Ctrl+K' },
  { command: 'ul', shortcut: 'Ctrl+Shift+8' },
  { command: 'ol', shortcut: 'Ctrl+Shift+7' },
  { command: 'quote', shortcut: 'Ctrl+Shift+9' },
];

// The command of a key with Ctrl, if it has one; a digit is the one of its
// place in the number row, as Shift makes another character of it.
function commandOf(e) {
  const key = /^[a-z]$/i.test(e.key) ? e.key.toUpperCase() : e.code.replace(/^Digit/, '');
  const shortcut = ['Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', key].filter(Boolean).join('+');
  return EDITOR_KEYS.find(entry => entry.shortcut === shortcut)?.command;
}

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

  // The editor's parts (editor.md#mechanisms), made for this editor.
  const {
    captureSelection, getSelectionRange, isCaretAtEndOf, isCaretAtStartOf, maxOffset, placeCaretAfterElement,
    placeCaretAtStart, restoreSelection, selectRange, selectionInside,
  } = createSelection(editor);
  const {
    CONVERTIBLE_TAGS, MERGEABLE_TAGS, convertListItemToParagraph, emptyParagraph, ensureBlockStructure,
    ensureTrailingParagraph, getAncestorTag, getBlockNode, getCurrentBlockTag, getEnclosingListItem,
    getRootBlock, getSelectionBlock, hasAncestorTag, indentListItem, indentQuotedBlock, insertRule,
    isEmptyBlock, mergeBlockIntoPrevious, normalizeInlineTags, outdentListItem, removeTrailingParagraph,
    replaceBlockTag, replaceBlockWithRule, strikeForCommand, tryExitBlockOnEnter, unwrapElement, unwrapFromQuote,
    wrapBlockAsCode, wrapBlockAsListItem, wrapBlockAsQuote,
  } = createBlocks(editor, {
    captureSelection, getSelectionRange, isCaretAtEndOf, isEditable, maxOffset, placeCaretAfterElement,
    placeCaretAtStart, restoreSelection, selectRange, selectionInside,
  });
  const {
    deleteColumn, deleteRow, deleteTable, enterInTable, getEnclosingCell, insertColumnRight, insertRowBelow,
    insertTable, tabInTable,
  } = createTables(editor, {
    CONVERTIBLE_TAGS, emptyParagraph, getBlockNode, getRootBlock, getSelectionRange, isEmptyBlock,
    placeCaretAtStart, selectRange,
  });
  const {
    tryBlockAutoformat, tryCodeBlockAutoformat, tryInlineAutoformat, tryInlineCodeAutoformat,
    tryLinkAutoformat, tryRuleAutoformat,
  } = createShortcuts(editor, {
    CONVERTIBLE_TAGS, getBlockNode, getSelectionBlock, getSelectionRange, hasAncestorTag, markJustClosed,
    placeCaretAfterElement, placeCaretAtStart, replaceBlockTag, replaceBlockWithRule, wrapBlockAsCode,
    wrapBlockAsListItem, wrapBlockAsQuote,
  });
  const {
    insertPastedContent,
  } = createPaste(editor, { afterEdit, getBlockNode, getSelectionRange, isEmptyBlock, placeCaretAfterElement });

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
    selectRange(r);
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
    selectRange(r);
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
    const href = window.prompt(t('editor.link_url'));
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
    const src = window.prompt(t('editor.image_url'));
    if (!src || !isSafeUrl(src)) return;
    const img = document.createElement('img');
    img.setAttribute('src', src);
    img.setAttribute('alt', '');
    range.deleteContents();
    range.insertNode(img);
    placeCaretAfterElement(img);
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
      case 'strike':
        strikeForCommand();
        document.execCommand('strikeThrough', false, null);
        break;
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
      // Tab acts in a table, a list, a quote and code; anywhere else it is
      // the browser's, on to the next control (editor.md#keys).
      const range = getSelectionRange();
      if (!range) return;
      const cell = getEnclosingCell(range.startContainer);
      const li = getEnclosingListItem(range.startContainer);
      const block = getBlockNode(range.startContainer);
      if (cell) {
        e.preventDefault();
        tabInTable(cell, e.shiftKey);
        afterEdit();
      } else if (li) {
        e.preventDefault();
        if (e.shiftKey) outdentListItem(li); else indentListItem(li);
        afterEdit();
      } else if (block?.parentNode.tagName === 'BLOCKQUOTE') {
        e.preventDefault();
        if (e.shiftKey) unwrapFromQuote(block); else indentQuotedBlock(block);
        afterEdit();
      } else if (hasAncestorTag(range.startContainer, 'PRE') && !e.shiftKey) {
        e.preventDefault();
        document.execCommand('insertText', false, '\t');
      }
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      const cmd = commandOf(e);
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
  // The toolbar is the mouse's: the keyboard formats by its keys and by
  // typing (editor.md#toolbar), so Tab passes it by.
  for (const btn of toolbar?.querySelectorAll('button') ?? []) btn.tabIndex = -1;

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
