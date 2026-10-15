//! The page the web view shows, the same on every platform: one
//! self-contained HTML file, with the platform's bridge to the host and the
//! page's preferences.

use crate::app::Prefs;
use crate::sys::BRIDGE_JS;

// ----------------------------------------------------------------------------
/// The page's block for its bridge to the host (`index.html`), empty.
const BRIDGE_BLOCK: &str = r#"<script id="bridge"></script>"#;

// ----------------------------------------------------------------------------
/// The page's data block for its preferences (`index.html`), empty.
const PREFS_BLOCK: &str = r#"<script type="application/json" id="prefs">{}</script>"#;

// ----------------------------------------------------------------------------
/// The page, `app/static/index.html` with its modules bundled in
/// (`architecture.md#build`), the platform's bridge (`BRIDGE_JS`) in its
/// bridge block, so the bridge is there before the modules run, and `prefs`
/// in its preferences block. `<` is escaped: a value can't end the script
/// element.
pub fn html(prefs: &Prefs) -> String {
    let html = include_str!(concat!(env!("OUT_DIR"), "/index.html"));
    let bridge = format!(r#"<script id="bridge">{BRIDGE_JS}</script>"#);
    let json = serde_json::to_string(prefs).unwrap_or_else(|_| "{}".to_string());
    let block = PREFS_BLOCK.replace("{}", &json.replace('<', "\\u003c"));
    html.replacen(BRIDGE_BLOCK, &bridge, 1)
        .replacen(PREFS_BLOCK, &block, 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_carries_the_bridge() {
        let page = html(&Prefs::new());
        assert!(!page.contains(BRIDGE_BLOCK));
        assert!(page.contains(BRIDGE_JS));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_carries_its_preferences() {
        let empty = html(&Prefs::new());
        assert!(empty.contains(PREFS_BLOCK));

        let prefs = Prefs::from([
            ("panel_width".to_string(), "500".to_string()),
            ("note".to_string(), "</script><b>".to_string()),
        ]);
        let page = html(&prefs);
        assert!(!page.contains(PREFS_BLOCK));
        let block = r#"<script type="application/json" id="prefs">{"note":"\u003c/script>\u003cb>","panel_width":"500"}</script>"#;
        assert!(page.contains(block));
    }
}
