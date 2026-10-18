//! MIME types of the files the app hands on: to a model.

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
}
