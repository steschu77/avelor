//! The page the web view shows, the same on every platform: one
//! self-contained HTML file.

// ----------------------------------------------------------------------------
/// The page, `app/static/index.html`.
pub fn html() -> &'static str {
    include_str!("../static/index.html")
}
