// The markdown selftest (markdown.md#testing): `serializeMarkdown` over
// the editor's HTML vocabulary, each case's result compared with the
// markdown expected; markdown.selftest.html shows the table. build.rs
// bundles the two into the build's OUT_DIR, where the page opens from
// disk (development.md#the-bundled-page).

import { serializeMarkdown } from './markdown.js';

// [name, editor HTML, expected markdown]
const CASES = [
  // --- the editor's mapping table ---
  ['paragraph', '<p>text</p>', 'text\n'],
  ['paragraphs', '<p>a</p><p>b</p>', 'a\n\nb\n'],
  ['strong', '<p><strong>text</strong></p>', '**text**\n'],
  ['em', '<p><em>text</em></p>', '*text*\n'],
  ['strong em', '<p><strong><em>text</em></strong></p>', '***text***\n'],
  ['inline code', '<p><code>text</code></p>', '`text`\n'],
  ['h1', '<h1>text</h1>', '# text\n'],
  ['h4', '<h4>text</h4>', '#### text\n'],
  ['bullet list', '<ul><li>a</li><li>b</li></ul>', '- a\n- b\n'],
  ['numbered list with start', '<ol start="3"><li>a</li><li>b</li></ol>', '3. a\n4. b\n'],
  ['nested bullet list', '<ul><li>a<ul><li>b</li></ul></li></ul>', '- a\n  - b\n'],
  ['nested numbered list', '<ol><li>a<ol><li>b</li></ol></li></ol>', '1. a\n   1. b\n'],
  ['loose list', '<ul><li><p>a</p></li><li><p>b</p></li></ul>', '- a\n\n- b\n'],
  ['loose item with two paragraphs', '<ul><li><p>a</p><p>b</p></li><li><p>c</p></li></ul>', '- a\n\n  b\n\n- c\n'],
  ['task list',
    '<ul><li><input type="checkbox" checked disabled>done</li><li><input type="checkbox" disabled>open</li></ul>',
    '- [x] done\n- [ ] open\n'],
  ['blockquote paragraphs', '<blockquote><p>a</p><p>b</p></blockquote>', '> a\n>\n> b\n'],
  ['nested blockquote', '<blockquote><p>a</p><blockquote><p>b</p></blockquote></blockquote>', '> a\n>\n> > b\n'],
  ['heading inside quote', '<blockquote><h2>title</h2></blockquote>', '> ## title\n'],
  ['code block', '<pre>line one<br>line two</pre>', '```\nline one\nline two\n```\n'],
  ['code block with language', '<pre data-lang="rust">fn x() {}</pre>', '```rust\nfn x() {}\n```\n'],
  ['empty code block placeholder', '<pre><br></pre>', '```\n```\n'],
  ['empty code block', '<pre></pre>', '```\n```\n'],
  ['code block trailing break', '<pre>line<br></pre>', '```\nline\n```\n'],
  ['code content is verbatim', '<pre>**not bold** &lt;b&gt;</pre>', '```\n**not bold** <b>\n```\n'],
  ['rule', '<p>a</p><hr><p>b</p>', 'a\n\n---\n\nb\n'],
  ['strikethrough', '<p><del>x</del></p>', '~~x~~\n'],
  ['link with title', '<p><a href="http://x" title="T">t</a></p>', '[t](http://x "T")\n'],
  ['link', '<p><a href="http://x">t</a></p>', '[t](http://x)\n'],
  ['image', '<p><img src="img.png" alt="alt text" title="T"></p>', '![alt text](img.png "T")\n'],
  ['table',
    '<table><thead><tr><th>a</th><th align="center">b</th><th align="right">c</th></tr></thead>' +
    '<tbody><tr><td>1</td><td align="center">2</td><td align="right">3</td></tr></tbody></table>',
    '| a | b | c |\n| --- | :---: | ---: |\n| 1 | 2 | 3 |\n'],
  ['table cell escapes pipe',
    '<table><thead><tr><th>a|b</th></tr></thead><tbody></tbody></table>',
    '| a\\|b |\n| --- |\n'],
  ['hard break', '<p>a<br>b</p>', 'a  \nb\n'],
  ['empty input', '', ''],

  // --- escaping literal text on the way out ---
  ['escapes inline marker characters everywhere',
    '<p>a * b _ c ~ d ` e [ f ] g \\ h &lt; i</p>',
    'a \\* b \\_ c \\~ d \\` e \\[ f \\] g \\\\ h \\< i\n'],
  ['escapes leading # ', '<p># not a heading</p>', '\\# not a heading\n'],
  ['escapes leading -', '<p>- not a list</p>', '\\- not a list\n'],
  ['escapes leading >', '<p>&gt; not a quote</p>', '\\> not a quote\n'],
  ['escapes leading N.', '<p>1. not a list</p>', '1\\. not a list\n'],
  ['escapes leading marker after hard break', '<p>a<br>- b</p>', 'a  \n\\- b\n'],
  ['escapes leading marker in list item', '<ul><li>- a</li></ul>', '- \\- a\n'],
  ['no escaping mid-sentence', '<p>well-known, 5 &gt; 3, a#b</p>', 'well-known, 5 > 3, a#b\n'],
  ['already-escaped source survives a no-op edit',
    '<p>1. not a list, a * b, well-known</p>',
    '1\\. not a list, a \\* b, well-known\n'],
  ['nbsp becomes a plain space', '<p>a\u00a0b</p>', 'a b\n'],
  ['nbsp entity becomes a plain space', '<p>a&nbsp;b</p>', 'a b\n'],
  ['raw html text is not resurrected',
    '<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>',
    '\\<script>alert(1)\\</script>\n'],

  // --- unrecognized tags are unwrapped, not dropped ---
  ['unwraps styled span', '<p><span style="color:red">text abc</span></p>', 'text abc\n'],
  ['unwraps span around markup', '<p><span><strong>bold</strong></span></p>', '**bold**\n'],
  ['unwraps div around blocks', '<div><p>a</p><p>b</p></div>', 'a\n\nb\n'],
  ['unwraps nested unknown wrappers', '<p><span><font>text</font></span></p>', 'text\n'],

  // --- editing-time placeholders ---
  ['trailing placeholder paragraph is dropped', '<p>a</p><p><br></p>', 'a\n'],
  ['lone placeholder paragraph is an empty document', '<p><br></p>', ''],
  ['placeholder paragraph mid-document is dropped', '<p>a</p><p><br></p><p>b</p>', 'a\n\nb\n'],
  ['trailing br in a paragraph is dropped', '<p>Hello<br></p>', 'Hello\n'],
  ['empty heading keeps its level', '<h2><br></h2>', '##\n'],
  ['root-level text is ignored', 'stray<p>a</p>', 'a\n'],
];

const container = document.createElement('div');
const results = document.getElementById('results');
let failed = 0;

for (const [name, html, expected] of CASES) {
  container.innerHTML = html;
  let actual;
  try {
    actual = serializeMarkdown(container);
  } catch (err) {
    actual = 'THREW: ' + err;
  }
  const ok = actual === expected;
  if (!ok) failed++;
  const tr = document.createElement('tr');
  tr.className = ok ? 'pass' : 'fail';
  for (const text of [ok ? 'pass' : 'FAIL', name, html, JSON.stringify(expected), JSON.stringify(actual)]) {
    const td = document.createElement('td');
    const pre = document.createElement('pre');
    pre.textContent = text;
    td.appendChild(pre);
    tr.appendChild(td);
  }
  results.appendChild(tr);
}

const summary = document.getElementById('summary');
summary.textContent = `${CASES.length - failed} passed, ${failed} failed`;
summary.className = failed ? 'fail' : 'pass';
document.title = `markdown.js selftest: ${failed ? failed + ' FAILED' : 'all passed'}`;
document.body.dataset.failed = String(failed);
