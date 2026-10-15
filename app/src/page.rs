//! The page the web view shows, the same on every platform: one
//! self-contained HTML file.

// ----------------------------------------------------------------------------
/// The page, `app/static/index.html` with its modules bundled in
/// (`architecture.md#build`).
pub fn html() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/index.html"))
}
