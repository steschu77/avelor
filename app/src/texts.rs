//! The app's texts in the user's language (`localization.md`): the catalogs
//! the build checked and wrote (`build.rs`), the language a run shows, the
//! texts the host shows or writes itself, and those it gives the page.

use std::sync::OnceLock;

use av_utils::datetime::{Date, DateTime};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::sys;

// ----------------------------------------------------------------------------
/// A language's catalog, its texts over English's, as JSON: the page reads
/// it, and the host once it needs a text of its own (`text`). A formal one
/// has the formal texts over its language's.
pub struct Catalog {
    pub language: &'static str,
    pub formal: bool,
    pub json: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/texts.rs"));

// ----------------------------------------------------------------------------
/// The language a run shows: its catalog, its tag with the system's
/// region for it (`de-AT`), which the page's dates and numbers and the web
/// view follow, and how it addresses the user.
pub struct Locale {
    pub catalog: &'static Catalog,
    pub tag: String,
    pub address: Address,
}

// ----------------------------------------------------------------------------
/// How the texts address the user, as the config's `language` says after
/// its tag (`de, informal`): informally ("du", "tú"), or formally where the
/// language has a formal catalog (`de.formal.json`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Address {
    Informal,
    Formal,
}

static LOCALE: OnceLock<Locale> = OnceLock::new();

// ----------------------------------------------------------------------------
/// Choose the run's language, once, at the start: `configured`, the
/// config's `language`, if a catalog has it, else the system's
/// (`localization.md#language`).
pub fn init(configured: Option<&str>) {
    let locale = choose(configured, &sys::languages());
    log::info!(
        "Language: {} ({}), {:?}",
        locale.tag,
        locale.catalog.language,
        locale.address
    );
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
/// The text of `key` as a data file of the app names it (a blueprint's
/// words, `blueprints.rs`), whose keys the build reads from the file.
pub fn of(key: &str) -> String {
    texts()
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(key)
        .to_string()
}

// ----------------------------------------------------------------------------
/// `text`'s text of `key` with its placeholders filled in from `args`.
pub fn filled(key: &'static str, args: &[(&str, &str)]) -> String {
    let mut text = text(key).to_string();
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

// ----------------------------------------------------------------------------
/// `date` for a text's placeholder: the page writes it as the locale does
/// (`core/text.js`), the host as ISO 8601.
pub fn date(date: Date) -> Value {
    let day = DateTime { date, time: None };
    serde_json::json!({ "date": day.as_iso8601() })
}

// ----------------------------------------------------------------------------
/// `amount` in `currency`, an ISO code, for a text's placeholder: the page
/// writes it as the locale writes money.
pub fn money(amount: f64, currency: &str) -> Value {
    serde_json::json!({ "amount": amount, "currency": currency })
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

    // ------------------------------------------------------------------------
    /// The text in the run's language, for what the host writes into the
    /// vault itself: its placeholders filled in, a text given rendered too,
    /// a count's form `one` for 1 and `other` else.
    pub fn render(&self) -> String {
        rendered(self.key, &self.args)
    }
}

/// The text of `key` with `args`, as `Text::render` writes it.
pub(crate) fn rendered(key: &str, args: &Map<String, Value>) -> String {
    let form = match args.get("n").and_then(Value::as_u64) {
        Some(1) => "one",
        _ => "other",
    };
    let text = match texts().get(key) {
        Some(Value::String(text)) => text.as_str(),
        Some(Value::Object(forms)) => {
            let text = forms.get(form).or_else(|| forms.get("other"));
            text.and_then(Value::as_str).unwrap_or(key)
        }
        _ => key,
    };
    let mut out = text.to_string();
    for (name, value) in args {
        let value = match value {
            Value::String(value) => value.clone(),
            Value::Object(text) => match (text.get("key"), text.get("args"), text.get("date")) {
                (Some(Value::String(key)), Some(Value::Object(args)), _) => rendered(key, args),
                (Some(Value::String(key)), ..) => rendered(key, &Map::new()),
                (.., Some(Value::String(date))) => date.clone(),
                _ => match (
                    text.get("amount").and_then(Value::as_f64),
                    text.get("currency"),
                ) {
                    (Some(amount), Some(Value::String(currency))) if currency == "USD" => {
                        format!("${amount:.2}")
                    }
                    (Some(amount), Some(Value::String(currency))) => {
                        format!("{amount:.2} {currency}")
                    }
                    _ => value.to_string(),
                },
            },
            value => value.to_string(),
        };
        out = out.replace(&format!("{{{name}}}"), &value);
    }
    out
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
    let (configured, address) = match configured.map(|c| c.split_once(',')) {
        Some(Some((tag, address))) if address.trim().eq_ignore_ascii_case("formal") => {
            (Some(tag), Address::Formal)
        }
        Some(Some((tag, _))) => (Some(tag), Address::Informal),
        _ => (configured, Address::Informal),
    };
    let catalog_of = |tag: &str| {
        let language = primary(tag);
        let of = |formal| {
            let mut catalogs = CATALOGS.iter();
            catalogs.find(|c| c.language == language && c.formal == formal)
        };
        match address {
            Address::Formal => of(true).or_else(|| of(false)),
            Address::Informal => of(false),
        }
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
            return Locale {
                catalog,
                tag,
                address,
            };
        }
    }
    for tag in system {
        if let Some(catalog) = catalog_of(&tag) {
            return Locale {
                catalog,
                tag,
                address,
            };
        }
    }
    let catalog = catalog_of("en").expect("the catalogs have English");
    Locale {
        catalog,
        tag: "en".to_string(),
        address,
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
    fn the_configured_language_may_say_how_it_addresses_the_user() {
        let locale = choose(Some("de, informal"), &tags(&["de-AT"]));
        assert_eq!(
            (locale.tag.as_str(), locale.address),
            ("de-AT", Address::Informal)
        );
        assert!(!locale.catalog.formal);
        let locale = choose(Some("es,formal"), &[]);
        assert_eq!(
            (locale.catalog.language, locale.address),
            ("es", Address::Formal)
        );
        assert!(locale.catalog.formal);
        assert_eq!(choose(Some("de"), &[]).address, Address::Informal);
        let english = choose(Some("en, formal"), &[]);
        assert_eq!(
            (english.catalog.language, english.catalog.formal),
            ("en", false)
        );
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
        assert!(text("start.failed").starts_with("Avelor could not start."));
        let de = choose(Some("de"), &[]);
        let texts: Map<String, Value> = serde_json::from_str(de.catalog.json).unwrap();
        assert!(
            texts["start.failed"]
                .as_str()
                .unwrap()
                .starts_with("Avelor konnte nicht")
        );
    }
}
