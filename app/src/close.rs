//! Closing the app (`saving.md#closing-the-app`): the window's Close asks the
//! page first, with the event `close {id}`. The page leaves edit mode in every
//! editor, saving as it does, and once every save is answered it says whether
//! it is ready (`close_ready`). Not ready -- a save found its file changed
//! outside the app, and the conflict dialog asks -- the window stays, and
//! closing again goes on once that is answered. A page that doesn't answer
//! within `CLOSE_WAIT` is hung or broken and can't save anything: the window
//! closes then. The app closes the window through the UI queue
//! (`UiQueue::close_window`), never from a worker.

use std::time::Duration;

use av_utils::id::Id;
use serde::Deserialize;
use serde_json::json;

use crate::app::App;
use crate::protocol::Req;

// ----------------------------------------------------------------------------
/// How long the page has to answer a close.
pub const CLOSE_WAIT: Duration = Duration::from_secs(4);

// ----------------------------------------------------------------------------
/// The close asked of the page (`App::close`).
#[derive(Default)]
pub struct CloseState {
    /// The close asked (the event `close`), until the page answers or its
    /// wait ends.
    asked: Option<Id>,
}

// ----------------------------------------------------------------------------
/// `close_ready`'s args: the close asked, and whether the page is ready.
#[derive(Deserialize)]
pub(crate) struct Answer {
    id: Id,
    ready: bool,
}

// ----------------------------------------------------------------------------
/// The window's Close: ask the page, unless a close is asked already -- its
/// answer or its wait ends that one.
pub fn on_close(app: &mut App) {
    if let Some(id) = app.close.asked {
        return log::info!("Close {id}: asked already");
    }
    let id = Id::generate();
    log::info!("Close {id}: asking the page");
    app.close.asked = Some(id);
    app.post_event("close", json!({ "id": id }));
    app.run(
        |_| std::thread::sleep(CLOSE_WAIT),
        move |app, ()| waited(app, id),
    );
}

// ----------------------------------------------------------------------------
/// `close_ready {id, ready}`: the page's answer to the close `id`, once
/// every save it sent is answered. Ready, the window closes; not ready,
/// it stays. An answer to another close is too late and changes nothing.
/// Answered `null` either way.
pub fn close_ready(app: &mut App, req: Req, answer: Answer) {
    let Answer { id, ready } = answer;
    if app.close.asked != Some(id) {
        log::info!("Close {id}: answered too late");
        return req.ok(());
    }
    app.close.asked = None;
    req.ok(());
    match ready {
        true => {
            log::info!("Close {id}: the page is ready");
            app.ui().close_window();
        }
        false => log::info!("Close {id}: not ready, a file changed outside the app"),
    }
}

// ----------------------------------------------------------------------------
/// The page had `CLOSE_WAIT` to answer the close `id`: if it hasn't, the
/// window closes anyway.
fn waited(app: &mut App, id: Id) {
    if app.close.asked != Some(id) {
        return;
    }
    let secs = CLOSE_WAIT.as_secs();
    log::warn!("Close {id}: the page didn't answer in {secs} s, closing");
    app.close.asked = None;
    app.ui().close_window();
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, parse, shape};
    use serde_json::Value;

    // ------------------------------------------------------------------------
    /// Close as the window's Close does; the id the page was asked with.
    fn close(h: &mut Harness) -> Value {
        on_close(&mut h.app);
        h.run_queue();
        let sent = parse(&h.take());
        assert_eq!(shape(&sent), ["close"]);
        sent[0]["data"]["id"].clone()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn close_asks_the_page_and_closes_once_it_is_ready() {
        let mut h = Harness::new("close-ready");
        let id = close(&mut h);
        assert!(!h.ui.window_closed(), "not before the page answered");
        let sent = h.call("close_ready", json!({ "id": id, "ready": true }));
        assert_eq!(sent[0]["ok"], Value::Null);
        assert!(h.ui.window_closed());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_page_not_ready_keeps_the_window_until_closed_again() {
        let mut h = Harness::new("close-not-ready");
        let first = close(&mut h);
        let sent = h.call("close_ready", json!({ "id": first, "ready": false }));
        assert_eq!(sent[0]["ok"], Value::Null);
        assert!(!h.ui.window_closed());

        let second = close(&mut h);
        assert_ne!(first, second, "asked again");
        h.call("close_ready", json!({ "id": first, "ready": true }));
        assert!(!h.ui.window_closed(), "the first answer is too late");
        h.call("close_ready", json!({ "id": second, "ready": true }));
        assert!(h.ui.window_closed());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_page_that_doesnt_answer_is_closed_after_the_wait() {
        let mut h = Harness::new("close-waited");
        let id: Id = serde_json::from_value(close(&mut h)).unwrap();
        on_close(&mut h.app);
        h.run_queue();
        assert!(h.take().is_empty(), "asked once");

        waited(&mut h.app, Id::generate());
        assert!(!h.ui.window_closed(), "another close's wait");
        waited(&mut h.app, id);
        assert!(h.ui.window_closed());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_answer_without_its_close_is_a_bad_request() {
        let mut h = Harness::new("close-bad-request");
        close(&mut h);
        let sent = h.call("close_ready", json!({ "ready": true }));
        assert_eq!(sent[0]["err"]["code"], "bad_request");
        assert!(!h.ui.window_closed());
    }
}
