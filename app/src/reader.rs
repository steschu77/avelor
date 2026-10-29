//! What a scanned page reads to (import.md#import-scans): its text,
//! transcribed into Markdown by an AI model. Import scans reads through it.

use av_llm::{ChatRequest, ChatResponse, Client, FileData, Message, Part};
use av_utils::cancel::Cancel;
use av_utils::error::Result;

use crate::ai;

/// How many more words a file must have before its progress is sent again.
pub const PROGRESS_WORDS: usize = 10;
/// What the model is told, and asked.
const SYSTEM: &str = "You transcribe scanned pages. Write the text of the \
page in the image as Markdown: keep its headings, paragraphs, lists and tables, \
in reading order. Write only the page's text: no comments, no code fence.";
const PAGE: &str = "Transcribe this page.";

// ----------------------------------------------------------------------------
/// Ask `model` for the text of `data`, an image, streamed: `on_words` gets
/// the words read so far, every `PROGRESS_WORDS`.
pub fn ask(
    client: &Client,
    cancel: &Cancel,
    model: &str,
    data: FileData,
    mut on_words: impl FnMut(usize),
) -> Result<ChatResponse> {
    let request = ChatRequest::new(model)
        .system(SYSTEM)
        .message(Message::user(PAGE).with(Part::Inline(data)))
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
