//! What a file reads to (import.md#import-scans, mail.md#attachments): an
//! image or a PDF through an AI model, transcribed into Markdown, a text
//! file as it is. Import scans and a mail's attachments read through it.

use std::path::Path;

use av_llm::{ChatRequest, ChatResponse, Client, FileData, Message, Part, Provider};
use av_utils::cancel::Cancel;
use av_utils::charsets::from_windows1252;
use av_utils::error::Result;
use av_utils::mime::type_for;
use av_utils::text::normalize;
use av_vault::{email, html, md_writer};

use crate::ai;

/// How many more words a file must have before its progress is sent again.
pub const PROGRESS_WORDS: usize = 10;
/// What the model is told, and asked of a page (an image) or a document (a
/// PDF).
const SYSTEM: &str = "You transcribe scanned pages and documents. Write the \
text of the page in the image, or of the document, as Markdown: keep its \
headings, paragraphs, lists and tables, in reading order. Write only that \
text: no comments, no code fence.";
const PAGE: &str = "Transcribe this page.";
const DOCUMENT: &str = "Transcribe this document.";

// ----------------------------------------------------------------------------
/// The kinds of files read: the first two by a model, the others here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// JPEG, PNG, GIF or WebP.
    Image,
    Pdf,
    Markdown,
    /// Plain text, a log or CSV.
    Plain,
    Html,
    /// An attached mail (`.eml`).
    Mail,
}

impl Kind {
    // ------------------------------------------------------------------------
    /// The kind of the file `name`, by its extension; `None` for a file
    /// nothing reads.
    pub fn of(name: &str) -> Option<Kind> {
        let path = Path::new(name);
        Some(match type_for(path) {
            image if image.starts_with("image/") => Kind::Image,
            "application/pdf" => Kind::Pdf,
            "text/markdown" => Kind::Markdown,
            "text/plain" | "text/csv" => Kind::Plain,
            "text/html" => Kind::Html,
            _ => match path.extension() {
                Some(ext) if ext.eq_ignore_ascii_case("eml") => Kind::Mail,
                _ => return None,
            },
        })
    }

    // ------------------------------------------------------------------------
    /// Whether a model reads it: an image or a PDF.
    pub fn by_model(self) -> bool {
        matches!(self, Kind::Image | Kind::Pdf)
    }

    // ------------------------------------------------------------------------
    /// Whether `provider`'s models take it inline: an image every one, a PDF
    /// all but Ollama (`ProviderSpec::pdf`).
    pub fn sent_to(self, provider: Provider) -> bool {
        match self {
            Kind::Image => true,
            Kind::Pdf => provider.spec().pdf,
            _ => false,
        }
    }
}

// ----------------------------------------------------------------------------
/// The text of a file of a kind read here, as markdown: Markdown as it is,
/// plain text and CSV line by line, HTML and a mail converted. The bytes
/// are UTF-8, else Windows-1252.
pub fn text_of(kind: Kind, bytes: &[u8]) -> String {
    if kind == Kind::Mail {
        return email::parse(bytes).to_markdown();
    }
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => from_windows1252(bytes),
    };
    let (_, _, text) = normalize(text);
    match kind {
        Kind::Plain => md_writer::plain_to_markdown(&text),
        Kind::Html => html::to_markdown(&text),
        _ => text,
    }
}

// ----------------------------------------------------------------------------
/// Ask `model` for the text of `data`, a file of the kind `kind` (an image or
/// a PDF), streamed: `on_words` gets the words read so far, every
/// `PROGRESS_WORDS`.
pub fn ask(
    client: &Client,
    cancel: &Cancel,
    model: &str,
    kind: Kind,
    data: FileData,
    mut on_words: impl FnMut(usize),
) -> Result<ChatResponse> {
    let asked = if kind == Kind::Pdf { DOCUMENT } else { PAGE };
    let request = ChatRequest::new(model)
        .system(SYSTEM)
        .message(Message::user(asked).with(Part::Inline(data)))
        .context(ai::CONTEXT_TOKENS);
    let mut text = String::new();
    let mut sent = 0;
    client.chat_stream(cancel, &request, |delta| {
        text.push_str(delta);
        let words = word_count(&text);
        if words >= sent + PROGRESS_WORDS {
            sent = words;
            on_words(words);
        }
    })
}

// ----------------------------------------------------------------------------
/// The model's answer as the file's text: trimmed, without a code fence
/// around it (which some models add despite being told not to).
pub fn clean(text: &str) -> String {
    let text = text.trim();
    let unfenced = text.strip_prefix("```").and_then(|rest| {
        let (info, body) = rest.split_once('\n')?;
        // Only an info string (`markdown`), not a fenced block that starts
        // the page and ends before its end.
        if info.contains('`') || info.trim().contains(' ') {
            return None;
        }
        body.trim_end().strip_suffix("```")
    });
    unfenced.unwrap_or(text).trim().to_string()
}

// ----------------------------------------------------------------------------
/// The words of `text`: the runs between whitespace that have a letter or a
/// digit (markdown's `#`, `-` and `|` don't count).
pub fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn kinds_go_by_extension_and_providers_take_theirs() {
        assert_eq!(Kind::of("scan.JPG"), Some(Kind::Image));
        assert_eq!(Kind::of("p1.webp"), Some(Kind::Image));
        assert_eq!(Kind::of("Rechnung.PDF"), Some(Kind::Pdf));
        assert_eq!(Kind::of("notes.md"), Some(Kind::Markdown));
        assert_eq!(Kind::of("export.csv"), Some(Kind::Plain));
        assert_eq!(Kind::of("page.htm"), Some(Kind::Html));
        assert_eq!(Kind::of("Fwd.eml"), Some(Kind::Mail));
        assert_eq!(Kind::of("archive.zip"), None);
        assert_eq!(Kind::of("attachment"), None);

        assert!(Kind::Image.sent_to(Provider::Ollama));
        assert!(Kind::Pdf.sent_to(Provider::Anthropic) && Kind::Pdf.sent_to(Provider::Gemini));
        assert!(!Kind::Pdf.sent_to(Provider::Ollama));
        assert!(Kind::Pdf.by_model() && !Kind::Html.by_model());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn text_files_read_as_markdown() {
        assert_eq!(
            text_of(Kind::Markdown, b"# Notes\r\n\r\nText\r\n"),
            "# Notes\n\nText\n"
        );
        assert_eq!(
            text_of(Kind::Plain, "Gr\u{fc}\u{df}e".as_bytes()),
            "Gr\u{fc}\u{df}e\n"
        );
        assert_eq!(text_of(Kind::Plain, b"Gr\xfc\xdfe"), "Gr\u{fc}\u{df}e\n");
        assert_eq!(text_of(Kind::Plain, b"# 1"), "\\# 1\n");
        assert_eq!(
            text_of(Kind::Html, b"<p>Dear <b>Jan</b></p>").trim(),
            "Dear **Jan**"
        );
        let mail = b"From: a@x\r\nSubject: Hi\r\nContent-Type: text/plain\r\n\r\nThe text.\r\n";
        assert_eq!(text_of(Kind::Mail, mail).trim(), "The text.");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn answers_lose_a_fence_around_them() {
        assert_eq!(clean("  # Bill\n\nText\n "), "# Bill\n\nText");
        assert_eq!(
            clean("```markdown\n# Bill\n\nText\n```\n"),
            "# Bill\n\nText"
        );
        assert_eq!(clean("```\nText\n```"), "Text");
        let code = "```\nfn a()\n```\nAfter the code.";
        assert_eq!(clean(code), code, "a fenced block at the start stays");
        assert_eq!(clean(" \n"), "");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn words_are_runs_with_a_letter_or_digit() {
        assert_eq!(word_count("# Bill\n\n- 3 x 4,50 EUR\n| a | b |"), 7);
        assert_eq!(word_count(""), 0);
    }
}
