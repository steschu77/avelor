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
