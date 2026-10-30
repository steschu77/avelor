//! What a file reads to (import.md#import-scans, import.md#reading-pdfs,
//! mail.md#attachments): an image through an AI model, transcribed into
//! Markdown; a PDF from its text layer and its scanned pages through the
//! model, the whole file only where that can't be done, an encrypted one
//! not at all; a text file as it is. Import scans and a mail's attachments
//! read through it.

use std::path::Path;

use av_llm::{ChatRequest, ChatResponse, Client, FileData, Message, Part, Provider, StopReason};
use av_pdf::Document;
use av_utils::cancel::Cancel;
use av_utils::charsets::from_windows1252;
use av_utils::error::Error;
use av_utils::mime::type_for;
use av_utils::text::normalize;
use av_vault::{email, html, md_writer};

use crate::ai;

/// How many more words a file must have before its progress is sent again.
pub const PROGRESS_WORDS: usize = 10;
/// The largest scanned page sent as an image of its own to a model that
/// takes PDFs too: Anthropic takes images up to 5 MB as base64. A larger
/// page goes with the whole file.
const MAX_PAGE_IMAGE: usize = (5 << 20) / 4 * 3;
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
    /// Whether a model may read it: an image, or a PDF's scanned pages.
    pub fn by_model(self) -> bool {
        matches!(self, Kind::Image | Kind::Pdf)
    }
}

// ----------------------------------------------------------------------------
/// What reads a file's images and scanned pages.
#[derive(Clone)]
pub enum Engine {
    /// A model of an AI account; `address` names the account in failures.
    Ai {
        client: Client,
        provider: Provider,
        address: String,
        model: String,
    },
}

// ----------------------------------------------------------------------------
/// Why a file read to no text.
#[derive(Debug)]
pub enum ReadError {
    /// The file can't be read from the disk.
    File(Error),
    /// The model's request failed.
    Model(Error),
    /// The model declined to read it.
    Refused,
    /// The model read no text.
    Blank,
    /// The file has no text.
    NoText,
    /// An encrypted PDF, which no model is sent.
    Encrypted,
    /// It can't be read here, in words.
    Unreadable(String),
}

impl ReadError {
    // ------------------------------------------------------------------------
    /// The failure in words, a model's failure named after `engine`'s
    /// account.
    pub fn words(&self, engine: &Engine) -> String {
        let Engine::Ai {
            provider, address, ..
        } = engine;
        match self {
            ReadError::File(_) => "Could not read the file.".to_string(),
            ReadError::Model(e) => ai::chat_error(*provider, address, e),
            ReadError::Refused => "The model declined to read the file.".to_string(),
            ReadError::Blank => "The model read no text.".to_string(),
            ReadError::NoText => "The file has no text.".to_string(),
            ReadError::Encrypted => "The PDF is encrypted; it isn't sent to a model.".to_string(),
            ReadError::Unreadable(why) => why.clone(),
        }
    }
}

// ----------------------------------------------------------------------------
/// The text of `data`, the file `name` of the kind `kind`, read with
/// `engine` where it needs a model, as markdown. `on_words` gets the words
/// read so far, every `PROGRESS_WORDS` while a model answers and after each
/// page of a PDF.
pub fn read(
    engine: &Engine,
    cancel: &Cancel,
    name: &str,
    kind: Kind,
    data: Vec<u8>,
    mut on_words: impl FnMut(usize),
) -> Result<String, ReadError> {
    let text = match kind {
        Kind::Image => {
            let mime_type = type_for(Path::new(name)).to_string();
            let file = FileData {
                name: name.to_string(),
                mime_type,
                data,
            };
            transcribe(engine, cancel, Kind::Image, file, &mut on_words)?
        }
        Kind::Pdf => read_pdf(engine, cancel, name, data, &mut on_words)?,
        _ => text_of(kind, &data).trim().to_string(),
    };
    match text.is_empty() {
        true => Err(ReadError::NoText),
        false => Ok(text),
    }
}

// ----------------------------------------------------------------------------
/// A PDF's page as `plan_pdf` sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageFacts {
    /// Its text layer stands for it.
    Text,
    /// Nothing on it.
    Blank,
    /// A JPEG image covering most of it, its text layer too thin.
    Jpeg,
    /// Neither: an image of another kind, drawings, text without its
    /// characters.
    Other,
}

// ----------------------------------------------------------------------------
/// How a PDF is read.
#[derive(Debug, PartialEq)]
pub enum PdfPlan {
    /// From its text layer, without a model.
    TextLayer,
    /// Page by page: a text page as it is, a JPEG page as an image by the
    /// model.
    Pages,
    /// The whole file sent to the model, for pages that are neither.
    WholeFile,
    /// Not at all: it is encrypted.
    Encrypted,
    /// Not at all, in words.
    Unreadable(String),
}

// ----------------------------------------------------------------------------
/// What a page read to tells `plan_pdf`, for a model that takes PDFs
/// (`takes_pdf`) or not. A page with a little text and no large image (a
/// cover sheet) is text; a scan too large to send alone is neither for a
/// model that can take the whole file instead.
fn facts_of(page: &av_pdf::Page, takes_pdf: bool) -> PageFacts {
    let mapped = page.glyphs > 0 && page.unmapped * 10 <= page.glyphs;
    if page.usable() {
        PageFacts::Text
    } else if let Some(scan) = &page.scan {
        match takes_pdf && scan.jpeg.len() > MAX_PAGE_IMAGE {
            true => PageFacts::Other,
            false => PageFacts::Jpeg,
        }
    } else if page.cover < av_pdf::SCAN_COVER && mapped {
        PageFacts::Text
    } else if page.blank {
        PageFacts::Blank
    } else {
        PageFacts::Other
    }
}

// ----------------------------------------------------------------------------
/// How to read a PDF of the pages `pages` (or that couldn't be parsed) with
/// a model that takes PDFs (`takes_pdf`) or not: its text layer when every
/// page has one (or is blank), else page by page if every page is text or a
/// JPEG, else the whole file if the model takes it. An encrypted one isn't
/// read.
pub fn plan_pdf(pages: Result<&[PageFacts], &av_pdf::Error>, takes_pdf: bool) -> PdfPlan {
    let pages = match pages {
        Ok(pages) => pages,
        Err(av_pdf::Error::Encrypted) => return PdfPlan::Encrypted,
        Err(_) if takes_pdf => return PdfPlan::WholeFile,
        Err(_) => return PdfPlan::Unreadable("The PDF couldn't be read.".to_string()),
    };
    let text_or_blank = pages
        .iter()
        .all(|p| matches!(p, PageFacts::Text | PageFacts::Blank));
    if text_or_blank && pages.contains(&PageFacts::Text) {
        return PdfPlan::TextLayer;
    }
    match pages.iter().position(|p| *p == PageFacts::Other) {
        None => PdfPlan::Pages,
        Some(_) if takes_pdf => PdfPlan::WholeFile,
        Some(i) => PdfPlan::Unreadable(format!(
            "Page {} has neither a text layer nor a scanned image this model can read.",
            i + 1
        )),
    }
}

// ----------------------------------------------------------------------------
/// A PDF's text, by `plan_pdf`.
fn read_pdf(
    engine: &Engine,
    cancel: &Cancel,
    name: &str,
    data: Vec<u8>,
    on_words: &mut impl FnMut(usize),
) -> Result<String, ReadError> {
    let pages: Result<Vec<av_pdf::Page>, av_pdf::Error> = Document::parse(&data)
        .map(|doc| (0..doc.page_count()).filter_map(|i| doc.page(i)).collect());
    let Engine::Ai { provider, .. } = engine;
    let takes_pdf = provider.spec().pdf;
    let facts: Result<Vec<PageFacts>, av_pdf::Error> = pages
        .as_ref()
        .map(|pages| pages.iter().map(|p| facts_of(p, takes_pdf)).collect())
        .map_err(|e| *e);
    let plan = plan_pdf(facts.as_deref(), takes_pdf);
    match plan {
        PdfPlan::WholeFile => {
            let file = FileData {
                name: name.to_string(),
                mime_type: "application/pdf".to_string(),
                data,
            };
            transcribe(engine, cancel, Kind::Pdf, file, on_words)
        }
        PdfPlan::Encrypted => Err(ReadError::Encrypted),
        PdfPlan::Unreadable(why) => Err(ReadError::Unreadable(why)),
        PdfPlan::TextLayer | PdfPlan::Pages => {
            let (Ok(pages), Ok(facts)) = (pages, facts) else {
                return Err(ReadError::NoText);
            };
            let mut texts = Vec::new();
            let mut done = 0;
            for (i, (page, facts)) in pages.into_iter().zip(facts).enumerate() {
                if cancel.is_cancelled() {
                    return Err(ReadError::Model(Error::Cancelled));
                }
                let text = match (facts, page.scan) {
                    (PageFacts::Text, _) => md_writer::plain_to_markdown(&page.text),
                    (PageFacts::Jpeg, Some(scan)) => {
                        let file = FileData {
                            name: format!("{name}, page {}.jpg", i + 1),
                            mime_type: "image/jpeg".to_string(),
                            data: scan.jpeg,
                        };
                        let mut page_words = |words| on_words(done + words);
                        match transcribe(engine, cancel, Kind::Image, file, &mut page_words) {
                            Err(ReadError::Blank) => String::new(),
                            other => other?,
                        }
                    }
                    _ => String::new(),
                };
                let text = text.trim().to_string();
                if !text.is_empty() {
                    done += word_count(&text);
                    on_words(done);
                    texts.push(text);
                }
            }
            Ok(texts.join("\n\n"))
        }
    }
}

// ----------------------------------------------------------------------------
/// What `engine` reads `file` (an image, or a PDF of the kind `kind`) to,
/// cleaned.
fn transcribe(
    engine: &Engine,
    cancel: &Cancel,
    kind: Kind,
    file: FileData,
    on_words: &mut impl FnMut(usize),
) -> Result<String, ReadError> {
    let Engine::Ai { client, model, .. } = engine;
    let response = ask(client, cancel, model, kind, file, on_words).map_err(ReadError::Model)?;
    let text = clean(&response.text);
    match (text.is_empty(), response.stop) {
        (false, _) => Ok(text),
        (true, StopReason::Refused) => Err(ReadError::Refused),
        (true, _) => Err(ReadError::Blank),
    }
}

// ----------------------------------------------------------------------------
/// The text of a file of a kind read here, as markdown: Markdown as it is,
/// plain text and CSV line by line, HTML and a mail converted. The bytes
/// are UTF-8, else Windows-1252.
fn text_of(kind: Kind, bytes: &[u8]) -> String {
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
) -> Result<ChatResponse, Error> {
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
    fn kinds_go_by_extension() {
        assert_eq!(Kind::of("scan.JPG"), Some(Kind::Image));
        assert_eq!(Kind::of("p1.webp"), Some(Kind::Image));
        assert_eq!(Kind::of("Rechnung.PDF"), Some(Kind::Pdf));
        assert_eq!(Kind::of("notes.md"), Some(Kind::Markdown));
        assert_eq!(Kind::of("export.csv"), Some(Kind::Plain));
        assert_eq!(Kind::of("page.htm"), Some(Kind::Html));
        assert_eq!(Kind::of("Fwd.eml"), Some(Kind::Mail));
        assert_eq!(Kind::of("archive.zip"), None);
        assert_eq!(Kind::of("attachment"), None);
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
    fn a_pdf_is_read_from_its_text_layer_and_scans_else_whole() {
        use PageFacts::*;
        let plan = |pages: &[PageFacts], takes_pdf| plan_pdf(Ok(pages), takes_pdf);
        assert_eq!(plan(&[Text, Blank, Text], true), PdfPlan::TextLayer);
        assert_eq!(plan(&[Text, Blank], false), PdfPlan::TextLayer);
        for takes_pdf in [true, false] {
            assert_eq!(plan(&[Text, Jpeg, Blank], takes_pdf), PdfPlan::Pages);
            assert_eq!(plan(&[Jpeg, Jpeg], takes_pdf), PdfPlan::Pages);
        }
        assert_eq!(plan(&[Blank], false), PdfPlan::Pages, "nothing to read");
        assert_eq!(plan(&[Other], true), PdfPlan::WholeFile);
        assert_eq!(plan(&[Jpeg, Other], true), PdfPlan::WholeFile);
        let PdfPlan::Unreadable(why) = plan(&[Jpeg, Other], false) else {
            panic!("page 2 can't be read");
        };
        assert!(why.starts_with("Page 2 "), "{why}");

        let encrypted = av_pdf::Error::Encrypted;
        for takes_pdf in [true, false] {
            assert_eq!(plan_pdf(Err(&encrypted), takes_pdf), PdfPlan::Encrypted);
        }
        let broken = av_pdf::Error::Broken("no pages");
        assert_eq!(plan_pdf(Err(&broken), true), PdfPlan::WholeFile);
        assert!(matches!(
            plan_pdf(Err(&broken), false),
            PdfPlan::Unreadable(_)
        ));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_scan_too_large_to_send_alone_goes_with_the_whole_file() {
        let page = |size| av_pdf::Page {
            scan: Some(av_pdf::Scan {
                jpeg: vec![0; size],
                upright: true,
                cmyk: false,
            }),
            ..Default::default()
        };
        assert_eq!(facts_of(&page(1000), true), PageFacts::Jpeg);
        let large = page(MAX_PAGE_IMAGE + 1);
        assert_eq!(facts_of(&large, true), PageFacts::Other);
        assert_eq!(
            facts_of(&large, false),
            PageFacts::Jpeg,
            "Ollama has no limit"
        );
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
