//! The protocol between host and page (`api.md#transport`,
//! `api.md#rules`): a JSON envelope per request, exactly
//! one reply to each.

use std::sync::Arc;

use crate::api;
use crate::app::{App, UiQueue};
use av_utils::error::Error;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ----------------------------------------------------------------------------
/// A request from the page: `{"id": 17, "cmd": "ready", "args": {…}}`.
/// Read before its command, so a request whose command or arguments don't
/// decode still has an `id` to answer.
#[derive(Deserialize)]
struct Envelope {
    id: u64,
    cmd: String,
    #[serde(default)]
    args: Value,
}

// ----------------------------------------------------------------------------
/// The commands of the typed protocol: one variant per command, holding the
/// command's `args`. Decoded by hand (`decode`): serde's derive for an
/// adjacently tagged enum grows by kilobytes per variant.
enum Command {
    Ready,
}

impl Command {
    // ------------------------------------------------------------------------
    /// The command `cmd` with its `args`.
    fn decode(cmd: &str, _args: Value) -> serde_json::Result<Self> {
        Ok(match cmd {
            "ready" => Command::Ready,
            _ => {
                let unknown = format!("unknown command `{cmd}`");
                return Err(serde::de::Error::custom(unknown));
            }
        })
    }
}

// ----------------------------------------------------------------------------
/// A message of the typed protocol arrived from the page. One that isn't a
/// request can't be answered, and is only logged.
pub fn on_message(app: &mut App, text: &str) {
    let envelope = match serde_json::from_str::<Envelope>(text) {
        Ok(envelope) => envelope,
        Err(e) => return log::warn!("A message from the page isn't a request: {e}"),
    };
    let req = app.req(envelope.id);
    match Command::decode(&envelope.cmd, envelope.args) {
        Ok(command) => route(app, req, command),
        Err(e) => req.err(Error::BadRequest { msg: e.to_string() }),
    }
}

// ----------------------------------------------------------------------------
fn route(app: &mut App, req: Req, command: Command) {
    match command {
        Command::Ready => api::ready(app, req),
    }
}

// ----------------------------------------------------------------------------
/// The answer to the page's request `id`, given exactly once: `ok` and
/// `err` consume it. Dropped unanswered -- a branch that forgot it -- it
/// answers `cancelled`, so every call on the page settles. It is `Send`: it
/// holds the UI queue, never the web view, and its reply is posted on the
/// UI thread.
pub struct Req {
    id: u64,
    ui: Arc<UiQueue>,
    answered: bool,
}

impl Req {
    // ------------------------------------------------------------------------
    pub fn new(id: u64, ui: Arc<UiQueue>) -> Self {
        Self {
            id,
            ui,
            answered: false,
        }
    }

    // ------------------------------------------------------------------------
    /// Answer with `data`: `{"re": id, "ok": data}`.
    pub fn ok(mut self, data: impl Serialize) {
        let reply = match serde_json::to_value(data) {
            Ok(data) => json!({ "re": self.id, "ok": data }),
            Err(e) => error_reply(self.id, &Error::from(e)),
        };
        self.send(reply);
    }

    // ------------------------------------------------------------------------
    /// Answer with `error`: `{"re": id, "err": {"code", "message"}}`.
    pub fn err(mut self, error: Error) {
        let reply = error_reply(self.id, &error);
        self.send(reply);
    }

    // ------------------------------------------------------------------------
    fn send(&mut self, reply: Value) {
        self.answered = true;
        let text = reply.to_string();
        self.ui.send_to_ui(move |app| app.post_text(&text));
    }
}

impl Drop for Req {
    fn drop(&mut self) {
        if !self.answered {
            let reply = error_reply(self.id, &Error::Cancelled);
            self.send(reply);
        }
    }
}

// ----------------------------------------------------------------------------
fn error_reply(id: u64, error: &Error) -> Value {
    let err = json!({ "code": error.code(), "message": error.to_string() });
    json!({ "re": id, "err": err })
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, parse};

    // ------------------------------------------------------------------------
    fn error_code(reply: &Value, id: u64) -> &str {
        assert_eq!(reply["re"], id, "{reply}");
        reply["err"]["code"].as_str().unwrap()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_request_that_does_not_decode_is_answered_as_a_bad_request() {
        let mut h = Harness::new("bad-request");
        let requests = [(2, r#"{"id": 2, "cmd": "no_such_command"}"#)];
        for (id, request) in requests {
            let replies = parse(&h.send(request));
            assert_eq!(replies.len(), 1, "{request}");
            assert_eq!(error_code(&replies[0], id), "bad_request", "{request}");
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_message_without_an_id_is_not_answered() {
        let mut h = Harness::new("no-id");
        for message in [
            r#"{"cmd": "ready"}"#,
            r#"{"id": "x", "cmd": "ready"}"#,
            "{oops",
        ] {
            assert!(h.send(message).is_empty(), "{message}");
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_req_answers_once_and_cancelled_when_dropped_anywhere() {
        let mut h = Harness::new("req");
        h.app.req(1).ok(json!({ "a": 1 }));
        h.app.req(2).ok(());
        let req = h.app.req(3);
        std::thread::spawn(move || drop(req)).join().unwrap();
        h.run_queue();
        let replies = parse(&h.take());
        assert_eq!(replies[0], json!({ "re": 1, "ok": { "a": 1 } }));
        assert_eq!(replies[1], json!({ "re": 2, "ok": null }));
        assert_eq!(error_code(&replies[2], 3), "cancelled");
        assert_eq!(replies.len(), 3);
    }
}
