//! The app's own commands (`api.md#the-page-side`): `ready` and the page's
//! preferences.

use crate::app::App;
use crate::app::Prefs;
use crate::protocol::Req;

// ----------------------------------------------------------------------------
/// `ready`: the page is loaded and listens. A config that couldn't be read
/// at start, and was set aside (`config::load_config`, which logs why), is
/// told now, in the status bar. Answers `null`.
pub fn ready(app: &mut App, req: Req) {
    if app.config.take_error().is_some() {
        app.notice(CONFIG_UNREAD);
    }
    req.ok(());
}

/// The notice of a config that couldn't be read at start.
const CONFIG_UNREAD: &str =
    "config.yaml couldn't be read; the app started with the default settings.";

// ----------------------------------------------------------------------------
/// `set_prefs {prefs}`: store the page's preferences (strings) as given; the
/// host never reads them, and fills them into the page it loads next
/// (`page::html`). Answers `null`, or the failed save, whose error is logged.
pub fn set_prefs(app: &mut App, req: Req, prefs: Prefs) {
    match app.config.set_prefs(prefs) {
        Ok(()) => req.ok(()),
        Err(e) => {
            log::warn!("Could not save the settings: {e}");
            req.fail(e, "Could not save the settings.");
        }
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::testing::Harness;
    use serde_json::json;

    // ------------------------------------------------------------------------
    #[test]
    fn ready_answers_at_once() {
        let mut h = Harness::new("ready");
        let sent = h.call("ready", json!({}));
        assert_eq!(sent, [json!({ "re": 1, "ok": null })]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_config_that_could_not_be_read_is_told_once_the_page_listens() {
        let mut h = Harness::new("unread");
        std::fs::write(h.dir().join("config.yaml"), "window: [unclosed\n").unwrap();
        h.app.config = load_config(h.dir()).unwrap();

        let sent = h.call("ready", json!({}));
        let notice = json!({ "level": "error", "message": CONFIG_UNREAD });
        let ready = json!({ "re": 1, "ok": null });
        assert_eq!(
            sent,
            [json!({ "ev": "notice", "data": notice }), ready.clone()]
        );
        assert_eq!(h.call("ready", json!({})), [ready], "told once");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_pages_preferences_are_stored_as_given() {
        let mut h = Harness::new("prefs");
        let prefs = json!({ "panel_width": "420", "sidebar_width": "240" });
        let sent = h.call("set_prefs", json!({ "prefs": prefs }));
        assert_eq!(sent, [json!({ "re": 1, "ok": null })]);
        let stored = load_config(h.dir()).unwrap();
        assert_eq!(json!(stored.prefs()), prefs);

        let sent = h.call("set_prefs", json!({ "prefs": { "panel_width": 420 } }));
        assert_eq!(sent[0]["err"]["code"], "bad_request", "strings only");
    }
}
