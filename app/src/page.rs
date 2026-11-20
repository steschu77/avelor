//! The page the web view shows, the same on every platform: one
//! self-contained HTML file, with the platform's bridge to the host, the
//! page's preferences, the shipped blueprints and the texts in the run's
//! language, held to its own scripts by its content security policy.

use av_utils::base64::to_base64;
use av_utils::sha256::sha256;

use crate::app::Prefs;
use crate::blueprints;
use crate::config::Theme;
use crate::sys::BRIDGE_JS;
use crate::texts;

// ----------------------------------------------------------------------------
/// The page's content security policy (`index.html`), empty.
const POLICY_BLOCK: &str = r#"<meta http-equiv="Content-Security-Policy" content="">"#;

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
/// The page's root as `index.html` has it: its language, and no theme but the
/// system's.
const ROOT: &str = r#"<html lang="en">"#;

// ----------------------------------------------------------------------------
/// The page, `app/static/index.html` with its modules bundled in
/// (`architecture.md#build`), the platform's bridge (`BRIDGE_JS`) in its
/// bridge block, so the bridge is there before the modules run, `prefs` in
/// its preferences block, the shipped blueprints (`blueprints::json`) in
/// theirs, the theme picked as its root's `data-theme` (`architecture.md#css`),
/// and the run's language (`texts::locale`) as its `lang` with its
/// catalog in the texts block and its own texts in it (`worded`). `<` is
/// escaped: a value can't end the script element. Last, its policy
/// (`with_policy`).
pub fn html(prefs: &Prefs, theme: Theme) -> String {
    let html = worded(include_str!(concat!(env!("OUT_DIR"), "/index.html")));
    let bridge = format!(r#"<script id="bridge">{BRIDGE_JS}</script>"#);
    let json = serde_json::to_string(prefs).unwrap_or_else(|_| "{}".to_string());
    let prefs = PREFS_BLOCK.replace("{}", &escaped(&json));
    let blueprints = BLUEPRINTS_BLOCK.replace("[]", &escaped(&blueprints::json()));
    let locale = texts::locale();
    let theme = match theme.name() {
        Some(name) => format!(r#" data-theme="{name}""#),
        None => String::new(),
    };
    let root = format!(r#"<html lang="{}"{theme}>"#, locale.tag);
    let texts = TEXTS_BLOCK.replace("{}", &escaped(locale.catalog.json));
    let html = html
        .replacen(ROOT, &root, 1)
        .replacen(BRIDGE_BLOCK, &bridge, 1)
        .replacen(PREFS_BLOCK, &prefs, 1)
        .replacen(BLUEPRINTS_BLOCK, &blueprints, 1)
        .replacen(TEXTS_BLOCK, &texts, 1);
    with_policy(&html)
}

// ----------------------------------------------------------------------------
/// `html` with its content security policy (`architecture.md#platforms`):
/// only the scripts it carries run, each by its SHA-256, so no handler or
/// `javascript:` link in the content does; styles inline; images only as
/// `data:` URLs, so a web image in a note or a model's reply never loads;
/// nothing fetched, framed or sent.
fn with_policy(html: &str) -> String {
    let hashes: Vec<String> = scripts(html)
        .iter()
        .map(|script| format!("'sha256-{}'", to_base64(&sha256(script.as_bytes()))))
        .collect();
    let policy = format!(
        "default-src 'none'; script-src {}; style-src 'unsafe-inline'; img-src data:; \
         base-uri 'none'; form-action 'none'",
        hashes.join(" ")
    );
    let block = POLICY_BLOCK.replace(r#"content="""#, &format!(r#"content="{policy}""#));
    html.replacen(POLICY_BLOCK, &block, 1)
}

// ----------------------------------------------------------------------------
/// The text of each script in `html` that runs: its script elements but the
/// data blocks (`type="application/json"`). A script's text ends at the first
/// `</script>`, as the browser reads it.
fn scripts(html: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("<script") {
        let Some((attributes, after)) = rest[at..].split_once('>') else {
            break;
        };
        let Some((script, after)) = after.split_once("</script>") else {
            break;
        };
        if !attributes.contains(r#"type="application/json""#) {
            found.push(script);
        }
        rest = after;
    }
    found
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
    fn the_page_s_root_names_the_theme_picked() {
        let root = |theme| {
            let page = html(&Prefs::new(), theme);
            let at = page.find("<html").unwrap();
            page[at..].split_inclusive('>').next().unwrap().to_string()
        };
        assert_eq!(root(Theme::System), r#"<html lang="en">"#);
        assert_eq!(root(Theme::Light), r#"<html lang="en" data-theme="light">"#);
        assert_eq!(root(Theme::Dark), r#"<html lang="en" data-theme="dark">"#);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_carries_the_bridge() {
        let page = html(&Prefs::new(), Theme::System);
        assert!(!page.contains(BRIDGE_BLOCK));
        assert!(page.contains(BRIDGE_JS));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_runs_only_the_scripts_it_carries() {
        let page = html(&Prefs::new(), Theme::System);
        let policy_at = page.find("Content-Security-Policy").unwrap();
        assert!(
            policy_at < page.find("<script").unwrap(),
            "before any script"
        );
        assert!(!page.contains(POLICY_BLOCK));
        let policy = &page[policy_at..page[policy_at..].find('>').unwrap() + policy_at];

        let scripts = scripts(&page);
        assert_eq!(scripts.len(), 2, "the bridge and the bundle");
        assert_eq!(scripts[0], BRIDGE_JS);
        for script in scripts {
            let hash = to_base64(&sha256(script.as_bytes()));
            assert!(policy.contains(&format!("'sha256-{hash}'")));
        }
        let directive = |name: &str| {
            let found = policy
                .split(';')
                .map(str::trim)
                .find(|d| d.starts_with(name));
            found.unwrap().to_string()
        };
        assert!(!directive("script-src").contains("unsafe"));
        assert_eq!(directive("img-src"), "img-src data:");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_data_block_is_no_script() {
        let html = r#"<script type="application/json" id="p">{"a":"<script>"}</script><script>run()</script><p>"#;
        assert_eq!(scripts(html), vec!["run()"]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_carries_its_preferences() {
        let empty = html(&Prefs::new(), Theme::System);
        assert!(empty.contains(PREFS_BLOCK));

        let prefs = Prefs::from([
            ("panel_width".to_string(), "500".to_string()),
            ("note".to_string(), "</script><b>".to_string()),
        ]);
        let page = html(&prefs, Theme::System);
        assert!(!page.contains(PREFS_BLOCK));
        let block = r#"<script type="application/json" id="prefs">{"note":"\u003c/script>\u003cb>","panel_width":"500"}</script>"#;
        assert!(page.contains(block));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_carries_its_texts_in_the_run_s_language() {
        let page = html(&Prefs::new(), Theme::System);
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
        let page = html(&Prefs::new(), Theme::System);
        assert!(!page.contains(BLUEPRINTS_BLOCK));
        let start = r#"<script type="application/json" id="blueprints">[{"name":"travel","#;
        assert!(page.contains(start));
    }
}
