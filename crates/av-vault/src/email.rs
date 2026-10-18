//! This is not a general-purpose mail library. Scope decisions, all
//! documented at the point they matter below:
//! - Charsets: UTF-8/US-ASCII natively, Windows-1252/ISO-8859-1 via a small
//!   table (treated as equivalent — see `av_utils::charsets::from_windows1252`),
//!   and ISO-8859-15. Anything else falls back to lossy UTF-8 with a visible
//!   marker rather than silently dropping text.
//! - Body selection: every `text/html` and every `text/plain` part that
//!   isn't an attachment is body; several of one kind are joined in order
//!   (Apple Mail splits HTML around inline images, list servers append a
//!   plain footer part). [`ParsedEmail::to_markdown`] prefers the HTML.
//!   Other parts are only ever listed (filename + content type), never
//!   extracted; images referenced from the HTML (`Content-ID`) are marked
//!   inline so they can be left out of that list.
//! - RFC 2231 continuation parameters (`boundary*0=`) are not supported;
//!   real-world boundaries/filenames are short enough that this rarely
//!   matters. A single-segment `filename*=charset''value` is.

use std::collections::HashMap;

use crate::md_writer::{self, MdWriter};
use av_utils::base64::from_base64;
use av_utils::charsets::{from_iso8859_15, from_utf8, from_windows1252};
use av_utils::text::normalize;

// ----------------------------------------------------------------------------
#[derive(Debug)]
pub struct ParsedEmail {
    pub from: String,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub html: Option<String>,
    pub plain: Option<String>,
    pub message_id: String,
    pub attachments: Vec<Attachment>,
}

// ----------------------------------------------------------------------------
/// A part that isn't body text.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub filename: String,
    /// Lowercase `type/subtype`.
    pub content_type: String,
    /// An image the HTML references by `Content-ID` (a logo, a signature),
    /// rather than a file the sender attached.
    pub inline: bool,
}

// ----------------------------------------------------------------------------
impl ParsedEmail {
    // ------------------------------------------------------------------------
    /// The body as canonical markdown (markdown.md#serializing): the HTML reduced to
    /// text, else the plain text, then a line naming the attached files.
    pub fn to_markdown(&self) -> String {
        let html = self.html.as_deref().map(crate::html::to_markdown);
        let body = match (html, &self.plain) {
            (Some(body), _) if !body.is_empty() => body,
            (_, Some(plain)) => md_writer::plain_to_markdown(plain),
            (html, None) => html.unwrap_or_default(),
        };

        let names: Vec<&str> = self
            .attachments
            .iter()
            .filter(|a| !a.inline)
            .map(|a| a.filename.as_str())
            .collect();
        if names.is_empty() {
            return body;
        }
        let mut w = MdWriter::default();
        w.text(&format!("Attachments: {}", names.join(", ")));
        let names = w.finish();
        if body.is_empty() {
            names
        } else {
            format!("{body}\n{names}")
        }
    }
}

// ----------------------------------------------------------------------------
/// A header value with its RFC 2047 encoded-words decoded; text without
/// any passes through unchanged.
pub fn decode_header(value: &str) -> String {
    decode_encoded_words(value)
}

// ----------------------------------------------------------------------------
/// Split an address list header (`To`, `Cc`) into its addresses, keeping
/// display names: commas inside quotes, angle brackets, or comments don't
/// split. Empty entries and empty groups (`undisclosed-recipients:;`) are
/// dropped.
pub fn split_addresses(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut escaped) = (false, false);
    let (mut angle, mut comment) = (0usize, 0usize);
    for c in list.chars() {
        if escaped {
            escaped = false;
        } else if quoted {
            match c {
                '\\' => escaped = true,
                '"' => quoted = false,
                _ => {}
            }
        } else {
            match c {
                '"' => quoted = true,
                '<' => angle += 1,
                '>' => angle = angle.saturating_sub(1),
                '(' => comment += 1,
                ')' => comment = comment.saturating_sub(1),
                ',' if angle == 0 && comment == 0 => {
                    out.push(std::mem::take(&mut current));
                    continue;
                }
                _ => {}
            }
        }
        current.push(c);
    }
    out.push(current);
    out.into_iter()
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty() && !a.ends_with(":;"))
        .collect()
}

// ----------------------------------------------------------------------------
struct Part {
    /// Lowercase key, raw (not RFC-2047-decoded) value.
    headers: Vec<(String, String)>,
    kind: PartKind,
}

// ----------------------------------------------------------------------------
enum PartKind {
    Multipart(Vec<Part>),
    /// Raw body bytes, before transfer-encoding/charset decoding.
    Leaf(Vec<u8>),
}

// ----------------------------------------------------------------------------
#[derive(Default)]
struct Selection {
    html: Option<String>,
    plain: Option<String>,
    attachments: Vec<Attachment>,
}

// ----------------------------------------------------------------------------
pub fn parse(bytes: &[u8]) -> ParsedEmail {
    let normalized = normalize_newlines(bytes);
    let root = parse_part(&normalized);

    let header = |key: &str| get_header(&root.headers, key).unwrap_or("").to_string();
    let from = decode_encoded_words(&header("from"));
    let to = decode_encoded_words(&header("to"));
    let cc = decode_encoded_words(&header("cc"));
    let subject = decode_encoded_words(&header("subject"));

    let mut sel = Selection::default();
    walk(&root, &mut sel);

    ParsedEmail {
        from,
        to,
        cc,
        subject,
        html: sel.html,
        plain: sel.plain,
        message_id: header("message-id").trim().to_string(),
        attachments: sel.attachments,
    }
}

// ----------------------------------------------------------------------------
fn walk(part: &Part, sel: &mut Selection) {
    match &part.kind {
        PartKind::Multipart(children) => {
            for child in children {
                walk(child, sel);
            }
        }
        PartKind::Leaf(raw) => {
            let content_type =
                get_header(&part.headers, "content-type").unwrap_or("text/plain; charset=us-ascii");
            let (main_type, params) = parse_params(content_type);
            let disposition = get_header(&part.headers, "content-disposition")
                .map(|d| parse_params(d).0)
                .unwrap_or_default();
            let body = match main_type.as_str() {
                _ if disposition == "attachment" => None,
                "text/html" => Some(&mut sel.html),
                "text/plain" => Some(&mut sel.plain),
                _ => None,
            };
            match body {
                Some(body) => {
                    let charset = params.get("charset").map_or("us-ascii", String::as_str);
                    let text = decode_charset(&transfer_decode(&part.headers, raw), charset);
                    let (_, _, text) = normalize(text);
                    match body {
                        Some(joined) => {
                            joined.push('\n');
                            joined.push_str(&text);
                        }
                        None => *body = Some(text),
                    }
                }
                None => {
                    let has_id = get_header(&part.headers, "content-id").is_some();
                    sel.attachments.push(Attachment {
                        filename: part_filename(&part.headers)
                            .unwrap_or_else(|| "attachment".into()),
                        inline: has_id && main_type.starts_with("image/"),
                        content_type: main_type,
                    });
                }
            }
        }
    }
}

// ----------------------------------------------------------------------------
/// A leaf's body with its Content-Transfer-Encoding undone.
fn transfer_decode(headers: &[(String, String)], raw: &[u8]) -> Vec<u8> {
    let cte = get_header(headers, "content-transfer-encoding").unwrap_or("7bit");
    match cte.trim().to_ascii_lowercase().as_str() {
        // Base64 is ASCII; `from_base64` skips line breaks and padding.
        "base64" => from_base64(&String::from_utf8_lossy(raw)),
        "quoted-printable" => quoted_printable_decode(raw),
        _ => raw.to_vec(),
    }
}

// ----------------------------------------------------------------------------
fn parse_part(bytes: &[u8]) -> Part {
    let (head, body) = split_head_body(bytes);
    let headers = parse_headers(head);
    let content_type = get_header(&headers, "content-type").unwrap_or("");
    let (main_type, params) = parse_params(content_type);
    if main_type.starts_with("multipart/") {
        if let Some(boundary) = params.get("boundary") {
            let children = split_multipart(body, boundary)
                .into_iter()
                .map(parse_part)
                .collect();
            return Part {
                headers,
                kind: PartKind::Multipart(children),
            };
        }
    }
    Part {
        headers,
        kind: PartKind::Leaf(body.to_vec()),
    }
}

// ----------------------------------------------------------------------------
/// Split a multipart body on `--boundary` delimiter lines into the raw
/// bytes of each part (still containing that part's own header block).
/// The closing `--boundary--` delimiter ends the scan.
fn split_multipart<'a>(body: &'a [u8], boundary: &str) -> Vec<&'a [u8]> {
    let delim = format!("--{boundary}");
    let delim = delim.as_bytes();
    let mut parts = Vec::new();
    let mut segment_start: Option<usize> = None;
    for (line_start, line_end) in line_ranges(body) {
        let line = &body[line_start..line_end];
        if !line.starts_with(delim) {
            continue;
        }
        if let Some(start) = segment_start {
            let end = line_start.saturating_sub(1).max(start); // drop the '\n' before the delimiter
            parts.push(&body[start..end]);
        }
        if line[delim.len()..].starts_with(b"--") {
            return parts; // closing delimiter
        }
        segment_start = Some((line_end + 1).min(body.len()));
    }
    parts
}

// ----------------------------------------------------------------------------
/// (start, end) of each line in `bytes`, `end` excluding the trailing `\n`.
fn line_ranges(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            out.push((start, i));
            start = i + 1;
        }
    }
    out.push((start, bytes.len()));
    out
}

// ----------------------------------------------------------------------------
fn split_head_body(bytes: &[u8]) -> (&[u8], &[u8]) {
    // A part may have no header fields at all (RFC 2046 §5.1.1): its body
    // starts right after the blank line.
    if let Some(body) = bytes.strip_prefix(b"\n") {
        return (&[], body);
    }
    match bytes.windows(2).position(|w| w == b"\n\n") {
        Some(pos) => (&bytes[..pos], &bytes[pos + 2..]),
        None => (bytes, &[]),
    }
}

// ----------------------------------------------------------------------------
// Specific u8 byte-level newline normalization
fn normalize_newlines(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' {
            out.push(b'\n');
            i += if bytes.get(i + 1) == Some(&b'\n') {
                2
            } else {
                1
            };
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

// ----------------------------------------------------------------------------
/// Unfold header continuation lines and split into lowercase-keyed pairs.
/// Values are kept raw (RFC 2047 decoding happens where a value is read).
fn parse_headers(head: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(head);
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = headers.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
        } else if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    headers
}

// ----------------------------------------------------------------------------
fn get_header<'a>(headers: &'a [(String, String)], key: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

/// Parse a `type/subtype; k=v; k="v"` header value into (lowercase main
/// type, params). Also used for Content-Disposition, which has the same
/// shape with "attachment"/"inline" in the main-type slot.
fn parse_params(value: &str) -> (String, HashMap<String, String>) {
    let mut segments = value.split(';');
    let main = segments.next().unwrap_or("").trim().to_ascii_lowercase();
    let mut params = HashMap::new();
    for seg in segments {
        if let Some((k, v)) = seg.split_once('=') {
            params.insert(
                k.trim().to_ascii_lowercase(),
                v.trim().trim_matches('"').to_string(),
            );
        }
    }
    (main, params)
}

// ----------------------------------------------------------------------------
/// The part's filename: Content-Disposition's `filename`, else
/// Content-Type's `name`, each preferring the RFC 2231 `*` form. Plain
/// values may carry RFC 2047 encoded-words, as Gmail and Outlook send them.
fn part_filename(headers: &[(String, String)]) -> Option<String> {
    let sources = [
        ("content-disposition", "filename"),
        ("content-type", "name"),
    ];
    for (header, key) in sources {
        let Some(value) = get_header(headers, header) else {
            continue;
        };
        let (_, params) = parse_params(value);
        if let Some(name) = params
            .get(&format!("{key}*"))
            .and_then(|v| decode_rfc2231(v))
        {
            return Some(name);
        }
        if let Some(name) = params.get(key) {
            return Some(decode_encoded_words(name));
        }
    }
    None
}

// ----------------------------------------------------------------------------
/// An RFC 2231 extended value, `charset'language'percent-encoded`.
fn decode_rfc2231(value: &str) -> Option<String> {
    let mut fields = value.splitn(3, '\'');
    let (charset, _language, text) = (fields.next()?, fields.next()?, fields.next()?);
    let bytes = text.as_bytes();
    let mut raw = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = match bytes[i] {
            b'%' if i + 2 < bytes.len() => hex_byte(bytes[i + 1], bytes[i + 2]),
            _ => None,
        };
        match escaped {
            Some(byte) => {
                raw.push(byte);
                i += 3;
            }
            None => {
                raw.push(bytes[i]);
                i += 1;
            }
        }
    }
    Some(decode_charset(&raw, charset))
}

// ----------------------------------------------------------------------------
/// RFC 2047 encoded-words (`=?charset?B|Q?text?=`) in header values;
/// whitespace directly between two encoded words is dropped per spec.
fn decode_encoded_words(input: &str) -> String {
    enum Tok {
        Text(String),
        Enc(String),
    }
    let mut tokens = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find("=?") {
        if start > 0 {
            tokens.push(Tok::Text(rest[..start].to_string()));
        }
        let tail = &rest[start..];
        match decode_one_encoded_word(tail) {
            Some((decoded, consumed)) => {
                tokens.push(Tok::Enc(decoded));
                rest = &tail[consumed..];
            }
            None => {
                tokens.push(Tok::Text("=?".to_string()));
                rest = &tail[2..];
            }
        }
    }
    if !rest.is_empty() {
        tokens.push(Tok::Text(rest.to_string()));
    }

    let mut out = String::new();
    for i in 0..tokens.len() {
        if let Tok::Text(t) = &tokens[i] {
            let between_encoded = i > 0
                && i + 1 < tokens.len()
                && matches!(tokens[i - 1], Tok::Enc(_))
                && matches!(tokens[i + 1], Tok::Enc(_));
            if t.trim().is_empty() && between_encoded {
                continue;
            }
        }
        match &tokens[i] {
            Tok::Text(t) | Tok::Enc(t) => out.push_str(t),
        }
    }
    out
}

// ----------------------------------------------------------------------------
fn decode_one_encoded_word(s: &str) -> Option<(String, usize)> {
    let rest = s.strip_prefix("=?")?;
    let q1 = rest.find('?')?;
    let charset = &rest[..q1];
    let rest = &rest[q1 + 1..];
    let q2 = rest.find('?')?;
    let enc = &rest[..q2];
    let rest = &rest[q2 + 1..];
    let end = rest.find("?=")?;
    let encoded_text = &rest[..end];
    let consumed = s.len() - rest.len() + end + 2;

    let raw = match enc.to_ascii_uppercase().as_str() {
        "B" => from_base64(encoded_text),
        "Q" => qp_word_decode(encoded_text.as_bytes()),
        _ => return None,
    };
    Some((decode_charset(&raw, charset), consumed))
}

// ----------------------------------------------------------------------------
/// RFC 2047 "Q" encoding: like quoted-printable but `_` means space and
/// there are no soft line breaks (an encoded-word is always one token).
fn qp_word_decode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'_' => {
                out.push(b' ');
                i += 1;
            }
            b'=' if i + 2 < bytes.len() => match hex_byte(bytes[i + 1], bytes[i + 2]) {
                Some(byte) => {
                    out.push(byte);
                    i += 3;
                }
                None => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

// ----------------------------------------------------------------------------
/// RFC 2045 quoted-printable body decoding, including soft line breaks
/// (`=\n`, after newline normalization).
fn quoted_printable_decode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'=' {
            if bytes.get(i + 1) == Some(&b'\n') {
                i += 2; // soft line break
                continue;
            }
            if i + 2 < bytes.len() {
                if let Some(byte) = hex_byte(bytes[i + 1], bytes[i + 2]) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

// ----------------------------------------------------------------------------
fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    let h = (hi as char).to_digit(16)?;
    let l = (lo as char).to_digit(16)?;
    Some(((h << 4) | l) as u8)
}

// ----------------------------------------------------------------------------
/// Text in `charset`. An unsupported charset still yields the text, as
/// lossy UTF-8 behind a visible marker, rather than dropping it.
fn decode_charset(bytes: &[u8], charset: &str) -> String {
    let charset = charset.trim().to_ascii_lowercase();
    match charset.as_str() {
        "utf-8" | "utf8" | "us-ascii" | "ascii" | "" => from_utf8(bytes),
        "iso-8859-1" | "iso8859-1" | "iso_8859-1" | "latin1" | "latin-1" | "l1"
        | "windows-1252" | "cp1252" => from_windows1252(bytes),
        "iso-8859-15" | "iso8859-15" | "iso_8859-15" | "latin9" | "latin-9" => {
            from_iso8859_15(bytes)
        }
        _ => format!("[charset {charset} not supported] {}", from_utf8(bytes)),
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use av_utils::base64::to_base64;

    fn attachment(filename: &str, content_type: &str) -> Attachment {
        Attachment {
            filename: filename.to_string(),
            content_type: content_type.to_string(),
            inline: false,
        }
    }

    #[test]
    fn plain_quoted_printable_utf8() {
        let src = b"From: Jane <jane@example.com>\r\nTo: steffen@example.com\r\nSubject: Contract\r\nDate: Thu, 28 Aug 2026 10:00:00 +0200\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nHello,=0D=0A\r\nplease find attached =E2=82=AC500.\r\n";
        let mail = parse(src);
        assert_eq!(mail.from, "Jane <jane@example.com>");
        assert_eq!(mail.subject, "Contract");
        assert!(mail.html.is_none());
        assert_eq!(
            mail.plain,
            Some(String::from(
                "Hello,\n\nplease find attached \u{20AC}500.\n"
            )),
            "CRLF from the decoded text is normalized too"
        );
        assert!(mail.attachments.is_empty());
    }

    #[test]
    fn base64_multipart_alternative() {
        let plain = String::from("Plain version");
        let html = String::from("<p>HTML version</p>");
        let plain_b64 = to_base64(plain.as_bytes());
        let html_b64 = to_base64(html.as_bytes());
        let src = format!(
            "From: a@example.com\r\nSubject: Test\r\nContent-Type: multipart/alternative; boundary=\"B1\"\r\n\r\n--B1\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{plain_b64}\r\n--B1\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{html_b64}\r\n--B1--\r\n"
        );
        let mail = parse(src.as_bytes());
        assert_eq!(mail.plain, Some(plain));
        assert_eq!(mail.html, Some(html));
        assert!(mail.attachments.is_empty());
    }

    #[test]
    fn windows1252_body_and_rfc2047_q_subject() {
        // Subject: "Café" with the 'é' Q-encoded from Windows-1252 (0xE9).
        // Body: a raw 0x80 byte (Windows-1252's euro sign, distinct from
        // ISO-8859-1's C1 control at that position) and a raw 0xE9 (which
        // both encodings map identically, straight to U+00E9).
        let mut src: Vec<u8> = b"From: a@example.com\r\nSubject: =?ISO-8859-1?Q?Caf=E9?=\r\nContent-Type: text/plain; charset=windows-1252\r\n\r\nPreis: 1".to_vec();
        src.push(0x80);
        src.extend_from_slice(b" Kaffee: caf".as_slice());
        src.push(0xE9);
        src.extend_from_slice(b".\r\n".as_slice());

        let mail = parse(&src);
        assert_eq!(mail.subject, "Caf\u{e9}");
        assert_eq!(
            mail.plain,
            Some(String::from("Preis: 1\u{20ac} Kaffee: caf\u{e9}.\n"))
        );
    }

    #[test]
    fn nested_multipart_mixed_with_attachment() {
        let src = concat!(
            "From: a@example.com\r\n",
            "Subject: Report\r\n",
            "Content-Type: multipart/mixed; boundary=\"OUTER\"\r\n\r\n",
            "--OUTER\r\n",
            "Content-Type: multipart/alternative; boundary=\"INNER\"\r\n\r\n",
            "--INNER\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n\r\n",
            "See attached.\r\n",
            "--INNER\r\n",
            "Content-Type: text/html; charset=utf-8\r\n\r\n",
            "<p>See attached.</p>\r\n",
            "--INNER--\r\n",
            "--OUTER\r\n",
            "Content-Type: application/pdf; name=\"report.pdf\"\r\n",
            "Content-Disposition: attachment; filename=\"report.pdf\"\r\n",
            "Content-Transfer-Encoding: base64\r\n\r\n",
            "JVBERi0xLjQK\r\n",
            "--OUTER--\r\n",
        );
        let mail = parse(src.as_bytes());
        assert_eq!(mail.html, Some(String::from("<p>See attached.</p>")));
        assert_eq!(mail.plain, Some(String::from("See attached.")));
        assert_eq!(
            mail.attachments,
            vec![attachment("report.pdf", "application/pdf")]
        );
    }

    #[test]
    fn missing_boundary_degrades_to_a_single_leaf_without_panicking() {
        // No `boundary` param: falls back to treating the whole thing as an
        // opaque (non-text) leaf rather than panicking on a missing split point.
        let src = b"From: a@example.com\r\nContent-Type: multipart/mixed\r\n\r\nwhatever body\r\n";
        let mail = parse(src);
        assert!(mail.html.is_none());
        assert!(mail.plain.is_none());
        assert_eq!(
            mail.attachments,
            vec![attachment("attachment", "multipart/mixed")]
        );
    }

    #[test]
    fn adjacent_encoded_words_join_without_extra_whitespace() {
        let src =
            b"From: a@example.com\r\nSubject: =?UTF-8?B?SGVs?= =?UTF-8?B?bG8=?=\r\n\r\nbody\r\n";
        let mail = parse(src);
        assert_eq!(mail.subject, "Hello");
    }

    #[test]
    fn attached_text_files_are_not_body() {
        let src = concat!(
            "Content-Type: multipart/mixed; boundary=\"M\"\r\n\r\n",
            "--M\r\n",
            "Content-Type: text/html; charset=utf-8\r\n\r\n",
            "<p>Body</p>\r\n",
            "--M\r\n",
            "Content-Type: text/html; name=\"page.html\"\r\n",
            "Content-Disposition: attachment; filename=\"page.html\"\r\n\r\n",
            "<p>Attached page</p>\r\n",
            "--M\r\n",
            "Content-Type: text/plain\r\n",
            "Content-Disposition: attachment; filename=\"notes.txt\"\r\n\r\n",
            "notes\r\n",
            "--M--\r\n",
        );
        let mail = parse(src.as_bytes());
        assert_eq!(mail.html.as_deref(), Some("<p>Body</p>"));
        assert!(mail.plain.is_none());
        assert_eq!(
            mail.attachments,
            vec![
                attachment("page.html", "text/html"),
                attachment("notes.txt", "text/plain")
            ]
        );
    }

    #[test]
    fn several_body_parts_of_one_kind_are_joined() {
        let src = concat!(
            "Content-Type: multipart/mixed; boundary=\"M\"\r\n\r\n",
            "--M\r\n",
            "Content-Type: text/html\r\n\r\n",
            "<p>One</p>\r\n",
            "--M\r\n",
            "Content-Type: image/png; name=\"logo.png\"\r\n",
            "Content-ID: <logo@x>\r\n",
            "Content-Disposition: inline; filename=\"logo.png\"\r\n\r\n",
            "iVBORw0KGgo=\r\n",
            "--M\r\n",
            "Content-Type: text/html\r\n\r\n",
            "<p>Two</p>\r\n",
            "--M--\r\n",
        );
        let mail = parse(src.as_bytes());
        assert_eq!(mail.html.as_deref(), Some("<p>One</p>\n<p>Two</p>"));
        assert_eq!(mail.attachments.len(), 1);
        assert!(mail.attachments[0].inline, "a Content-ID image is inline");
    }

    #[test]
    fn filenames_are_decoded() {
        let src = concat!(
            "Content-Type: multipart/mixed; boundary=\"M\"\r\n\r\n",
            "--M\r\n",
            "Content-Type: application/pdf; name=\"=?UTF-8?B?UmVjaG51bmcgTcOkcnoucGRm?=\"\r\n\r\n",
            "JVBERi0=\r\n",
            "--M\r\n",
            "Content-Type: application/pdf\r\n",
            "Content-Disposition: attachment; filename*=utf-8''%C3%9Cbergabe.pdf\r\n\r\n",
            "JVBERi0=\r\n",
            "--M--\r\n",
        );
        let mail = parse(src.as_bytes());
        let names: Vec<&str> = mail
            .attachments
            .iter()
            .map(|a| a.filename.as_str())
            .collect();
        assert_eq!(names, ["Rechnung M\u{e4}rz.pdf", "\u{dc}bergabe.pdf"]);
    }

    #[test]
    fn a_part_without_headers_is_plain_text() {
        let src = "Content-Type: multipart/mixed; boundary=\"M\"\r\n\r\n--M\r\n\r\nJust: text\r\n--M--\r\n";
        let mail = parse(src.as_bytes());
        assert_eq!(mail.plain.as_deref(), Some("Just: text"));
    }

    #[test]
    fn charsets_beyond_utf8_and_1252() {
        let src = b"Subject: =?iso-8859-15?Q?Preis_=A4?=\r\nContent-Type: text/plain; charset=koi8-r\r\n\r\nabc\r\n";
        let mail = parse(src);
        assert_eq!(mail.subject, "Preis \u{20ac}");
        assert_eq!(
            mail.plain.as_deref(),
            Some("[charset koi8-r not supported] abc\n")
        );
    }

    #[test]
    fn address_lists_split_outside_quotes_and_brackets() {
        let list = "\"Kramer, Jan\" <jan@example.com>, anna@example.com , <b,c@x.de> (Work, Inc), undisclosed-recipients:;";
        assert_eq!(
            split_addresses(list),
            [
                "\"Kramer, Jan\" <jan@example.com>",
                "anna@example.com",
                "<b,c@x.de> (Work, Inc)"
            ]
        );
        assert!(split_addresses("").is_empty());
    }

    #[test]
    fn markdown_prefers_html_and_lists_attached_files() {
        let src = concat!(
            "Content-Type: multipart/mixed; boundary=\"M\"\r\n\r\n",
            "--M\r\n",
            "Content-Type: multipart/alternative; boundary=\"A\"\r\n\r\n",
            "--A\r\n",
            "Content-Type: text/plain\r\n\r\n",
            "Plain *body*\r\n",
            "--A\r\n",
            "Content-Type: text/html\r\n\r\n",
            "<p>Html <b>body</b></p>\r\n",
            "--A--\r\n",
            "--M\r\n",
            "Content-Type: application/pdf; name=\"a_b.pdf\"\r\n\r\n",
            "JVBERi0=\r\n",
            "--M--\r\n",
        );
        let mail = parse(src.as_bytes());
        assert_eq!(
            mail.to_markdown(),
            "Html **body**\n\nAttachments: a\\_b.pdf\n"
        );

        let plain = parse(b"Content-Type: text/plain\r\n\r\nLine *one*\r\nLine two\r\n");
        assert_eq!(plain.to_markdown(), "Line \\*one\\*  \nLine two\n");
    }
}
