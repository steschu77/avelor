//! The page the web view shows, the same on every platform: one
//! self-contained HTML file, with the platform's bridge to the host.

use crate::sys::BRIDGE_JS;

// ----------------------------------------------------------------------------
/// The page's block for its bridge to the host (`index.html`), empty.
const BRIDGE_BLOCK: &str = r#"<script id="bridge"></script>"#;

// ----------------------------------------------------------------------------
/// The page, `app/static/index.html` with its modules bundled in
/// (`architecture.md#build`) and the platform's bridge (`BRIDGE_JS`) in its
/// bridge block, so the bridge is there before the modules run.
pub fn html() -> String {
    let html = include_str!(concat!(env!("OUT_DIR"), "/index.html"));
    let block = format!(r#"<script id="bridge">{BRIDGE_JS}</script>"#);
    html.replacen(BRIDGE_BLOCK, &block, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_carries_the_bridge() {
        let page = html();
        assert!(!page.contains(BRIDGE_BLOCK));
        assert!(page.contains(BRIDGE_JS));
    }
}
