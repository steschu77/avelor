//! The app's own commands (`api.md#the-page-side`): `ready`.

use crate::app::App;
use crate::protocol::Req;

// ----------------------------------------------------------------------------
/// `ready`: the page is loaded and listens. Answers `null`.
pub fn ready(_app: &mut App, req: Req) {
    req.ok(());
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use crate::testing::Harness;
    use serde_json::json;

    // ------------------------------------------------------------------------
    #[test]
    fn ready_answers_at_once() {
        let mut h = Harness::new("ready");
        let sent = h.call("ready", json!({}));
        assert_eq!(sent, [json!({ "re": 1, "ok": null })]);
    }
}
