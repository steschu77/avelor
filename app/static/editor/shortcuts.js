// Typed markdown shortcuts (editor.md#shortcuts): a block's prefix, a code
// block's fence, a rule, and the inline code, emphasis and link shortcuts.

import { isSafeUrl } from './links.js';

export function createShortcuts(editor, {
  CONVERTIBLE_TAGS, getBlockNode, getSelectionBlock, getSelectionRange, hasAncestorTag, markJustClosed,
  placeCaretAfterElement, placeCaretAtStart, replaceBlockTag, replaceBlockWithRule, wrapBlockAsCode,
  wrapBlockAsListItem, wrapBlockAsQuote,
}) {
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

  // Where an inline shortcut may start: a collapsed caret in a text node,
  // with the text before it. Delimiters typed inside a code block or a code
  // span (and inside `outside`'s tags) are literal characters, so none
  // starts there.
  function inlineShortcutStart(...outside) {
    const range = getSelectionRange();
    if (!range || !range.collapsed) return null;
    const node = range.startContainer;
    if (node.nodeType !== Node.TEXT_NODE) return null;
    if (['PRE', 'CODE', ...outside].some(tag => hasAncestorTag(node, tag))) return null;
    return { range, node, text: node.textContent.slice(0, range.startOffset) };
  }

  function tryInlineCodeAutoformat() {
    const start = inlineShortcutStart();
    if (!start) return false;
    const { range, node, text } = start;
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
    const start = inlineShortcutStart();
    if (!start) return false;
    const { range, node, text } = start;
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
    const start = inlineShortcutStart('A');
    if (!start) return false;
    const { range, node, text } = start;
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

  return {
    tryBlockAutoformat, tryCodeBlockAutoformat, tryInlineAutoformat, tryInlineCodeAutoformat,
    tryLinkAutoformat, tryRuleAutoformat,
  };
}
