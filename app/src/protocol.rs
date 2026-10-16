//! The protocol between host and page (`api.md#transport`,
//! `api.md#rules`): a JSON envelope per request, exactly
//! one reply to each, events pushed.

use std::path::PathBuf;
use std::sync::Arc;

use crate::api;
use crate::app::Prefs;
use crate::app::{App, UiQueue};
use crate::close;
use crate::documents;
use crate::structure;
use av_utils::error::Error;
use av_utils::id::Id;
use av_vault::vault::Index;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ----------------------------------------------------------------------------
/// A request from the page: `{"id": 17, "cmd": "open_url", "args": {…}}`.
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
    // The vault and the page's preferences (`api`).
    Ready,
    SwitchVault(SwitchTo),
    SetPrefs(PagePrefs),
    RevealVault,
    /// Open a web, mail or phone link in its default app.
    OpenUrl(Url),
    /// The page's answer to the event `close` (`close::close_ready`).
    CloseReady(close::Answer),
    // A document's content in the editors (`documents`).
    LoadDocContent(Target),
    SaveDocContent(Save),
    CheckFiles(Ids),
    // The structural commands (`structure`).
    CreateProject(Title),
    JoinProject(InProject),
    LeaveProject(InProject),
    SetTitle(Retitle),
    SetStarred(Star),
    SetArchived(Archive),
    SetTags(Retag),
    SetLink(Relink),
    ReloadEntity(Target),
}

/// `{path?}`: no `path` shows the folder picker.
#[derive(Deserialize)]
struct SwitchTo {
    #[serde(default)]
    path: Option<PathBuf>,
}

/// `{prefs}`: the page's preferences, strings.
#[derive(Deserialize)]
struct PagePrefs {
    prefs: Prefs,
}

/// `{url}`
#[derive(Deserialize)]
struct Url {
    url: String,
}

/// `{id}`: the entity the command is about.
#[derive(Deserialize)]
struct Target {
    id: Id,
}

/// `{id, markdown, overwrite?}`: `overwrite` writes over a file changed
/// outside the app (the conflict dialog's Use ours).
#[derive(Deserialize)]
struct Save {
    id: Id,
    markdown: String,
    #[serde(default)]
    overwrite: bool,
}

/// `{ids}`
#[derive(Deserialize)]
struct Ids {
    ids: Vec<Id>,
}

/// `{title}`
#[derive(Deserialize)]
struct Title {
    title: String,
}

/// `{id, project}`
#[derive(Deserialize)]
struct InProject {
    id: Id,
    project: Id,
}

/// `{id, title}`
#[derive(Deserialize)]
struct Retitle {
    id: Id,
    title: String,
}

/// `{id, tags}`
#[derive(Deserialize)]
struct Retag {
    id: Id,
    tags: Vec<String>,
}

/// `{id, to}`: `to` null removes the link.
#[derive(Deserialize)]
struct Relink {
    id: Id,
    to: Option<Id>,
}

/// `{id, starred}`
#[derive(Deserialize)]
struct Star {
    id: Id,
    starred: bool,
}

/// `{id, archived}`
#[derive(Deserialize)]
struct Archive {
    id: Id,
    archived: bool,
}

impl Command {
    // ------------------------------------------------------------------------
    /// The command `cmd` with its `args`.
    fn decode(cmd: &str, args: Value) -> serde_json::Result<Self> {
        use serde_json::from_value;
        Ok(match cmd {
            "ready" => Command::Ready,
            "switch_vault" => Command::SwitchVault(from_value(args)?),
            "set_prefs" => Command::SetPrefs(from_value(args)?),
            "reveal_vault" => Command::RevealVault,
            "open_url" => Command::OpenUrl(from_value(args)?),
            "close_ready" => Command::CloseReady(from_value(args)?),
            "load_doc_content" => Command::LoadDocContent(from_value(args)?),
            "save_doc_content" => Command::SaveDocContent(from_value(args)?),
            "check_files" => Command::CheckFiles(from_value(args)?),
            "create_project" => Command::CreateProject(from_value(args)?),
            "join_project" => Command::JoinProject(from_value(args)?),
            "leave_project" => Command::LeaveProject(from_value(args)?),
            "set_title" => Command::SetTitle(from_value(args)?),
            "set_starred" => Command::SetStarred(from_value(args)?),
            "set_archived" => Command::SetArchived(from_value(args)?),
            "set_tags" => Command::SetTags(from_value(args)?),
            "set_link" => Command::SetLink(from_value(args)?),
            "reload_entity" => Command::ReloadEntity(from_value(args)?),
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
    use Index as S;
    match command {
        Command::Ready => api::ready(app, req),
        Command::SwitchVault(a) => api::switch_vault(app, req, a.path),
        Command::SetPrefs(a) => api::set_prefs(app, req, a.prefs),
        Command::RevealVault => api::reveal_vault(app, req),
        Command::OpenUrl(a) => api::open_url(app, req, a.url),
        Command::CloseReady(a) => close::close_ready(app, req, a),
        Command::LoadDocContent(a) => documents::load_doc_content(app, req, a.id),
        Command::SaveDocContent(a) => {
            documents::save_doc_content(app, req, a.id, a.markdown, a.overwrite)
        }
        Command::CheckFiles(a) => documents::check_files(app, req, a.ids),
        Command::CreateProject(a) => structure::create_project(app, req, a.title),
        Command::JoinProject(a) => {
            structure::membership(app, req, a.id, a.project, S::join_project)
        }
        Command::LeaveProject(a) => {
            structure::membership(app, req, a.id, a.project, S::leave_project)
        }
        Command::SetTitle(a) => structure::set_title(app, req, a.id, a.title),
        Command::SetStarred(a) => structure::set_starred(app, req, a.id, a.starred),
        Command::SetArchived(a) => structure::set_archived(app, req, a.id, a.archived),
        Command::SetTags(a) => structure::set_tags(app, req, a.id, a.tags),
        Command::SetLink(a) => structure::set_link(app, req, a.id, a.to),
        Command::ReloadEntity(a) => structure::reload_entity(app, req, a.id),
    }
}

// ----------------------------------------------------------------------------
/// The answer to the page's request `id`, given exactly once: `ok`, `err`
/// and `fail` consume it. Dropped unanswered -- a branch that forgot it -- it
/// answers `cancelled`, so every call on the page settles. It is `Send`: it
/// holds the UI queue, never the web view, and its reply is posted on the
/// UI thread, after the events its handler posted.
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
    /// Answer with `error`, but with `message` to show instead of the
    /// error's own text: the host words what went wrong, the page still
    /// sees its code.
    pub fn fail(mut self, error: Error, message: impl Into<String>) {
        let err = json!({ "code": error.code(), "message": message.into() });
        let reply = json!({ "re": self.id, "err": err });
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
    use crate::app::UiTask;
    use crate::testing::{Harness, parse};

    // ------------------------------------------------------------------------
    fn error_code(reply: &Value, id: u64) -> &str {
        assert_eq!(reply["re"], id, "{reply}");
        reply["err"]["code"].as_str().unwrap()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn links_that_are_no_web_mail_or_phone_links_are_refused() {
        let mut h = Harness::new("refused");
        let urls = [
            "file:///C:/Windows/notepad.exe",
            "javascript:alert(1)",
            "C:/Windows/notepad.exe",
            "notes/a.md",
        ];
        for (id, url) in (1..).zip(urls) {
            let request = json!({ "id": id, "cmd": "open_url", "args": { "url": url } });
            let replies = parse(&h.send(&request.to_string()));
            assert_eq!(replies.len(), 1, "{url}");
            assert_eq!(error_code(&replies[0], id), "invalid_argument", "{url}");
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_web_link_is_opened_after_the_handler_and_answered_once() {
        let mut h = Harness::new("web-link");
        let request =
            r#"{"id": 7, "cmd": "open_url", "args": {"url": "HTTPS://example.com/a?b=1"}}"#;
        h.app.on_web_message(request.to_string());
        // The shell runs as a dialog does, after the handler: nothing is
        // opened or answered yet.
        let Some(UiTask::Modal(open)) = h.ui.pop() else {
            panic!("expected the link to be opened outside the handler");
        };
        assert!(h.take().is_empty());
        // Dropped without running (the app closed), it still answers.
        drop(open);
        h.run_queue();
        let replies = parse(&h.take());
        assert_eq!(error_code(&replies[0], 7), "cancelled");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_request_that_does_not_decode_is_answered_as_a_bad_request() {
        let mut h = Harness::new("bad-request");
        let requests = [
            (2, r#"{"id": 2, "cmd": "no_such_command"}"#),
            (3, r#"{"id": 3, "cmd": "open_url", "args": {}}"#),
            (4, r#"{"id": 4, "cmd": "open_url"}"#),
            (5, r#"{"id": 5, "cmd": "open_url", "args": {"url": 5}}"#),
        ];
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
            r#"{"cmd": "open_url"}"#,
            r#"{"id": "x", "cmd": "open_url"}"#,
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

    // ------------------------------------------------------------------------
    #[test]
    fn a_reply_follows_the_events_of_its_handler() {
        let mut h = Harness::new("order");
        let req = h.app.req(1);
        req.ok("done");
        h.app.post_event("entity", json!({ "id": "x" }));
        h.run_queue();
        let sent = parse(&h.take());
        assert_eq!(sent[0], json!({ "ev": "entity", "data": { "id": "x" } }));
        assert_eq!(sent[1], json!({ "re": 1, "ok": "done" }));
    }
}
