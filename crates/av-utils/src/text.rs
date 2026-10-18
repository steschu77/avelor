use std::borrow::Cow;

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    LF,
    CR,
    CRLF,
}

// ----------------------------------------------------------------------------
/// Strip a UTF-8 BOM and normalize line endings to the canonical in-memory form LF.
/// If the input contains mixed CRLF and lone CR line endings, CRLF wins.
pub fn normalize(text: String) -> (String, LineEnding, String) {
    let (bom, text) = match text.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}".to_string(), rest.to_string()),
        None => ("".to_string(), text),
    };
    let (crlf, text) = if text.contains("\r\n") {
        (true, text.replace("\r\n", "\n"))
    } else {
        (false, text)
    };
    let (cr, text) = if text.contains('\r') {
        (true, text.replace('\r', "\n"))
    } else {
        (false, text)
    };

    let ending = if crlf {
        LineEnding::CRLF
    } else if cr {
        LineEnding::CR
    } else {
        LineEnding::LF
    };

    (bom, ending, text)
}

// ----------------------------------------------------------------------------
pub fn renormalize(bom: &str, ending: LineEnding, text: String) -> String {
    let mut result = match ending {
        LineEnding::CR => text.replace("\n", "\r"),
        LineEnding::LF => text,
        LineEnding::CRLF => text.replace("\n", "\r\n"),
    };
    if !bom.is_empty() {
        result = format!("{bom}{result}");
    }
    result
}

// ----------------------------------------------------------------------------
/// `names` each trimmed, without the empty ones and the repeats, the first
/// of each kept where it was.
pub fn unique_trimmed(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut unique: Vec<String> = Vec::new();
    for name in names {
        let name = name.trim();
        if !name.is_empty() && !unique.iter().any(|kept| kept == name) {
            unique.push(name.to_string());
        }
    }
    unique
}

// ----------------------------------------------------------------------------
/// `text` as it is if it has at most `most` characters, else cut there,
/// trimmed at its end, with `marker` after it.
pub fn cut_marked<'a>(text: &'a str, most: usize, marker: &str) -> Cow<'a, str> {
    match text.char_indices().nth(most) {
        None => Cow::Borrowed(text),
        Some((end, _)) => Cow::Owned(format!("{}{marker}", text[..end].trim_end())),
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn a_text_is_cut_at_a_count_of_characters_and_marked() {
        assert_eq!(cut_marked("äöü", 2, " [cut]"), "äö [cut]");
        assert_eq!(
            cut_marked("ab  cd", 3, "|"),
            "ab|",
            "trimmed before the marker"
        );
        assert_eq!(cut_marked("short", 5, "|"), "short");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn unique_trimmed_drops_empties_and_repeats_in_order() {
        let names = ["b", "", "a", "b", "c", "a"].map(String::from);
        assert_eq!(unique_trimmed(names), ["b", "a", "c"]);
        let patterns = [" *.jpg", "*.png", "", "*.jpg "].map(String::from);
        assert_eq!(unique_trimmed(patterns), ["*.jpg", "*.png"]);
    }
}
