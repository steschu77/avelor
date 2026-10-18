//! Canonical markdown from text: the paragraph / hard-break / quote / list /
//! emphasis subset the mail conversions produce, escaped and laid out
//! exactly like the editor's serializer writes it (markdown.md#serializing), so
//! an imported body isn't rewritten just for being opened.
//!
//! [`MdWriter`] collects lines; a blank line or a change of quote depth
//! separates blocks. Lines inside a block are joined by hard breaks, list
//! items by a newline. Emphasis delimiters are written right before the
//! next visible character, so they always hug the text, and closed at the
//! end of every line; a line whose delimiters the markdown parser wouldn't
//! read back as written is kept without them.

use crate::markdown::read_inline;

// ----------------------------------------------------------------------------
/// Plain text as markdown: every line kept (as a hard break), blank lines
/// separating paragraphs, and leading `>` runs as (nested) blockquotes.
pub fn plain_to_markdown(text: &str) -> String {
    let mut w = MdWriter::default();
    for line in text.lines() {
        let (depth, rest) = quote_depth(line);
        w.set_depth(depth);
        if rest.trim().is_empty() {
            w.paragraph();
        } else {
            w.text(rest);
            w.end_line();
        }
    }
    w.finish()
}

// ----------------------------------------------------------------------------
/// The number of leading `>` quote markers (each optionally preceded by
/// spaces) and the rest of the line.
fn quote_depth(line: &str) -> (usize, &str) {
    let mut depth = 0;
    let mut rest = line;
    while let Some(after) = rest.trim_start_matches(' ').strip_prefix('>') {
        depth += 1;
        rest = after;
    }
    (depth, rest)
}

// ----------------------------------------------------------------------------
/// A position in the line being written, taken by [`MdWriter::mark`].
#[derive(Clone, Copy)]
pub struct Mark {
    line: usize,
    at: usize,
    /// The same position in the line's plain copy.
    plain_at: usize,
}

// ----------------------------------------------------------------------------
/// An inline style with a markdown form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Strong,
    Emphasis,
    Strikethrough,
}

impl Style {
    // ------------------------------------------------------------------------
    fn delimiter(self) -> &'static str {
        match self {
            Style::Strong => "**",
            Style::Emphasis => "*",
            Style::Strikethrough => "~~",
        }
    }
}

// ----------------------------------------------------------------------------
struct Line {
    /// Escaped markdown; empty for a blank separator line.
    text: String,
    depth: usize,
    /// Spaces before the list marker, or before the text of a line without
    /// one.
    indent: usize,
    /// The list marker the line starts with (`- `, `3. `).
    marker: Option<String>,
}

// ----------------------------------------------------------------------------
/// An open list.
struct List {
    /// The next item's number; `None` for a bullet list.
    next: Option<u32>,
    /// The column of its markers.
    indent: usize,
    /// The column of its current item's text.
    content: usize,
}

// ----------------------------------------------------------------------------
#[derive(Default)]
pub struct MdWriter {
    lines: Vec<Line>,
    /// The current line, with its emphasis delimiters.
    cur: String,
    /// The current line without them: what is kept if they don't read back.
    plain: String,
    /// The list marker (and its column) for the next line with text.
    marker: Option<(String, usize)>,
    /// Quote depth of the current line.
    depth: usize,
    /// A collapsed space is due before the next text on this line.
    space: bool,
    /// The open lists, innermost last.
    lists: Vec<List>,
    /// The styles of the open elements, innermost last; one may repeat.
    styles: Vec<Style>,
    /// The delimiters open in `cur`, innermost last, with where each starts.
    written: Vec<(Style, usize)>,
    /// `written` may not match `styles` any more.
    stale: bool,
    /// The spans opened on the current line.
    spans: usize,
}

impl MdWriter {
    // ------------------------------------------------------------------------
    /// Append literal text: whitespace runs collapse to one space (none at
    /// the start of a line), zero-width characters vanish, and markdown
    /// syntax characters are escaped.
    pub fn text(&mut self, text: &str) {
        for c in text.chars() {
            if c.is_whitespace() {
                self.space = true;
            } else if !is_invisible(c) {
                if self.stale {
                    self.restyle();
                } else {
                    self.flush_space();
                }
                if matches!(c, '\\' | '`' | '*' | '_' | '~' | '[' | ']' | '<') {
                    self.push('\\');
                }
                self.push(c);
            }
        }
    }

    // ------------------------------------------------------------------------
    /// Ask for a space before the next text (e.g. between table cells).
    pub fn space(&mut self) {
        self.space = true;
    }

    // ------------------------------------------------------------------------
    fn flush_space(&mut self) {
        if self.space && !self.plain.is_empty() {
            self.push(' ');
        }
        self.space = false;
    }

    // ------------------------------------------------------------------------
    /// Append text to the line and its plain copy.
    fn push(&mut self, c: char) {
        self.cur.push(c);
        self.plain.push(c);
    }

    // ------------------------------------------------------------------------
    /// Write the following text in `style`, until [`MdWriter::close`].
    pub fn open(&mut self, style: Style) {
        self.styles.push(style);
        self.stale = true;
    }

    // ------------------------------------------------------------------------
    /// End the innermost open `style`; one that isn't open is ignored.
    pub fn close(&mut self, style: Style) {
        if let Some(i) = self.styles.iter().rposition(|&s| s == style) {
            self.styles.remove(i);
            self.stale = true;
        }
    }

    // ------------------------------------------------------------------------
    /// Before visible text: close the delimiters no open element wants any
    /// more (before the pending space), then open the ones wanted.
    fn restyle(&mut self) {
        let wanted = self.wanted();
        let kept = self.kept(&wanted);
        self.close_from(kept);
        self.flush_space();
        for &style in &wanted[kept..] {
            self.written.push((style, self.cur.len()));
            self.cur.push_str(style.delimiter());
            self.spans += 1;
        }
        self.stale = false;
    }

    // ------------------------------------------------------------------------
    /// The styles of the open elements, outermost first, each once.
    fn wanted(&self) -> Vec<Style> {
        let mut wanted = Vec::new();
        for &style in &self.styles {
            if !wanted.contains(&style) {
                wanted.push(style);
            }
        }
        wanted
    }

    // ------------------------------------------------------------------------
    /// How many of the written delimiters, from the outermost, are `wanted`.
    fn kept(&self, wanted: &[Style]) -> usize {
        let same = self.written.iter().zip(wanted);
        same.take_while(|((written, _), style)| written == *style)
            .count()
    }

    // ------------------------------------------------------------------------
    /// Close the written delimiters from `index` on, innermost first.
    fn close_from(&mut self, index: usize) {
        for (style, _) in self.written.drain(index..).rev() {
            self.cur.push_str(style.delimiter());
        }
    }

    // ------------------------------------------------------------------------
    /// The current position, for [`MdWriter::link`]. What no open element
    /// wants any more closes here, outside the link.
    pub fn mark(&mut self) -> Mark {
        let kept = self.kept(&self.wanted());
        self.close_from(kept);
        Mark {
            line: self.lines.len(),
            at: self.cur.len(),
            plain_at: self.plain.len(),
        }
    }

    // ------------------------------------------------------------------------
    /// Turn the text written since `mark` into a link to `href`; what it
    /// opened closes inside the link. Text that is empty (an image-only
    /// link) or spans lines stays as it is.
    pub fn link(&mut self, mark: Mark, href: &str) {
        if mark.line != self.lines.len() || mark.plain_at > self.plain.len() {
            return;
        }
        let (mut at, mut plain_at) = (mark.at, mark.plain_at);
        if self.plain[plain_at..].starts_with(' ') {
            // the space before the link belongs outside it
            at += 1;
            plain_at += 1;
        }
        if self.plain[plain_at..].trim().is_empty() {
            return;
        }
        let inside = self.written.iter().position(|&(_, start)| start >= at);
        if let Some(index) = inside {
            self.close_from(index);
            self.stale = true;
        }
        self.cur.insert(at, '[');
        self.plain.insert(plain_at, '[');
        for line in [&mut self.cur, &mut self.plain] {
            line.push_str("](");
            line.push_str(href);
            line.push(')');
        }
    }

    // ------------------------------------------------------------------------
    /// End the current line, if it has any text.
    pub fn end_line(&mut self) {
        if !self.plain.trim().is_empty() {
            self.push_line();
        }
        self.clear_line();
    }

    // ------------------------------------------------------------------------
    /// End the current line even if it is empty: an empty one separates
    /// paragraphs, like two `<br>` in a row.
    pub fn hard_break(&mut self) {
        self.push_line();
        self.clear_line();
    }

    // ------------------------------------------------------------------------
    /// Start the next line empty; the open styles are written again with its
    /// first text.
    fn clear_line(&mut self) {
        self.cur.clear();
        self.plain.clear();
        self.written.clear();
        self.stale = !self.styles.is_empty();
        self.spans = 0;
        self.space = false;
    }

    // ------------------------------------------------------------------------
    /// End the current paragraph.
    pub fn paragraph(&mut self) {
        self.end_line();
        if self.lines.last().is_some_and(|l| !l.text.is_empty()) {
            self.push_blank();
        }
    }

    // ------------------------------------------------------------------------
    /// Set the blockquote depth of what follows; a change ends the paragraph.
    pub fn set_depth(&mut self, depth: usize) {
        if depth != self.depth {
            self.paragraph();
            self.depth = depth;
        }
    }

    // ------------------------------------------------------------------------
    /// Start a list, numbered from `start` or with bullets. A list inside
    /// another one is indented to its item's text.
    pub fn start_list(&mut self, start: Option<u32>) {
        if self.lists.is_empty() {
            self.paragraph();
        } else {
            self.end_line();
        }
        let indent = self.lists.last().map_or(0, |list| list.content);
        let width = start.map_or(2, |n| format!("{n}. ").len());
        self.lists.push(List {
            next: start,
            indent,
            content: indent + width,
        });
    }

    // ------------------------------------------------------------------------
    /// Start a list item with the next text: `- `, or the list's next
    /// number. Outside a list, a bullet.
    pub fn list_item(&mut self) {
        self.end_line();
        let Some(list) = self.lists.last_mut() else {
            self.marker = Some(("- ".to_string(), 0));
            return;
        };
        let marker = match &mut list.next {
            Some(n) => {
                let marker = format!("{n}. ");
                *n = n.saturating_add(1);
                marker
            }
            None => "- ".to_string(),
        };
        list.content = list.indent + marker.len();
        self.marker = Some((marker, list.indent));
    }

    // ------------------------------------------------------------------------
    /// End the innermost list; the outermost one ends the paragraph.
    pub fn end_list(&mut self) {
        self.end_line();
        self.marker = None;
        self.lists.pop();
        if self.lists.is_empty() {
            self.paragraph();
        }
    }

    // ------------------------------------------------------------------------
    /// Push the current line, its delimiters closed. A line with text takes
    /// the pending list marker; one whose delimiters wouldn't read back is
    /// pushed without them.
    fn push_line(&mut self) {
        self.close_from(0);
        let plain = self.plain.trim();
        if plain.is_empty() {
            return self.push_blank();
        }
        let cur = self.cur.trim();
        let text = if self.spans == 0 || reads_back(cur, plain, self.spans) {
            cur.to_string()
        } else {
            plain.to_string()
        };
        let (marker, indent) = match self.marker.take() {
            Some((marker, indent)) => (Some(marker), indent),
            None => (None, self.lists.last().map_or(0, |list| list.content)),
        };
        self.lines.push(Line {
            text,
            depth: self.depth,
            indent,
            marker,
        });
    }

    // ------------------------------------------------------------------------
    fn push_blank(&mut self) {
        self.lines.push(Line {
            text: String::new(),
            depth: self.depth,
            indent: 0,
            marker: None,
        });
    }

    // ------------------------------------------------------------------------
    /// The markdown: blocks separated by a blank line (a bare `>` inside a
    /// quote), lines of a block by hard breaks, list items by a newline.
    /// Empty, or ending in exactly one newline.
    pub fn finish(mut self) -> String {
        self.end_line();

        // Blocks: maximal runs of non-blank lines of one depth.
        let mut blocks: Vec<&[Line]> = Vec::new();
        let mut start = 0;
        for i in 0..=self.lines.len() {
            let split = match self.lines.get(i) {
                None => true,
                Some(line) if line.text.is_empty() => true,
                Some(line) => i > start && line.depth != self.lines[i - 1].depth,
            };
            if split {
                if i > start {
                    blocks.push(&self.lines[start..i]);
                }
                start = if self.lines.get(i).is_some_and(|l| l.text.is_empty()) {
                    i + 1
                } else {
                    i
                };
            }
        }

        let mut out = String::new();
        let mut prev_depth: Option<usize> = None;
        for block in blocks {
            let depth = block[0].depth;
            if let Some(prev) = prev_depth {
                let shared = prev.min(depth);
                if shared > 0 {
                    out.push_str(&"> ".repeat(shared - 1));
                    out.push('>');
                }
                out.push('\n');
            }
            prev_depth = Some(depth);

            let prefix = "> ".repeat(depth);
            for (i, line) in block.iter().enumerate() {
                out.push_str(&prefix);
                out.push_str(&" ".repeat(line.indent));
                if let Some(marker) = &line.marker {
                    out.push_str(marker);
                }
                out.push_str(&escape_leading_marker(&line.text));
                match block.get(i + 1) {
                    Some(next) if next.marker.is_some() => out.push('\n'),
                    Some(_) => out.push_str("  \n"),
                    None => out.push('\n'),
                }
            }
        }
        out
    }
}

// ----------------------------------------------------------------------------
/// Whether the parser reads `line` like its `plain` copy, with all of its
/// `spans`: delimiters that don't flank their text the way CommonMark wants
/// (`**Von:**Jan`) would show as asterisks.
fn reads_back(line: &str, plain: &str, spans: usize) -> bool {
    let (text, read) = read_inline(&escape_leading_marker(line));
    read == spans && text == read_inline(&escape_leading_marker(plain)).0
}

// ----------------------------------------------------------------------------
/// Escape a leading `#`, `>`, `+`, `-`, or `N.`/`N)`: the markers that only
/// mean something at the start of a line (markdown.md#escaping, tier 2).
fn escape_leading_marker(line: &str) -> String {
    if line.starts_with(['#', '>', '+', '-']) {
        return format!("\\{line}");
    }
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0 && line[digits..].starts_with(['.', ')']) {
        return format!("{}\\{}", &line[..digits], &line[digits..]);
    }
    line.to_string()
}

// ----------------------------------------------------------------------------
/// Characters that render as nothing: zero-width spaces and joiners, soft
/// hyphens, the BOM -- newsletters pad their hidden preheaders with them.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}' | '\u{034F}' | '\u{00AD}'
    )
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn plain_text_keeps_lines_and_paragraphs() {
        assert_eq!(
            plain_to_markdown("Hello Jan,\n\nthe offer:\n- 3 boxes\n1. item\n\n\nBye\n"),
            "Hello Jan,\n\nthe offer:  \n\\- 3 boxes  \n1\\. item\n\nBye\n"
        );
        assert_eq!(plain_to_markdown(""), "");
        assert_eq!(plain_to_markdown("\n \n"), "");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn plain_text_escapes_inline_syntax() {
        assert_eq!(
            plain_to_markdown("a_b *c* [d] <e> `f` ~g~ \\h #i"),
            "a\\_b \\*c\\* \\[d\\] \\<e> \\`f\\` \\~g\\~ \\\\h #i\n"
        );
        assert_eq!(plain_to_markdown("# not a heading"), "\\# not a heading\n");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn plain_quotes_become_blockquotes() {
        let text = "Thanks!\n\nOn Monday Jan wrote:\n> Hi,\n>\n> see below\n> > older\nSigned";
        assert_eq!(
            plain_to_markdown(text),
            "Thanks!\n\nOn Monday Jan wrote:\n\n> Hi,\n>\n> see below\n>\n> > older\n\nSigned\n"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn writer_links_lists_and_breaks() {
        let mut w = MdWriter::default();
        w.text("See ");
        let mark = w.mark();
        w.text("our site");
        w.link(mark, "https://example.com/a_b");
        w.text(" now.");
        w.hard_break();
        w.text("\u{200B}Next\u{00A0}line");
        w.start_list(None);
        w.list_item();
        w.text("one");
        w.hard_break();
        w.text("more");
        w.list_item();
        w.text("two");
        w.end_list();
        let mark = w.mark();
        w.link(mark, "https://example.com/logo");
        w.text("end");
        assert_eq!(
            w.finish(),
            "See [our site](https://example.com/a_b) now.  \nNext line\n\n- one  \n  more\n- two\n\nend\n"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn writer_numbers_and_nests_lists() {
        let mut w = MdWriter::default();
        w.start_list(Some(9));
        w.list_item();
        w.text("nine");
        w.list_item();
        w.text("ten");
        w.start_list(None);
        w.list_item();
        w.text("inner");
        w.end_list();
        w.end_list();
        w.text("after");
        assert_eq!(w.finish(), "9. nine\n10. ten\n    - inner\n\nafter\n");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn writer_styles_hug_their_text() {
        let mut w = MdWriter::default();
        w.text("a ");
        w.open(Style::Strong);
        w.text(" bold ");
        w.open(Style::Emphasis);
        w.text("both");
        w.close(Style::Emphasis);
        w.close(Style::Strong);
        w.text(" b ");
        w.open(Style::Strikethrough);
        w.text(" ");
        w.close(Style::Strikethrough);
        w.text("c");
        assert_eq!(w.finish(), "a **bold *both*** b c\n");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn writer_styles_close_per_line_and_inside_links() {
        let mut w = MdWriter::default();
        w.open(Style::Strong);
        w.text("one");
        w.hard_break();
        w.text("two ");
        let mark = w.mark();
        w.open(Style::Emphasis);
        w.text("link");
        w.link(mark, "https://example.com");
        w.text(" three");
        w.close(Style::Emphasis);
        w.close(Style::Strong);
        assert_eq!(
            w.finish(),
            "**one**  \n**two [*link*](https://example.com) *three***\n"
        );
    }
}
