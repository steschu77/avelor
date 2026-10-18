//! Markdown -> HTML rendering for the WYSIWYG editor, and a markdown line
//! as a plain title ([`plain_title`]).
//!
//! [`to_html`] renders a markdown body into the exact DOM shape that
//! `app/static/editor/editor.js` edits — no whitespace between blocks, bare
//! `<pre>` with `<br>` line breaks, `<strong>`/`<em>`, attributes only
//! where the round trip needs them; raw HTML escaped to visible text. The
//! contract, the mapping table, and the rules of the reverse direction
//! (the webview's `app/static/editor/markdown.js`, `serializeMarkdown`)
//! are in `markdown.md`; the two test tables — this module's and
//! `app/static/editor/markdown.selftest.js` — mirror each other
//! (`markdown.md#testing`), so anything added to the writer below needs its
//! counterpart in both.

use std::fmt::Write as _;

use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

// ----------------------------------------------------------------------------
/// Render a markdown body (frontmatter already stripped) to editor HTML.
pub fn to_html(markdown: &str) -> String {
    let mut writer = Writer::default();
    for event in Parser::new_ext(markdown, options()) {
        writer.event(event);
    }
    writer.out
}

// ----------------------------------------------------------------------------
/// How the parser reads one line of inline markdown: its text, and how many
/// spans of emphasis, strong emphasis, and strikethrough it finds. The HTML
/// conversion checks the delimiters it writes with it (`md_writer`).
pub(crate) fn read_inline(line: &str) -> (String, usize) {
    let mut text = String::new();
    let mut spans = 0;
    for event in Parser::new_ext(line, options()) {
        match event {
            Event::Text(t) => text.push_str(&t),
            Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough) => spans += 1,
            _ => {}
        }
    }
    (text, spans)
}

// ----------------------------------------------------------------------------
/// The extensions the vault's markdown uses: tables, strikethrough, task
/// lists.
fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS
}

// ----------------------------------------------------------------------------
#[derive(Default)]
struct Writer {
    out: String,
    /// Raw text of the code block or HTML block being collected; emitted as
    /// one escaped run with `<br>` line breaks when the block ends.
    block_text: Option<String>,
    /// The image being collected: (src, title, alt-so-far). Alt text arrives
    /// as ordinary events between `Start(Image)` and `End(Image)`.
    image: Option<(String, String, String)>,
    in_table_head: bool,
    table_alignments: Vec<Alignment>,
    table_col: usize,
}

impl Writer {
    // ------------------------------------------------------------------------
    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => {
                self.out.push_str("<code>");
                escape_into(&mut self.out, &code);
                self.out.push_str("</code>");
            }
            // Raw HTML becomes visible text (see module docs).
            Event::Html(html) | Event::InlineHtml(html) => self.text(&html),
            Event::SoftBreak => self.text("\n"),
            Event::HardBreak => self.out.push_str("<br>"),
            Event::Rule => self.out.push_str("<hr>"),
            Event::TaskListMarker(checked) => self.out.push_str(if checked {
                r#"<input type="checkbox" checked disabled>"#
            } else {
                r#"<input type="checkbox" disabled>"#
            }),
            // Not enabled in `to_html`; degrade to their source form.
            Event::FootnoteReference(name) => self.text(&format!("[^{name}]")),
            Event::InlineMath(math) => self.text(&format!("${math}$")),
            Event::DisplayMath(math) => self.text(&format!("$${math}$$")),
        }
    }

    // ------------------------------------------------------------------------
    /// Text content: collected while inside a code/HTML block or an image,
    /// escaped straight into the output otherwise.
    fn text(&mut self, text: &str) {
        if let Some(buf) = &mut self.block_text {
            buf.push_str(text);
        } else if let Some((_, _, alt)) = &mut self.image {
            alt.push_str(text);
        } else {
            escape_into(&mut self.out, text);
        }
    }

    // ------------------------------------------------------------------------
    fn start(&mut self, tag: Tag) {
        // Inside an image only the text matters (it becomes `alt`).
        if self.image.is_some() {
            return;
        }
        match tag {
            Tag::Paragraph => self.out.push_str("<p>"),
            Tag::Heading { level, .. } => {
                self.out.push('<');
                self.out.push_str(heading_tag(level));
                self.out.push('>');
            }
            Tag::BlockQuote(_) => self.out.push_str("<blockquote>"),
            Tag::CodeBlock(kind) => {
                self.out.push_str("<pre");
                if let CodeBlockKind::Fenced(info) = kind {
                    let lang = info.split_whitespace().next().unwrap_or("");
                    if !lang.is_empty() {
                        self.attr("data-lang", lang);
                    }
                }
                self.out.push('>');
                self.block_text = Some(String::new());
            }
            Tag::HtmlBlock => {
                self.out.push_str("<p>");
                self.block_text = Some(String::new());
            }
            Tag::List(None) => self.out.push_str("<ul>"),
            Tag::List(Some(1)) => self.out.push_str("<ol>"),
            Tag::List(Some(start)) => {
                let _ = write!(self.out, "<ol start=\"{start}\">");
            }
            Tag::Item => self.out.push_str("<li>"),
            Tag::Table(alignments) => {
                self.table_alignments = alignments;
                self.out.push_str("<table>");
            }
            Tag::TableHead => {
                self.in_table_head = true;
                self.table_col = 0;
                self.out.push_str("<thead><tr>");
            }
            Tag::TableRow => {
                self.table_col = 0;
                self.out.push_str("<tr>");
            }
            Tag::TableCell => {
                self.out.push('<');
                self.out.push_str(self.cell_tag());
                let align = match self.table_alignments.get(self.table_col) {
                    Some(Alignment::Left) => Some("left"),
                    Some(Alignment::Center) => Some("center"),
                    Some(Alignment::Right) => Some("right"),
                    Some(Alignment::None) | None => None,
                };
                if let Some(align) = align {
                    self.attr("align", align);
                }
                self.out.push('>');
                self.table_col += 1;
            }
            Tag::Emphasis => self.out.push_str("<em>"),
            Tag::Strong => self.out.push_str("<strong>"),
            Tag::Strikethrough => self.out.push_str("<del>"),
            Tag::Superscript => self.out.push_str("<sup>"),
            Tag::Subscript => self.out.push_str("<sub>"),
            Tag::Link {
                dest_url, title, ..
            } => {
                self.out.push_str("<a");
                self.attr("href", &dest_url);
                if !title.is_empty() {
                    self.attr("title", &title);
                }
                self.out.push('>');
            }
            Tag::Image {
                dest_url, title, ..
            } => {
                self.image = Some((dest_url.to_string(), title.to_string(), String::new()));
            }
            // Not enabled in `to_html`.
            Tag::FootnoteDefinition(_)
            | Tag::MetadataBlock(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition => {}
        }
    }

    // ------------------------------------------------------------------------
    fn end(&mut self, tag: TagEnd) {
        if self.image.is_some() && tag != TagEnd::Image {
            return;
        }
        match tag {
            TagEnd::Paragraph => self.out.push_str("</p>"),
            TagEnd::Heading(level) => {
                self.out.push_str("</");
                self.out.push_str(heading_tag(level));
                self.out.push('>');
            }
            TagEnd::BlockQuote(_) => self.out.push_str("</blockquote>"),
            TagEnd::CodeBlock => {
                self.flush_block_text();
                self.out.push_str("</pre>");
            }
            TagEnd::HtmlBlock => {
                self.flush_block_text();
                self.out.push_str("</p>");
            }
            TagEnd::List(ordered) => self.out.push_str(if ordered { "</ol>" } else { "</ul>" }),
            TagEnd::Item => self.out.push_str("</li>"),
            TagEnd::Table => self.out.push_str("</tbody></table>"),
            TagEnd::TableHead => {
                self.in_table_head = false;
                self.out.push_str("</tr></thead><tbody>");
            }
            TagEnd::TableRow => self.out.push_str("</tr>"),
            TagEnd::TableCell => {
                self.out.push_str("</");
                self.out.push_str(self.cell_tag());
                self.out.push('>');
            }
            TagEnd::Emphasis => self.out.push_str("</em>"),
            TagEnd::Strong => self.out.push_str("</strong>"),
            TagEnd::Strikethrough => self.out.push_str("</del>"),
            TagEnd::Superscript => self.out.push_str("</sup>"),
            TagEnd::Subscript => self.out.push_str("</sub>"),
            TagEnd::Link => self.out.push_str("</a>"),
            TagEnd::Image => {
                if let Some((src, title, alt)) = self.image.take() {
                    self.out.push_str("<img");
                    self.attr("src", &src);
                    self.attr("alt", &alt);
                    if !title.is_empty() {
                        self.attr("title", &title);
                    }
                    self.out.push('>');
                }
            }
            TagEnd::FootnoteDefinition
            | TagEnd::MetadataBlock(_)
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition => {}
        }
    }

    // ------------------------------------------------------------------------
    /// Emit the collected code/HTML block text: one trailing newline dropped
    /// (the fence's own line end), inner newlines as `<br>`, and a lone
    /// `<br>` for an empty block so contenteditable can place a caret in it.
    fn flush_block_text(&mut self) {
        let text = self.block_text.take().unwrap_or_default();
        let text = text.strip_suffix('\n').unwrap_or(&text);
        if text.is_empty() {
            self.out.push_str("<br>");
            return;
        }
        let mut escaped = String::with_capacity(text.len());
        escape_into(&mut escaped, text);
        self.out.push_str(&escaped.replace('\n', "<br>"));
    }

    // ------------------------------------------------------------------------
    fn cell_tag(&self) -> &'static str {
        if self.in_table_head { "th" } else { "td" }
    }

    // ------------------------------------------------------------------------
    fn attr(&mut self, name: &str, value: &str) {
        self.out.push(' ');
        self.out.push_str(name);
        self.out.push_str("=\"");
        escape_into(&mut self.out, value);
        self.out.push('"');
    }
}

// ----------------------------------------------------------------------------
fn heading_tag(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "h1",
        HeadingLevel::H2 => "h2",
        HeadingLevel::H3 => "h3",
        HeadingLevel::H4 => "h4",
        HeadingLevel::H5 => "h5",
        HeadingLevel::H6 => "h6",
    }
}

// ----------------------------------------------------------------------------
/// Escape text for both element content and double-quoted attribute values.
fn escape_into(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

// ----------------------------------------------------------------------------
/// A title made of the markdown line `line` (a chat's first line): without
/// its block marker (`#`, `>`, `-`, `1.`, …) and inline marks, cut at a word
/// to at most `max` characters and an ellipsis.
pub fn plain_title(line: &str, max: usize) -> String {
    let line = line
        .trim()
        .trim_start_matches(|c: char| "#>*-+ ".contains(c));
    let line = match line.split_once(". ") {
        Some((number, rest))
            if !number.is_empty() && number.chars().all(|c| c.is_ascii_digit()) =>
        {
            rest
        }
        _ => line,
    };
    let line: String = line
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '`'))
        .collect();
    let line = line.trim();
    if line.chars().count() <= max {
        return line.to_string();
    }
    let cut: String = line.chars().take(max).collect();
    let cut = match cut.rfind(' ') {
        Some(space) if space > max / 2 => &cut[..space],
        _ => &cut,
    };
    format!(
        "{}\u{2026}",
        cut.trim_end_matches([' ', ',', '.', ';', ':'])
    )
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // --- the editor's mapping table (mirrored in markdown.selftest.html) ---

    #[test]
    fn paragraphs() {
        assert_eq!(to_html("text"), "<p>text</p>");
        assert_eq!(to_html("a\n\nb\n"), "<p>a</p><p>b</p>");
    }

    #[test]
    fn inline_emphasis_and_code() {
        assert_eq!(to_html("**text**"), "<p><strong>text</strong></p>");
        assert_eq!(to_html("__text__"), "<p><strong>text</strong></p>");
        assert_eq!(to_html("*text*"), "<p><em>text</em></p>");
        assert_eq!(to_html("_text_"), "<p><em>text</em></p>");
        assert_eq!(to_html("`text`"), "<p><code>text</code></p>");
    }

    #[test]
    fn combined_emphasis() {
        // CommonMark (and pulldown-cmark) nest ***x*** as <em><strong>, the
        // editor produces <strong><em>. Both mean the same; the webview's
        // serializer writes either as ***text***.
        let html = to_html("***text***");
        assert!(
            html == "<p><em><strong>text</strong></em></p>"
                || html == "<p><strong><em>text</em></strong></p>",
            "got {html}"
        );
        assert_eq!(
            to_html("**_text_**"),
            "<p><strong><em>text</em></strong></p>"
        );
    }

    #[test]
    fn headings() {
        assert_eq!(to_html("# text"), "<h1>text</h1>");
        assert_eq!(to_html("## text"), "<h2>text</h2>");
        assert_eq!(to_html("### text"), "<h3>text</h3>");
        assert_eq!(to_html("#### text"), "<h4>text</h4>");
        assert_eq!(to_html("###### text"), "<h6>text</h6>");
        // An empty heading (what the serializer writes for `<h2><br></h2>`).
        assert_eq!(to_html("##"), "<h2></h2>");
    }

    #[test]
    fn bullet_list() {
        assert_eq!(to_html("- a\n- b"), "<ul><li>a</li><li>b</li></ul>");
        assert_eq!(to_html("* a\n* b"), "<ul><li>a</li><li>b</li></ul>");
    }

    #[test]
    fn nested_lists_indented_to_marker_width() {
        assert_eq!(
            to_html("- a\n  - b"),
            "<ul><li>a<ul><li>b</li></ul></li></ul>"
        );
        assert_eq!(
            to_html("1. a\n   1. b"),
            "<ol><li>a<ol><li>b</li></ol></li></ol>"
        );
    }

    #[test]
    fn blockquote_paragraphs() {
        assert_eq!(
            to_html("> a\n>\n> b"),
            "<blockquote><p>a</p><p>b</p></blockquote>"
        );
        assert_eq!(
            to_html("> a\n>\n> > b"),
            "<blockquote><p>a</p><blockquote><p>b</p></blockquote></blockquote>"
        );
        assert_eq!(
            to_html("> ## title"),
            "<blockquote><h2>title</h2></blockquote>"
        );
    }

    #[test]
    fn fenced_code_block_is_bare_pre_with_br() {
        assert_eq!(
            to_html("```\nline one\nline two\n```"),
            "<pre>line one<br>line two</pre>"
        );
        assert_eq!(
            to_html("```rust\nfn x() {}\n```"),
            "<pre data-lang=\"rust\">fn x() {}</pre>"
        );
        assert_eq!(to_html("```\n```"), "<pre><br></pre>");
        // No escaping of markdown inside the fence, but HTML is escaped.
        assert_eq!(
            to_html("```\n**not bold** <b>\n```"),
            "<pre>**not bold** &lt;b&gt;</pre>"
        );
    }

    // --- editor DOM invariants ---

    #[test]
    fn no_whitespace_between_blocks() {
        let html = to_html("# Title\n\ntext\n\n- a\n\n> q\n");
        assert_eq!(
            html,
            "<h1>Title</h1><p>text</p><ul><li>a</li></ul><blockquote><p>q</p></blockquote>"
        );
        assert!(!html.contains(">\n<"));
    }

    #[test]
    fn empty_input_renders_nothing() {
        assert_eq!(to_html(""), "");
        assert_eq!(to_html("\n\n"), "");
    }

    #[test]
    fn line_breaks() {
        assert_eq!(to_html("a  \nb"), "<p>a<br>b</p>");
        assert_eq!(to_html("a\\\nb"), "<p>a<br>b</p>");
        assert_eq!(to_html("a\nb"), "<p>a\nb</p>");
    }

    // --- safety ---

    #[test]
    fn text_is_escaped() {
        assert_eq!(
            to_html("a & b < c > d \"e\""),
            "<p>a &amp; b &lt; c &gt; d &quot;e&quot;</p>"
        );
    }

    #[test]
    fn raw_html_becomes_visible_text() {
        assert_eq!(
            to_html("x <b onclick=\"y\">bold</b> z"),
            "<p>x &lt;b onclick=&quot;y&quot;&gt;bold&lt;/b&gt; z</p>"
        );
        assert_eq!(
            to_html("<div>\nhi\n</div>\n"),
            "<p>&lt;div&gt;<br>hi<br>&lt;/div&gt;</p>"
        );
        assert_eq!(
            to_html("<script>alert(1)</script>"),
            "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>"
        );
    }

    // --- elements the editor can't edit, but must display and keep ---

    #[test]
    fn ordered_lists() {
        assert_eq!(to_html("1. a\n2. b"), "<ol><li>a</li><li>b</li></ol>");
        assert_eq!(
            to_html("3. a\n4. b"),
            "<ol start=\"3\"><li>a</li><li>b</li></ol>"
        );
    }

    #[test]
    fn task_list() {
        assert_eq!(
            to_html("- [x] done\n- [ ] open"),
            "<ul><li><input type=\"checkbox\" checked disabled>done</li>\
             <li><input type=\"checkbox\" disabled>open</li></ul>"
        );
    }

    #[test]
    fn links_and_images() {
        assert_eq!(
            to_html("[t](http://x/?a=1&b=2 \"T\")"),
            "<p><a href=\"http://x/?a=1&amp;b=2\" title=\"T\">t</a></p>"
        );
        assert_eq!(
            to_html("![alt *text*](img.png \"T\")"),
            "<p><img src=\"img.png\" alt=\"alt text\" title=\"T\"></p>"
        );
        assert_eq!(
            to_html("<http://x>"),
            "<p><a href=\"http://x\">http://x</a></p>"
        );
    }

    #[test]
    fn rule_and_strikethrough() {
        assert_eq!(to_html("a\n\n---\n\nb"), "<p>a</p><hr><p>b</p>");
        assert_eq!(to_html("~~x~~"), "<p><del>x</del></p>");
    }

    #[test]
    fn table() {
        assert_eq!(
            to_html("| a | b | c |\n|---|:-:|--:|\n| 1 | 2 | 3 |"),
            "<table><thead><tr><th>a</th><th align=\"center\">b</th><th align=\"right\">c</th></tr></thead>\
             <tbody><tr><td>1</td><td align=\"center\">2</td><td align=\"right\">3</td></tr></tbody></table>"
        );
        // The serializer's own separator row and pipe escaping read back.
        assert_eq!(
            to_html("| a\\|b |\n| --- |\n"),
            "<table><thead><tr><th>a|b</th></tr></thead><tbody></tbody></table>"
        );
    }

    #[test]
    fn loose_list_items_keep_paragraphs() {
        assert_eq!(
            to_html("- a\n\n- b"),
            "<ul><li><p>a</p></li><li><p>b</p></li></ul>"
        );
        assert_eq!(
            to_html("- a\n\n  b\n\n- c"),
            "<ul><li><p>a</p><p>b</p></li><li><p>c</p></li></ul>"
        );
    }

    // --- what the serializer's escaping must read back as ---

    #[test]
    fn escaped_literal_text_reads_back_as_plain_text() {
        // The webview's serializer escapes `\ ` * _ ~ [ ] <` everywhere and
        // a leading `# - + >` / `N.` at line start; every one of those must
        // come back as the literal character, not as markup.
        assert_eq!(
            to_html("a \\* b \\_ c \\~ d \\` e \\[ f \\] g \\\\ h \\< i"),
            "<p>a * b _ c ~ d ` e [ f ] g \\ h &lt; i</p>"
        );
        assert_eq!(to_html("\\# not a heading"), "<p># not a heading</p>");
        assert_eq!(to_html("\\- not a list"), "<p>- not a list</p>");
        assert_eq!(to_html("\\> not a quote"), "<p>&gt; not a quote</p>");
        assert_eq!(to_html("1\\. not a list"), "<p>1. not a list</p>");
        assert_eq!(to_html("a  \n\\- b"), "<p>a<br>- b</p>");
        assert_eq!(to_html("- \\- a"), "<ul><li>- a</li></ul>");
        assert_eq!(
            to_html("\\<script>alert(1)\\</script>"),
            "<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>"
        );
        // Mid-sentence, none of these are escaped, and none need to be.
        assert_eq!(
            to_html("well-known, 5 > 3, a#b"),
            "<p>well-known, 5 &gt; 3, a#b</p>"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_plain_title_drops_the_marks_and_is_cut_at_a_word() {
        assert_eq!(
            plain_title("## The *quarterly* report", 40),
            "The quarterly report"
        );
        assert_eq!(plain_title("> - 1. `Step` one", 40), "Step one");
        assert_eq!(plain_title("3.5 kg of flour", 40), "3.5 kg of flour");
        assert_eq!(
            plain_title("The quick brown fox jumps over the lazy dog", 20),
            "The quick brown fox\u{2026}"
        );
        assert_eq!(
            plain_title("Supercalifragilistic", 10),
            "Supercalif\u{2026}"
        );
    }
}
