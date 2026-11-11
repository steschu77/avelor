//! The page the web view shows, the same on every platform: one
//! self-contained HTML file, with the platform's bridge to the host, the
//! page's preferences, the shipped blueprints and the texts in the run's
//! language.

use crate::app::Prefs;
use crate::blueprints;
use crate::sys::BRIDGE_JS;
use crate::texts;

// ----------------------------------------------------------------------------
/// The page's block for its bridge to the host (`index.html`), empty.
const BRIDGE_BLOCK: &str = r#"<script id="bridge"></script>"#;

// ----------------------------------------------------------------------------
/// The page's data block for its preferences (`index.html`), empty.
const PREFS_BLOCK: &str = r#"<script type="application/json" id="prefs">{}</script>"#;

// ----------------------------------------------------------------------------
/// The page's data block for the shipped blueprints (`index.html`), empty.
const BLUEPRINTS_BLOCK: &str = r#"<script type="application/json" id="blueprints">[]</script>"#;

// ----------------------------------------------------------------------------
/// The page's data block for its texts (`index.html`), empty.
const TEXTS_BLOCK: &str = r#"<script type="application/json" id="texts">{}</script>"#;

// ----------------------------------------------------------------------------
/// The page's language as `index.html` has it.
const LANGUAGE: &str = r#"<html lang="en">"#;

// ----------------------------------------------------------------------------
/// The page, `app/static/index.html` with its modules bundled in
/// (`architecture.md#build`), the platform's bridge (`BRIDGE_JS`) in its
/// bridge block, so the bridge is there before the modules run, `prefs` in
/// its preferences block, the shipped blueprints (`blueprints::json`) in
/// theirs, and the run's language (`texts::locale`) as its `lang` with its
/// catalog in the texts block and its own texts in it (`worded`). `<` is
/// escaped: a value can't end the script element.
pub fn html(prefs: &Prefs) -> String {
    let html = worded(include_str!(concat!(env!("OUT_DIR"), "/index.html")));
    let bridge = format!(r#"<script id="bridge">{BRIDGE_JS}</script>"#);
    let json = serde_json::to_string(prefs).unwrap_or_else(|_| "{}".to_string());
    let prefs = PREFS_BLOCK.replace("{}", &escaped(&json));
    let blueprints = BLUEPRINTS_BLOCK.replace("[]", &escaped(&blueprints::json()));
    let locale = texts::locale();
    let language = format!(r#"<html lang="{}">"#, locale.tag);
    let texts = TEXTS_BLOCK.replace("{}", &escaped(locale.catalog.json));
    html.replacen(LANGUAGE, &language, 1)
        .replacen(BRIDGE_BLOCK, &bridge, 1)
        .replacen(PREFS_BLOCK, &prefs, 1)
        .replacen(BLUEPRINTS_BLOCK, &blueprints, 1)
        .replacen(TEXTS_BLOCK, &texts, 1)
}

// ----------------------------------------------------------------------------
/// `html` with the texts its elements name in the run's language
/// (`localization.md#page`): `data-t="key"` the element's text,
/// `data-t-<attribute>="key"` that attribute's (`title`, `placeholder`).
/// The build checked the keys, that such an element is empty and that no
/// script has such a mark.
fn worded(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(" data-t") {
        out.push_str(&rest[..at]);
        let after = &rest[at + " data-t".len()..];
        let (attribute, after) = if let Some(after) = after.strip_prefix("=\"") {
            (None, after)
        } else if let Some((attribute, after)) = after
            .strip_prefix('-')
            .and_then(|after| after.split_once("=\""))
        {
            (Some(attribute), after)
        } else {
            out.push_str(" data-t");
            rest = after;
            continue;
        };
        let Some((key, after)) = after.split_once('"') else {
            break;
        };
        let text = texts::of(key)
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        match attribute {
            Some(attribute) => {
                out.push_str(&format!(" {attribute}=\"{text}\""));
                rest = after;
            }
            None => {
                out.push('>');
                out.push_str(&text);
                rest = after.strip_prefix('>').unwrap_or(after);
            }
        }
    }
    out.push_str(rest);
    out
}

// ----------------------------------------------------------------------------
/// `json` with every `<` escaped, so it can't end a script element.
fn escaped(json: &str) -> String {
    json.replace('<', "\\u003c")
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

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_carries_its_texts_in_the_run_s_language() {
        let page = html(&Prefs::new());
        assert!(!page.contains(TEXTS_BLOCK));
        assert!(page.contains(r#"<html lang="en">"#));
        let start = r#"<script type="application/json" id="texts">{""#;
        assert!(page.contains(start));
        assert!(page.contains(r#""status.dismiss":"Dismiss""#));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_s_own_texts_are_filled_in() {
        let html =
            r#"<b data-table-only data-t-title="status.dismiss" data-t="status.dismiss"></b>"#;
        assert_eq!(
            worded(html),
            r#"<b data-table-only title="Dismiss">Dismiss</b>"#
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_carries_the_shipped_blueprints() {
        let page = html(&Prefs::new());
        assert!(!page.contains(BLUEPRINTS_BLOCK));
        let start = r#"<script type="application/json" id="blueprints">[{"name":"travel","#;
        assert!(page.contains(start));
    }
}
