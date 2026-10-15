//! The app's texts in the user's language (`localization.md`): the catalogs
//! the build checked and wrote (`build.rs`), the language a run shows, the
//! texts the host shows or writes itself, and those it gives the page.

use std::sync::OnceLock;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::sys;

// ----------------------------------------------------------------------------
/// A language's catalog, its texts over English's, as JSON: the page reads
/// it, and the host once it needs a text of its own (`text`).
pub struct Catalog {
    pub language: &'static str,
    pub json: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/texts.rs"));

// ----------------------------------------------------------------------------
/// The language a run shows: its catalog, its tag with the system's
/// region for it (`de-AT`), which the page's dates and numbers and the web
/// view follow.
pub struct Locale {
    pub catalog: &'static Catalog,
    pub tag: String,
}

static LOCALE: OnceLock<Locale> = OnceLock::new();

// ----------------------------------------------------------------------------
/// Choose the run's language, once, at the start: `configured`, the
/// config's `language`, if a catalog has it, else the system's
/// (`localization.md#language`).
pub fn init(configured: Option<&str>) {
    let locale = choose(configured, &sys::languages());
    log::info!("Language: {} ({})", locale.tag, locale.catalog.language);
    let _ = LOCALE.set(locale);
}

// ----------------------------------------------------------------------------
/// The run's language: the system's before `init` (a start that failed
/// before it), English in the tests.
pub fn locale() -> &'static Locale {
    LOCALE.get_or_init(|| {
        let system = if cfg!(test) {
            Vec::new()
        } else {
            sys::languages()
        };
        choose(None, &system)
    })
}

// ----------------------------------------------------------------------------
/// The text of `key` in the run's language, one the host shows or writes
/// itself; the build checked that every language has it, plain.
pub fn text(key: &'static str) -> &'static str {
    texts().get(key).and_then(Value::as_str).unwrap_or(key)
}

/// The run's catalog, read once the host needs a text of it.
fn texts() -> &'static Map<String, Value> {
    static TEXTS: OnceLock<Map<String, Value>> = OnceLock::new();
    TEXTS.get_or_init(|| serde_json::from_str(locale().catalog.json).unwrap_or_default())
}

// ----------------------------------------------------------------------------
/// A text the page shows in its language (`localization.md#host-texts`):
/// a key of the catalogs and what its placeholders are given, each a
/// string, a number or a text itself. The page gets `{key, args}`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Text {
    pub key: &'static str,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub args: Map<String, Value>,
}

impl Text {
    // ------------------------------------------------------------------------
    pub fn new(key: &'static str) -> Text {
        Text {
            key,
            args: Map::new(),
        }
    }

    // ------------------------------------------------------------------------
    /// The text with `value` for its placeholder `{name}`.
    pub fn arg(mut self, name: &str, value: impl Into<Value>) -> Text {
        self.args.insert(name.to_string(), value.into());
        self
    }
}

impl From<Text> for Value {
    fn from(text: Text) -> Value {
        serde_json::to_value(text).unwrap_or_default()
    }
}

/// For the log: the key and what it is given.
impl std::fmt::Display for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.key)?;
        if !self.args.is_empty() {
            write!(f, " {}", Value::Object(self.args.clone()))?;
        }
        Ok(())
    }
}

// ----------------------------------------------------------------------------
/// The language of `configured`, else of the first of the `system`'s
/// languages a catalog has, else English. A configured language without a
/// region takes the system's for it.
fn choose(configured: Option<&str>, system: &[String]) -> Locale {
    let catalog_of = |tag: &str| {
        let language = primary(tag);
        CATALOGS.iter().find(|c| c.language == language)
    };
    let system: Vec<String> = system.iter().map(|tag| normalized(tag)).collect();
    if let Some(tag) = configured.map(normalized) {
        if let Some(catalog) = catalog_of(&tag) {
            let regional = system
                .iter()
                .find(|system| primary(system) == catalog.language && system.contains('-'));
            let tag = match regional {
                Some(regional) if !tag.contains('-') => regional.clone(),
                _ => tag,
            };
            return Locale { catalog, tag };
        }
    }
    for tag in system {
        if let Some(catalog) = catalog_of(&tag) {
            return Locale { catalog, tag };
        }
    }
    let catalog = catalog_of("en").expect("the catalogs have English");
    Locale {
        catalog,
        tag: "en".to_string(),
    }
}

// ----------------------------------------------------------------------------
/// `tag` as BCP 47 writes it: `de_AT.UTF-8` is `de-AT`; letters, digits and
/// hyphens only, as it goes into the page's `lang`.
fn normalized(tag: &str) -> String {
    let tag = tag.split(['.', '@']).next().unwrap_or_default();
    let tag = tag.replace('_', "-");
    tag.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

/// The language of `tag`, lowercase: `de` of `de-AT`.
fn primary(tag: &str) -> String {
    let language = tag.split('-').next().unwrap_or_default();
    language.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(tags: &[&str]) -> Vec<String> {
        tags.iter().map(|tag| tag.to_string()).collect()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_system_s_first_language_with_a_catalog_is_shown() {
        let locale = choose(None, &tags(&["fy-NL", "de-AT", "en-US"]));
        assert_eq!(locale.catalog.language, "de");
        assert_eq!(locale.tag, "de-AT");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_configured_language_comes_first_with_the_system_s_region() {
        let locale = choose(Some("es"), &tags(&["de-DE", "es-MX"]));
        assert_eq!(locale.catalog.language, "es");
        assert_eq!(locale.tag, "es-MX");

        let locale = choose(Some("de-CH"), &tags(&["de-DE"]));
        assert_eq!(locale.tag, "de-CH");

        let locale = choose(Some("de"), &tags(&["en-US"]));
        assert_eq!(locale.tag, "de");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_language_without_a_catalog_falls_back() {
        let locale = choose(Some("tlh"), &tags(&["C", "fy-NL"]));
        assert_eq!(locale.catalog.language, "en");
        assert_eq!(locale.tag, "en");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn linux_s_names_are_read_as_tags() {
        let locale = choose(None, &tags(&["de_AT.UTF-8", "de_AT", "de", "C"]));
        assert_eq!(locale.tag, "de-AT");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_host_s_texts_are_the_language_s() {
        assert_eq!(locale().catalog.language, "en");
        assert!(text("start.failed").starts_with("An error occurred."));
        let de = choose(Some("de"), &[]);
        let texts: Map<String, Value> = serde_json::from_str(de.catalog.json).unwrap();
        assert!(
            texts["start.failed"]
                .as_str()
                .unwrap()
                .starts_with("Ein Fehler")
        );
    }
}
