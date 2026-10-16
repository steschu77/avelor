// DOM -> markdown serializer for the WYSIWYG editor's own HTML vocabulary.
// See markdown.md#dom-contract for the DOM shape and
// mapping table this implements, markdown.md#serializing for the
// serializer's rules (unknown tags unwrapped, placeholders dropped,
// escaping), markdown.md#testing for its test page
// (markdown.selftest.html) and how to add a construct, and
// markdown.md#overview for why this half of the conversion lives in
// the webview while markdown -> HTML (`av_vault::markdown::to_html`) stays
// in Rust.
//
//   const markdown = serializeMarkdown(rootElement);

const MD_KNOWN_TAGS = new Set([
  'P', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'UL', 'OL', 'LI', 'BLOCKQUOTE', 'PRE', 'HR',
  'TABLE', 'THEAD', 'TBODY', 'TR', 'TH', 'TD', 'STRONG', 'EM', 'DEL', 'CODE', 'A', 'IMG',
  'BR', 'INPUT',
]);

const MD_BLOCK_TAGS = new Set([
  'P', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'UL', 'OL', 'BLOCKQUOTE', 'PRE', 'HR', 'TABLE',
]);

// Render the editor root's content as a markdown body: blocks separated by
// one blank line, one trailing newline, or the empty string for no content.
export function serializeMarkdown(root) {
  const blocks = [];
  for (const node of mdChildren(root)) {
    if (!mdIsElement(node) || !MD_BLOCK_TAGS.has(node.tagName)) continue;
    const block = mdRenderBlock(node);
    if (block !== null) blocks.push(block);
  }
  const text = blocks.join('\n\n').replace(/\n+$/, '');
  return text === '' ? '' : text + '\n';
}

// ---------------------------------------------------------------------------
// Tree access. Unknown elements are unwrapped here, once, so every renderer
// below only ever sees known tags and text.

function mdIsElement(node) {
  return node.nodeType === Node.ELEMENT_NODE;
}

function mdIsTag(node, tagName) {
  return mdIsElement(node) && node.tagName === tagName;
}

function mdChildren(node) {
  const out = [];
  for (const child of node.childNodes) {
    if (child.nodeType === Node.TEXT_NODE) {
      out.push(child);
    } else if (mdIsElement(child)) {
      if (MD_KNOWN_TAGS.has(child.tagName)) out.push(child);
      else out.push(...mdChildren(child));
    }
    // Comments and anything else are dropped.
  }
  return out;
}

// A literal U+00A0 is the editing-time artifact `contenteditable` leaves
// behind for a typed trailing/leading space (editor.md#shortcuts); markdown
// has no reason to know about it.
function mdText(node) {
  return node.data.replace(/\u00a0/g, ' ');
}

function mdPlainText(node) {
  return node.textContent.replace(/\u00a0/g, ' ');
}

// ---------------------------------------------------------------------------
// Blocks. Each returns the block's markdown without a trailing newline, or
// null for a block that has no markdown form and should be skipped.

function mdRenderBlock(el) {
  switch (el.tagName) {
    case 'P': {
      const text = mdRenderInlineBlock(el);
      return text === '' ? null : mdEscapeLeadingMarkers(text);
    }
    case 'H1': case 'H2': case 'H3': case 'H4': case 'H5': case 'H6': {
      const text = mdRenderInlineBlock(el);
      const hashes = '#'.repeat(Number(el.tagName[1]));
      return text === '' ? hashes : hashes + ' ' + text;
    }
    case 'BLOCKQUOTE': return mdRenderBlockquote(el);
    case 'UL': return mdRenderList(el, false);
    case 'OL': return mdRenderList(el, true);
    case 'PRE': return mdRenderCodeBlock(el);
    case 'HR': return '---';
    case 'TABLE': return mdRenderTable(el);
    default: return null;
  }
}

// Each child block on its own line(s), then every line prefixed with `> `
// (an empty line becomes a bare `>`). A nested quote's own prefix is
// prefixed again here, giving `> > text` -- recursion falls out naturally.
function mdRenderBlockquote(el) {
  const inner = [];
  for (const child of mdChildren(el)) {
    if (!mdIsElement(child)) continue;
    const block = mdRenderBlock(child);
    if (block !== null) inner.push(block);
  }
  if (inner.length === 0) return '>';
  return inner.join('\n\n').split('\n').map(line => (line === '' ? '>' : '> ' + line)).join('\n');
}

function mdRenderList(el, ordered) {
  const parsedStart = parseInt(el.getAttribute('start') ?? '', 10);
  let num = Number.isNaN(parsedStart) ? 1 : parsedStart;
  const items = mdChildren(el).filter(n => mdIsTag(n, 'LI'));
  // A "loose" list (blank line between items, each wrapped in its own <p>)
  // is preserved as loose; a "tight" list stays tight.
  const loose = items.some(item => mdChildren(item).some(n => mdIsTag(n, 'P')));

  const lines = [];
  for (const item of items) {
    const marker = ordered ? `${num++}. ` : '- ';
    lines.push(mdRenderListItem(item, marker));
  }
  return lines.join(loose ? '\n\n' : '\n');
}

function mdRenderListItem(item, marker) {
  const children = mdChildren(item);
  const indent = ' '.repeat(marker.length);
  let out = marker;

  let idx = 0;
  if (children.length > 0 && mdIsTag(children[0], 'INPUT')) {
    out += children[0].hasAttribute('checked') ? '[x] ' : '[ ] ';
    idx = 1;
  }

  // Tab (`indentListItem`) always appends a nested list as the item's last
  // child, after whatever content came before it.
  let nestedStart = children.findIndex((n, i) => i >= idx && (mdIsTag(n, 'UL') || mdIsTag(n, 'OL')));
  if (nestedStart < 0) nestedStart = children.length;
  const content = children.slice(idx, nestedStart);
  const nested = children.slice(nestedStart);

  // A loose item's content is one or more <p>; a tight item's is bare
  // inline nodes. Either way each paragraph's continuation lines are
  // indented to the marker width so they stay inside the item.
  const paragraphs = [];
  let run = [];
  const flushRun = () => {
    if (run.length === 0) return;
    const text = mdRenderInlineNodes(run);
    if (text !== '') paragraphs.push(text);
    run = [];
  };
  for (const node of content) {
    if (mdIsTag(node, 'P')) {
      flushRun();
      const text = mdRenderInlineBlock(node);
      if (text !== '') paragraphs.push(text);
    } else {
      run.push(node);
    }
  }
  flushRun();
  out += paragraphs
    .map(p => mdEscapeLeadingMarkers(p).split('\n').join('\n' + indent))
    .join('\n\n' + indent);

  for (const list of nested) {
    for (const line of mdRenderList(list, list.tagName === 'OL').split('\n')) {
      out += '\n' + (line === '' ? '' : indent + line);
    }
  }
  return out;
}

function mdRenderCodeBlock(el) {
  const lang = el.getAttribute('data-lang') ?? '';
  const children = mdChildren(el);
  let body = '';
  // `<pre><br></pre>` is the empty-block placeholder, not one blank line.
  if (!(children.length === 1 && mdIsTag(children[0], 'BR'))) {
    body = mdCodeText(children);
    if (body !== '' && !body.endsWith('\n')) body += '\n';
  }
  return '```' + lang + '\n' + body + '```';
}

// Code content is literal -- no markdown escaping, matching how `to_html`
// writes it on the way in.
function mdCodeText(nodes) {
  let out = '';
  for (const node of nodes) {
    if (!mdIsElement(node)) out += mdText(node);
    else if (node.tagName === 'BR') out += '\n';
    else out += mdCodeText(mdChildren(node));
  }
  return out;
}

function mdRenderTable(table) {
  const thead = mdChildren(table).find(n => mdIsTag(n, 'THEAD'));
  const headerRow = thead && mdChildren(thead).find(n => mdIsTag(n, 'TR'));
  if (!headerRow) return null;
  const headerCells = mdChildren(headerRow).filter(n => mdIsTag(n, 'TH'));

  const lines = [mdRenderTableRow(headerCells)];
  lines.push('|' + headerCells.map(cell => {
    switch (cell.getAttribute('align')) {
      case 'left': return ' :--- |';
      case 'center': return ' :---: |';
      case 'right': return ' ---: |';
      default: return ' --- |';
    }
  }).join(''));

  const tbody = mdChildren(table).find(n => mdIsTag(n, 'TBODY'));
  if (tbody) {
    for (const row of mdChildren(tbody).filter(n => mdIsTag(n, 'TR'))) {
      lines.push(mdRenderTableRow(mdChildren(row).filter(n => mdIsTag(n, 'TD'))));
    }
  }
  return lines.join('\n');
}

function mdRenderTableRow(cells) {
  return '|' + cells.map(cell => ' ' + mdRenderInlineBlock(cell).replace(/\|/g, '\\|') + ' |').join('');
}

// ---------------------------------------------------------------------------
// Inline content.

// The inline content of a leaf block. A trailing <br> is a placeholder
// (or a hard break with nothing after it, which is the same thing on
// screen), never a line break to keep.
function mdRenderInlineBlock(el) {
  const nodes = mdChildren(el);
  while (nodes.length > 0 && mdIsTag(nodes[nodes.length - 1], 'BR')) nodes.pop();
  return mdRenderInlineNodes(nodes);
}

function mdRenderInlineNodes(nodes) {
  return nodes.map(mdRenderInline).join('');
}

function mdRenderInline(node) {
  if (!mdIsElement(node)) return mdEscapeText(mdText(node));
  switch (node.tagName) {
    case 'STRONG': return '**' + mdRenderInlineNodes(mdChildren(node)) + '**';
    case 'EM': return '*' + mdRenderInlineNodes(mdChildren(node)) + '*';
    case 'DEL': return '~~' + mdRenderInlineNodes(mdChildren(node)) + '~~';
    case 'CODE': return '`' + mdPlainText(node) + '`';
    case 'A':
      return '[' + mdRenderInlineNodes(mdChildren(node)) + '](' +
        (node.getAttribute('href') ?? '') + mdOptionalTitle(node) + ')';
    case 'IMG':
      return '![' + (node.getAttribute('alt') ?? '') + '](' +
        (node.getAttribute('src') ?? '') + mdOptionalTitle(node) + ')';
    case 'BR': return '  \n';
    // A block-level or structural tag where only inline content belongs;
    // keep its text rather than dropping it.
    default: return mdRenderInlineNodes(mdChildren(node));
  }
}

function mdOptionalTitle(el) {
  const title = el.getAttribute('title');
  return title ? ` "${title}"` : '';
}

// ---------------------------------------------------------------------------
// Escaping.

// Exactly the characters that could be misread as markdown syntax if they
// came from literal text -- unconditionally, regardless of position. Never
// applied inside <pre>/<code>, whose content is written verbatim.
function mdEscapeText(text) {
  return text.replace(/[\\`*_~[\]<]/g, '\\$&');
}

// A line's rendered text needs `#`/`-`/`+`/`>` or a leading `N.`/`N)`
// escaped only when it is genuinely the first thing on that markdown line
// -- and only those, since every other marker-capable character is already
// escaped by `mdEscapeText` wherever it came from literal text, so a leading
// one of *those* in the rendered output can only be real syntax this
// serializer inserted (e.g. `**bold**`), never a leftover literal.
function mdEscapeLeadingMarkers(text) {
  return text.split('\n').map(mdEscapeLeadingMarker).join('\n');
}

function mdEscapeLeadingMarker(line) {
  if (/^[#>+-]/.test(line)) return '\\' + line;
  return line.replace(/^(\d+)([.)])/, '$1\\$2');
}
