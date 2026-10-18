//! HTML mail bodies as canonical
//! markdown: paragraphs and line breaks kept, bold, italic and struck-out
//! text as `**`, `*` and `~~`, lists bulleted, numbered and nested, links as
//! `[text](href)`, quoted replies as blockquotes; headings, code, tables,
//! images, underlining and CSS styling are reduced to their text.
//!
//! Not an HTML parser: a tolerant tag tokenizer drives [`MdWriter`]
//! directly, with no tree building. Mail HTML is messy (unclosed tags,
//! Office markup, conditional comments), and a flat token walk degrades
//! gracefully where a tree builder would need the full HTML5 recovery rules.

use std::borrow::Cow;

use crate::md_writer::{Mark, MdWriter, Style};

// ----------------------------------------------------------------------------
/// The markdown text of an HTML mail body; see the module docs.
pub fn to_markdown(html: &str) -> String {
    let mut conv = Converter::default();
    let mut tokens = Tokenizer { src: html, pos: 0 };
    while let Some(token) = tokens.next_token() {
        conv.token(token);
    }
    conv.w.finish()
}

// ----------------------------------------------------------------------------
enum Token<'a> {
    Text(&'a str),
    Start(Tag),
    End(String),
}

// ----------------------------------------------------------------------------
struct Tag {
    /// Lowercase.
    name: String,
    /// Lowercase names; values with entities decoded.
    attrs: Vec<(String, String)>,
}

impl Tag {
    // ------------------------------------------------------------------------
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

// ----------------------------------------------------------------------------
struct Tokenizer<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    // ------------------------------------------------------------------------
    /// The next token; comments, doctypes, and the content of `script` and
    /// `style` are skipped.
    fn next_token(&mut self) -> Option<Token<'a>> {
        loop {
            let rest = &self.src[self.pos..];
            if rest.is_empty() {
                return None;
            }
            if !rest.starts_with('<') {
                let end = rest.find('<').unwrap_or(rest.len());
                self.pos += end;
                return Some(Token::Text(&rest[..end]));
            }
            if let Some(comment) = rest.strip_prefix("<!--") {
                self.pos += comment.find("-->").map_or(rest.len(), |i| i + 7);
                continue;
            }
            match rest.as_bytes().get(1) {
                Some(b'!' | b'?') => {
                    self.skip_past('>');
                    continue;
                }
                Some(b'/') => {
                    let name = tag_name(&rest[2..]);
                    self.skip_past('>');
                    if name.is_empty() {
                        continue;
                    }
                    return Some(Token::End(name));
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    let tag = self.start_tag();
                    if tag.name == "script" || tag.name == "style" {
                        self.skip_raw_text(&tag.name);
                    }
                    return Some(Token::Start(tag));
                }
                _ => {
                    self.pos += 1;
                    return Some(Token::Text("<"));
                }
            }
        }
    }

    // ------------------------------------------------------------------------
    fn skip_past(&mut self, c: char) {
        let rest = &self.src[self.pos..];
        self.pos += rest.find(c).map_or(rest.len(), |i| i + c.len_utf8());
    }

    // ------------------------------------------------------------------------
    /// Skip a raw-text element's content, up to (not past) its end tag.
    fn skip_raw_text(&mut self, name: &str) {
        let end = format!("</{name}");
        let rest = &self.src.as_bytes()[self.pos..];
        self.pos += rest
            .windows(end.len())
            .position(|w| w.eq_ignore_ascii_case(end.as_bytes()))
            .unwrap_or(rest.len());
    }

    // ------------------------------------------------------------------------
    /// Parse the start tag at `pos` (which is at its `<`) and move past it.
    fn start_tag(&mut self) -> Tag {
        let src = self.src;
        let bytes = src.as_bytes();
        let name = tag_name(&src[self.pos + 1..]);
        let mut i = self.pos + 1 + name.len();
        let mut attrs = Vec::new();
        loop {
            while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] == b'>' {
                break;
            }
            let start = i;
            while i < bytes.len()
                && !matches!(bytes[i], b'=' | b'>' | b'/')
                && !bytes[i].is_ascii_whitespace()
            {
                i += 1;
            }
            let attr = src[start..i].to_ascii_lowercase();
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let mut value = String::new();
            if i < bytes.len() && bytes[i] == b'=' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                let (start, end) = match bytes.get(i) {
                    Some(&q @ (b'"' | b'\'')) => {
                        let start = i + 1;
                        let end = src[start..]
                            .find(q as char)
                            .map_or(src.len(), |e| start + e);
                        i = (end + 1).min(src.len());
                        (start, end)
                    }
                    _ => {
                        let start = i;
                        while i < bytes.len() && bytes[i] != b'>' && !bytes[i].is_ascii_whitespace()
                        {
                            i += 1;
                        }
                        (start, i)
                    }
                };
                value = decode_entities(&src[start..end]).into_owned();
            }
            if !attr.is_empty() {
                attrs.push((attr, value));
            }
        }
        self.pos = (i + 1).min(src.len());
        Tag { name, attrs }
    }
}

// ----------------------------------------------------------------------------
/// The lowercase tag name at the start of `s`.
fn tag_name(s: &str) -> String {
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == ':'))
        .unwrap_or(s.len());
    s[..end].to_ascii_lowercase()
}

// ----------------------------------------------------------------------------
/// Elements that end the paragraph they're in, and the one after them.
const PARAGRAPH_TAGS: [&str; 11] = [
    "p", "h1", "h2", "h3", "h4", "h5", "h6", "table", "dl", "figure", "address",
];

/// Elements that end the line they're in, and the one after them.
const LINE_TAGS: [&str; 17] = [
    "div",
    "tr",
    "li",
    "section",
    "article",
    "header",
    "footer",
    "main",
    "nav",
    "aside",
    "center",
    "form",
    "fieldset",
    "figcaption",
    "dt",
    "dd",
    "caption",
];

/// Elements without content or end tag.
const VOID_TAGS: [&str; 14] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Element ids that start Outlook's reply history: everything after is quoted.
const OUTLOOK_REPLY_IDS: [&str; 2] = ["divrplyfwdmsg", "appendonsend"];

// ----------------------------------------------------------------------------
#[derive(Default)]
struct Converter {
    w: MdWriter,
    /// An element whose content isn't shown (`title`, `display:none`): its
    /// name, and how many of that name are open.
    skip: Option<(String, usize)>,
    /// Open `pre` elements: whitespace is kept.
    pre: usize,
    /// Open `blockquote` elements.
    quotes: usize,
    /// Outlook's reply history started: the rest is one more quote level.
    history: bool,
    /// The open link: where its text starts, and its target if it's safe.
    link: Option<(Mark, Option<String>)>,
}

impl Converter {
    // ------------------------------------------------------------------------
    fn token(&mut self, token: Token) {
        if let Some((name, open)) = &mut self.skip {
            match &token {
                Token::Start(tag) if tag.name == *name => *open += 1,
                Token::End(end) if end == name => {
                    *open -= 1;
                    if *open == 0 {
                        self.skip = None;
                    }
                }
                _ => {}
            }
            return;
        }
        match token {
            Token::Text(text) => self.text(&decode_entities(text)),
            Token::Start(tag) => self.start(tag),
            Token::End(name) => self.end(&name),
        }
    }

    // ------------------------------------------------------------------------
    fn text(&mut self, text: &str) {
        if self.pre == 0 {
            return self.w.text(text);
        }
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                self.w.hard_break();
            }
            self.w.text(line);
        }
    }

    // ------------------------------------------------------------------------
    fn start(&mut self, tag: Tag) {
        let name = tag.name.as_str();
        if !VOID_TAGS.contains(&name) && (name == "title" || is_hidden(&tag)) {
            self.skip = Some((tag.name, 1));
            return;
        }
        let id = tag.attr("id").unwrap_or_default().to_ascii_lowercase();
        if !self.history && OUTLOOK_REPLY_IDS.contains(&id.as_str()) {
            self.history = true;
            self.update_depth();
        }
        if let Some(style) = inline_style(name) {
            return self.w.open(style);
        }
        match name {
            "br" => self.w.hard_break(),
            "hr" => self.w.paragraph(),
            "pre" => {
                self.pre += 1;
                self.w.paragraph();
            }
            "blockquote" => {
                self.quotes += 1;
                self.update_depth();
            }
            "ul" => self.w.start_list(None),
            "ol" => self.w.start_list(Some(list_start(&tag))),
            "li" => self.w.list_item(),
            "td" | "th" => self.w.space(),
            "a" if self.link.is_none() => {
                let href = tag.attr("href").and_then(safe_href);
                self.link = Some((self.w.mark(), href));
            }
            _ if PARAGRAPH_TAGS.contains(&name) => self.w.paragraph(),
            _ if LINE_TAGS.contains(&name) => self.w.end_line(),
            _ => {}
        }
    }

    // ------------------------------------------------------------------------
    fn end(&mut self, name: &str) {
        if let Some(style) = inline_style(name) {
            return self.w.close(style);
        }
        match name {
            "pre" => {
                self.pre = self.pre.saturating_sub(1);
                self.w.paragraph();
            }
            "blockquote" => {
                self.quotes = self.quotes.saturating_sub(1);
                self.update_depth();
            }
            "ul" | "ol" => self.w.end_list(),
            "a" => {
                if let Some((mark, Some(href))) = self.link.take() {
                    self.w.link(mark, &href);
                }
            }
            _ if PARAGRAPH_TAGS.contains(&name) => self.w.paragraph(),
            _ if LINE_TAGS.contains(&name) => self.w.end_line(),
            _ => {}
        }
    }

    // ------------------------------------------------------------------------
    fn update_depth(&mut self) {
        self.w.set_depth(self.quotes + usize::from(self.history));
    }
}

// ----------------------------------------------------------------------------
/// The markdown style of an inline element.
fn inline_style(name: &str) -> Option<Style> {
    match name {
        "b" | "strong" => Some(Style::Strong),
        "i" | "em" => Some(Style::Emphasis),
        "s" | "strike" | "del" => Some(Style::Strikethrough),
        _ => None,
    }
}

// ----------------------------------------------------------------------------
/// An `ol`'s first number: its `start`, else 1 (and 1 for one markdown
/// can't write, of more than nine digits).
fn list_start(tag: &Tag) -> u32 {
    let start = tag.attr("start").and_then(|s| s.trim().parse().ok());
    start.filter(|&n| n < 1_000_000_000).unwrap_or(1)
}

// ----------------------------------------------------------------------------
/// Whether `tag` hides its content: newsletters' preheaders, Outlook's
/// `mso-hide:all` blocks.
fn is_hidden(tag: &Tag) -> bool {
    if tag.attr("hidden").is_some() {
        return true;
    }
    let style: String = tag
        .attr("style")
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    style.contains("display:none") || style.contains("mso-hide:all")
}

// ----------------------------------------------------------------------------
/// A link target worth keeping: web, mail, or phone. Characters that would
/// end a markdown link destination are percent-encoded.
fn safe_href(href: &str) -> Option<String> {
    let href = href.trim();
    let lower = href.to_ascii_lowercase();
    let schemes = ["http://", "https://", "mailto:", "tel:"];
    if !schemes.iter().any(|s| lower.starts_with(s)) {
        return None;
    }
    let mut out = String::with_capacity(href.len());
    for c in href.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            c if c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    Some(out)
}

// ----------------------------------------------------------------------------
/// Decode character references: numeric ones, and the named ones mail
/// actually uses. Unknown names stay as they are.
fn decode_entities(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let decoded = rest[1..]
            .find(';')
            .filter(|&semi| semi > 0 && semi <= 10)
            .and_then(|semi| entity(&rest[1..=semi]).map(|c| (c, semi + 2)));
        match decoded {
            Some((c, len)) => {
                out.extend(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

// ----------------------------------------------------------------------------
/// The character of an entity name (without `&` and `;`); `None` for a
/// character that renders as nothing (`&zwnj;`, `&shy;`).
fn entity(name: &str) -> Option<Option<char>> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return Some(Some(char::from_u32(code).unwrap_or('\u{FFFD}')));
    }
    #[rustfmt::skip]
    let c = match name {
        "amp" => '&', "lt" => '<', "gt" => '>', "quot" => '"', "apos" => '\'',
        "nbsp" | "ensp" | "emsp" | "thinsp" => ' ',
        "zwnj" | "zwj" | "shy" | "lrm" | "rlm" => return Some(None),
        "copy" => '©', "reg" => '®', "trade" => '™', "euro" => '€', "pound" => '£',
        "yen" => '¥', "cent" => '¢', "sect" => '§', "para" => '¶', "deg" => '°',
        "middot" => '·', "bull" => '•', "hellip" => '…', "ndash" => '–', "mdash" => '—',
        "lsquo" => '‘', "rsquo" => '’', "sbquo" => '‚', "ldquo" => '“', "rdquo" => '”',
        "bdquo" => '„', "laquo" => '«', "raquo" => '»', "lsaquo" => '‹', "rsaquo" => '›',
        "times" => '×', "divide" => '÷', "plusmn" => '±', "micro" => 'µ',
        "frac12" => '½', "frac14" => '¼', "frac34" => '¾', "sup2" => '²', "sup3" => '³',
        "iexcl" => '¡', "iquest" => '¿', "szlig" => 'ß',
        "Auml" => 'Ä', "auml" => 'ä', "Ouml" => 'Ö', "ouml" => 'ö', "Uuml" => 'Ü', "uuml" => 'ü',
        "Agrave" => 'À', "agrave" => 'à', "Aacute" => 'Á', "aacute" => 'á',
        "Acirc" => 'Â', "acirc" => 'â', "Atilde" => 'Ã', "atilde" => 'ã',
        "Aring" => 'Å', "aring" => 'å', "AElig" => 'Æ', "aelig" => 'æ',
        "Ccedil" => 'Ç', "ccedil" => 'ç',
        "Egrave" => 'È', "egrave" => 'è', "Eacute" => 'É', "eacute" => 'é',
        "Ecirc" => 'Ê', "ecirc" => 'ê', "Euml" => 'Ë', "euml" => 'ë',
        "Igrave" => 'Ì', "igrave" => 'ì', "Iacute" => 'Í', "iacute" => 'í',
        "Icirc" => 'Î', "icirc" => 'î', "Iuml" => 'Ï', "iuml" => 'ï',
        "Ntilde" => 'Ñ', "ntilde" => 'ñ',
        "Ograve" => 'Ò', "ograve" => 'ò', "Oacute" => 'Ó', "oacute" => 'ó',
        "Ocirc" => 'Ô', "ocirc" => 'ô', "Otilde" => 'Õ', "otilde" => 'õ',
        "Oslash" => 'Ø', "oslash" => 'ø', "OElig" => 'Œ', "oelig" => 'œ',
        "Ugrave" => 'Ù', "ugrave" => 'ù', "Uacute" => 'Ú', "uacute" => 'ú',
        "Ucirc" => 'Û', "ucirc" => 'û', "Yacute" => 'Ý', "yacute" => 'ý', "yuml" => 'ÿ',
        _ => return None,
    };
    Some(Some(c))
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn paragraphs_lines_and_whitespace() {
        let html = "<html><head><title>T</title><style>p { color: red }</style></head>\n<body>\n  <p>Hallo   Jan,</p>\n<p>das ist\n  ein Test.<br>Zweite Zeile</p></body></html>";
        assert_eq!(
            to_markdown(html),
            "Hallo Jan,\n\ndas ist ein Test.  \nZweite Zeile\n"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn gmail_divs_and_empty_lines() {
        let html = "<div dir=\"ltr\"><div>Hi,</div><div><br></div><div>see <b>below</b>.<br></div><div>Jan</div></div>";
        assert_eq!(to_markdown(html), "Hi,\n\nsee **below**.  \nJan\n");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn links_are_kept_and_unsafe_ones_dropped() {
        let html = "<p>Go <a href=\"https://x.de/a?b=1&amp;c=(2)\">to <i>the</i> site</a>, \
                    <a href=\"javascript:alert(1)\">not this</a>, \
                    <a href=\"https://x.de/logo\"><img src=\"logo.png\"></a>end</p>";
        assert_eq!(
            to_markdown(html),
            "Go [to *the* site](https://x.de/a?b=1&c=%282%29), not this, end\n"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn quoted_replies_become_blockquotes() {
        let html = "<div>Danke!</div><div class=\"gmail_quote\"><div class=\"gmail_attr\">Am Mo. schrieb Jan:<br></div>\
                    <blockquote class=\"gmail_quote\"><div>Hallo</div><blockquote>Alt</blockquote></blockquote></div>";
        assert_eq!(
            to_markdown(html),
            "Danke!  \nAm Mo. schrieb Jan:\n\n> Hallo\n>\n> > Alt\n"
        );

        let outlook = "<p>Neu</p><div id=\"appendonsend\"></div><hr><div id=\"divRplyFwdMsg\"><b>Von:</b> Jan</div><p>Alt</p>";
        assert_eq!(to_markdown(outlook), "Neu\n\n> **Von:** Jan\n>\n> Alt\n");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn tables_lists_and_pre() {
        let html = "<table><tr><td>Betrag</td><td>12,00 &euro;</td></tr><tr><td>MwSt</td><td>2,28&nbsp;&#8364;</td></tr></table>\
                    <ul><li>eins<li>zwei</ul><pre>a  b\nc</pre>";
        assert_eq!(
            to_markdown(html),
            "Betrag 12,00 €  \nMwSt 2,28 €\n\n- eins\n- zwei\n\na b  \nc\n"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn hidden_content_comments_and_escaping() {
        let html = "<!--[if mso]><p>Outlook only</p><![endif]-->\
                    <div style=\"display: none; max-height:0\">Preheader &zwnj;&nbsp;&zwnj;<div>nested</div></div>\
                    <p>1. Rang: *wow* &lt;tag&gt; &unknown; a < b</p><script>var x = '<p>';</script>";
        assert_eq!(
            to_markdown(html),
            "1\\. Rang: \\*wow\\* \\<tag> &unknown; a \\< b\n"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn emphasis_hugs_its_text() {
        let html = "<p><b>bold</b>, <strong>strong</strong>, <i>it</i>, <em>em</em>, \
                    <del>gone</del>, <s>s</s> and <strike>old</strike></p>\
                    <p><b><i>both</i></b> a<b> spaced </b>b <b></b><i> </i>c <b>one</b><b>word</b></p>";
        assert_eq!(
            to_markdown(html),
            "**bold**, **strong**, *it*, *em*, ~~gone~~, ~~s~~ and ~~old~~\n\n\
             ***both*** a **spaced** b c **oneword**\n"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn emphasis_closes_per_line_and_inside_links() {
        let html = "<b>one<br>two</b><p><i>a</i></p><p><i>b</i></p>\
                    <p><a href=\"https://x.de\"><b>Click</b></a> <b>*note*</b> \
                    <i>snake_case</i> <s>~x~</s></p>";
        assert_eq!(
            to_markdown(html),
            "**one**  \n**two**\n\n*a*\n\n*b*\n\n\
             [**Click**](https://x.de) **\\*note\\*** *snake\\_case* ~~\\~x\\~~~\n"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn emphasis_that_wouldnt_read_back_is_left_out() {
        // CommonMark closes no `**` after punctuation before a letter, and
        // opens none after a letter before punctuation; misnested elements
        // can't be written at all. The whole line is kept plain.
        let html = "<p><b>Von:</b>Jan</p><p>foo<b>(bar)</b> ok <b>yes</b></p>\
                    <p><b>a <a href=\"https://x.de\">b</b> c</a></p>";
        assert_eq!(
            to_markdown(html),
            "Von:Jan\n\nfoo(bar) ok yes\n\na [b c](https://x.de)\n"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn lists_are_numbered_and_nested() {
        let html = "<ol><li>one</li><li>two<ul><li>a<br>more</li><li>b</li></ul></li><li>three</li></ol>\
                    <ul><li>x</li><ul><li>gmail</li></ul><li>y</li></ul>\
                    <ol start=\"9\"><li>nine</li><li>ten<ol><li>in</li></ol></li></ol>";
        assert_eq!(
            to_markdown(html),
            "1. one\n2. two\n   - a  \n     more\n   - b\n3. three\n\n\
             - x\n  - gmail\n- y\n\n\
             9. nine\n10. ten\n    1. in\n"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn items_of_paragraphs_make_a_loose_list() {
        let html = "<ul><li><p>a</p></li><li><p>b</p><p>more</p></li></ul><p>after</p>";
        assert_eq!(to_markdown(html), "- a\n\n- b\n\n  more\n\nafter\n");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn editor_markdown_survives_the_round_trip() {
        let bodies = [
            "Para one.\n\nPara two.\n",
            "Line one  \nline two\n",
            "**Bold**, *italic*, ***both*** and ~~struck~~.\n",
            "A [**bold** link](https://example.com) and *\\*stars\\**.\n",
            "> quoted *text*\n>\n> > nested\n",
            "1. one\n2. two\n",
            "3. three\n4. four\n",
            "- a  \n  b\n- c\n",
            "- a\n  - b\n    1. c\n- d\n",
            "1. a\n   1. b\n2. c\n",
            "- a\n\n- b\n",
        ];
        for md in bodies {
            assert_eq!(to_markdown(&crate::markdown::to_html(md)), md);
        }
    }
}
