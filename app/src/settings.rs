//! What the Settings page sets (ui.md#settings), kept in config.yaml for
//! every vault: the language and how its texts address the user, and the
//! theme.

use av_utils::error::Error;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::config::Theme;
use crate::protocol::Req;
use crate::texts::{self, Address, Language, Text};

// ----------------------------------------------------------------------------
/// The language as the page gets it (`api::ready`).
#[derive(Serialize)]
pub struct LanguageMeta {
    /// The one picked for the next start, none for the system's, and whether
    /// its texts are to address the user formally.
    picked: Option<&'static str>,
    formal: bool,
    /// The run's.
    run: &'static str,
    run_formal: bool,
    /// The one the system's languages take.
    system: &'static str,
    all: Vec<Language>,
}

// ----------------------------------------------------------------------------
/// `set_language`'s arguments: a language of the catalogs, none for the
/// system's.
#[derive(Deserialize)]
pub struct LanguageChange {
    language: Option<String>,
    #[serde(default)]
    formal: bool,
}

// ----------------------------------------------------------------------------
pub fn language(app: &App) -> LanguageMeta {
    let (picked, address) = texts::picked(app.config.language());
    let run = texts::locale().catalog;
    LanguageMeta {
        picked,
        formal: address == Address::Formal,
        run: run.language,
        run_formal: run.formal,
        system: texts::system(),
        all: texts::languages(),
    }
}

// ----------------------------------------------------------------------------
/// `set_language {language, formal?}`: the language of the next start.
/// Answers `null`; a language without a catalog is `invalid_argument`.
pub fn set_language(app: &mut App, req: Req, change: LanguageChange) {
    let known = |id: &str| texts::languages().iter().any(|language| language.id == id);
    if change.language.as_deref().is_some_and(|id| !known(id)) {
        let arg = "language".into();
        return req.err(Error::InvalidArgument { arg });
    }
    let address = match change.formal {
        true => Address::Formal,
        false => Address::Informal,
    };
    let before = app.config.language();
    let language = texts::configured(before, change.language.as_deref(), address);
    match app.config.set_language(language) {
        Ok(()) => req.ok(()),
        Err(e) => req.save_failed(e, Text::new("save.settings")),
    }
}

// ----------------------------------------------------------------------------
/// `set_theme`'s arguments.
#[derive(Deserialize)]
pub struct ThemeChange {
    theme: Theme,
}

// ----------------------------------------------------------------------------
/// `set_theme {theme}`: the theme of the window's title bar, at once, and of
/// the next start; the page changes its own colors. Answers `null`.
pub fn set_theme(app: &mut App, req: Req, change: ThemeChange) {
    match app.config.set_theme(change.theme) {
        Ok(()) => {
            app.ui().window_theme(change.theme);
            req.ok(());
        }
        Err(e) => req.save_failed(e, Text::new("save.settings")),
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use crate::config::{Theme, load_config};
    use crate::testing::Harness;
    use serde_json::{Value, json};
    use std::fs;

    // ------------------------------------------------------------------------
    /// The language as `ready` says it.
    fn language(h: &mut Harness) -> Value {
        let sent = h.call("ready", json!({}));
        sent.last().unwrap()["ok"]["language"].clone()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_is_told_the_languages_and_how_they_address_the_user() {
        let language = language(&mut Harness::new("languages"));
        assert_eq!(language["picked"], json!(null));
        assert_eq!(language["formal"], json!(false));
        assert_eq!(language["system"], "en");
        assert_eq!(language["run"], "en");
        assert_eq!(language["run_formal"], json!(false));
        let all = language["all"].as_array().unwrap();
        let ids: Vec<_> = all.iter().map(|l| l["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["en", "de", "es"]);
        assert_eq!(all[0]["forms"], json!([]));
        let names = |at: usize| -> Vec<_> {
            let forms = all[at]["forms"].as_array().unwrap();
            forms.iter().map(|f| f["name"].as_str().unwrap()).collect()
        };
        assert_eq!(names(1), ["du", "Sie"]);
        assert_eq!(names(2), ["tú", "usted"]);
        let says = all[1]["forms"][1]["says"].as_str().unwrap();
        assert!(says.contains("Sie"), "{says}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_language_picked_is_the_next_start_s() {
        let mut h = Harness::new("set-language");
        let config = h.dir().join("config.yaml");
        fs::write(&config, "language: tlh\nzoom: 2\n").unwrap();
        h.app.config = load_config(h.dir()).unwrap();
        let picks = [
            (
                json!({ "language": "de", "formal": true }),
                Some("de, formal"),
            ),
            (json!({ "language": "es", "formal": false }), Some("es")),
            (
                json!({ "language": null, "formal": true }),
                Some("system, formal"),
            ),
            (json!({ "language": null }), None),
        ];
        for (args, stored) in picks {
            let sent = h.call("set_language", args.clone());
            assert_eq!(sent[0]["ok"], json!(null), "{sent:?}");
            let kept = load_config(h.dir()).unwrap();
            assert_eq!(kept.language(), stored, "{args}");

            let language = language(&mut h);
            assert_eq!(language["picked"], args["language"], "{args}");
            assert_eq!(language["formal"], json!(args["formal"] == json!(true)));
            assert_eq!(language["run"], "en", "this run's stays");
        }
        assert!(fs::read_to_string(&config).unwrap().contains("zoom: 2"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_language_without_a_catalog_isnt_picked() {
        let mut h = Harness::new("no-language");
        for language in ["tlh", "de-AT", "system", "", "de, formal"] {
            let sent = h.call("set_language", json!({ "language": language }));
            assert_eq!(sent[0]["err"]["code"], "invalid_argument", "{language}");
        }
        assert_eq!(load_config(h.dir()).unwrap().language(), None);
        let sent = h.call("set_language", json!({ "language": 4 }));
        assert_eq!(sent[0]["err"]["code"], "bad_request");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_theme_picked_is_the_window_s_and_kept() {
        let mut h = Harness::new("set-theme");
        let config = h.dir().join("config.yaml");
        fs::write(&config, "theme: sepia\nzoom: 2\n").unwrap();
        h.app.config = load_config(h.dir()).unwrap();
        assert_eq!(h.app.config.theme(), Theme::System, "none it knows");
        h.call("set_language", json!({ "language": "de" }));
        let kept = fs::read_to_string(&config).unwrap();
        assert!(kept.contains("theme: sepia"), "a save keeps it: {kept}");

        let themes = [("dark", Theme::Dark), ("light", Theme::Light)];
        for (name, theme) in themes {
            let sent = h.call("set_theme", json!({ "theme": name }));
            assert_eq!(sent[0]["ok"], json!(null), "{sent:?}");
            assert_eq!(h.ui.theme_told(), Some(theme));
            assert_eq!(load_config(h.dir()).unwrap().theme(), theme);
        }
        h.call("set_theme", json!({ "theme": "system" }));
        assert_eq!(h.ui.theme_told(), Some(Theme::System));
        let kept = fs::read_to_string(&config).unwrap();
        assert!(
            !kept.contains("theme") && kept.contains("zoom: 2"),
            "{kept}"
        );

        for theme in [json!("sepia"), json!("Dark"), json!(null)] {
            let sent = h.call("set_theme", json!({ "theme": theme }));
            assert_eq!(sent[0]["err"]["code"], "bad_request", "{theme}");
        }
        assert_eq!(h.ui.theme_told(), None);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_theme_that_can_t_be_saved_isnt_the_window_s() {
        let mut h = Harness::new("unsaved-theme");
        fs::create_dir(h.dir().join("config.yaml")).unwrap();
        let sent = h.call("set_theme", json!({ "theme": "dark" }));
        assert_eq!(sent[0]["err"]["text"]["key"], "save.settings");
        assert_eq!(h.ui.theme_told(), None);
    }
}
