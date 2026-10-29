//! MIME types of the files the app hands on: to a model, or to Import
//! scans (is this an image); and the extension a mail's attachment
//! without one gets.

use std::path::Path;

// ----------------------------------------------------------------------------
/// The MIME type of the file types models read, by extension;
/// `application/octet-stream` for any other.
pub fn type_for(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "htm" | "html" => "text/html",
        "json" => "application/json",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

// ----------------------------------------------------------------------------
/// The extension of a file of the lowercase MIME type `content_type`:
/// [`type_for`]'s types, a mail and a calendar entry; `None` for any other.
pub fn extension_for(content_type: &str) -> Option<&'static str> {
    Some(match content_type {
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "text/plain" => "txt",
        "text/markdown" => "md",
        "text/csv" => "csv",
        "text/html" => "html",
        "application/json" => "json",
        "application/xml" => "xml",
        "message/rfc822" => "eml",
        "text/calendar" => "ics",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn types_go_by_extension_in_any_case() {
        assert_eq!(type_for(Path::new("scan.JPG")), "image/jpeg");
        assert_eq!(type_for(Path::new("a/b/letter.pdf")), "application/pdf");
        assert_eq!(type_for(Path::new("notes.md")), "text/markdown");
        assert_eq!(
            type_for(Path::new("archive.zip")),
            "application/octet-stream"
        );
        assert_eq!(type_for(Path::new("README")), "application/octet-stream");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn extensions_go_by_type() {
        assert_eq!(extension_for("application/pdf"), Some("pdf"));
        assert_eq!(extension_for("message/rfc822"), Some("eml"));
        assert_eq!(extension_for("application/zip"), None);
        for ext in [
            "pdf", "png", "jpg", "gif", "webp", "txt", "md", "csv", "html",
        ] {
            let name = format!("a.{ext}");
            assert_eq!(extension_for(type_for(Path::new(&name))), Some(ext));
        }
    }
}
