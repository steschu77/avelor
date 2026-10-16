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
#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn unique_trimmed_drops_empties_and_repeats_in_order() {
        let names = ["b", "", "a", "b", "c", "a"].map(String::from);
        assert_eq!(unique_trimmed(names), ["b", "a", "c"]);
        let patterns = [" *.jpg", "*.png", "", "*.jpg "].map(String::from);
        assert_eq!(unique_trimmed(patterns), ["*.jpg", "*.png"]);
    }
}
