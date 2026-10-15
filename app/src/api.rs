//! The app's own commands (`api.md#the-page-side`): `ready` and the page's
//! preferences.

use crate::app::App;
use crate::app::Prefs;
use crate::protocol::Req;

// ----------------------------------------------------------------------------
/// `ready`: the page is loaded and listens. Answers `null`.
pub fn ready(_app: &mut App, req: Req) {
    req.ok(());
}

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
