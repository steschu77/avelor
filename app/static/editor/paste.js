// Paste and drop (editor.md#paste): the HTML taken in reduced to the editor's
// own tags and inserted.

import { CONTENT_TAGS, ROOT_BLOCK_TAGS } from './tags.js';
import { isSafeUrl } from './links.js';

export function createPaste(editor, {
  afterEdit, getBlockNode, getSelectionRange, isEmptyBlock, placeCaretAfterElement,
}) {
  // ---------------------------------------------------------------------------
  // Paste / drop sanitization -- see editor.md#paste.
  const PASTE_ATTRIBUTES = {
    OL: ['start'], TH: ['align'], TD: ['align'], A: ['href', 'title'], IMG: ['src', 'alt', 'title'],
  };
  const ALLOWED_PASTE_TAGS = new Map(CONTENT_TAGS.map(tag => [tag, PASTE_ATTRIBUTES[tag] ?? []]));
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

  return { insertPastedContent };
}
