//! The protocol between host and page (`api.md#transport`,
//! `api.md#rules`): a JSON envelope per request, exactly
//! one reply to each, events pushed.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::api;
use crate::app::Prefs;
use crate::app::{App, UiQueue};
use crate::calendar;
use crate::close;
use crate::documents;
use crate::folders;
use crate::google;
use crate::google_accounts::{CalendarSource, Level};
use crate::scans;
use crate::structure;
use crate::trash;
use crate::{accounts, ai};
use crate::{chat, vault_settings};
use crate::{jobs, mail};
use av_llm::Provider;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::id::Id;
use av_vault::change::Change;
use av_vault::vault::Index;
use av_vault::vault::NewEntity;
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
    /// The vault's own settings (`vault_settings`).
    SetVaultSettings(vault_settings::SettingsChange),
    RevealVault,
    /// Open a web, mail or phone link in its default app.
    OpenUrl(Url),
    /// Stop the job `id` (`jobs::cancel_job`).
    CancelJob(Target),
    /// The page's answer to the event `close` (`close::close_ready`).
    CloseReady(close::Answer),
    // A document's content in the editors (`documents`).
    LoadDocContent(Target),
    SaveDocContent(Save),
    CheckFiles(Ids),
    SetEditing(Ids),
    // The structural commands (`structure`).
    CreateEntity(Box<NewEntity>),
    CreateThread(NewThread),
    CreateProject(Title),
    JoinThread(InThread),
    LeaveThread(InThread),
    JoinProject(InProject),
    LeaveProject(InProject),
    SetTitle(Retitle),
    SetStarred(Star),
    SetArchived(Archive),
    SetTags(Retag),
    SetLink(Relink),
    EditEntities(EditEntities),
    DeleteEntity(Target),
    ReloadEntity(Target),
    // The trash (`trash`).
    ListTrash,
    RestoreEntity(TrashPath),
    DeleteTrashed(TrashPath),
    EmptyTrash,
    // Google accounts (`google`), and what both kinds of account do alike
    // (`accounts`).
    ConnectGoogle(Connect),
    CancelConnect,
    ListCalendars(OfAccount),
    SetCalendarSource(CalendarChoice),
    DisconnectAccount(OfAccount),
    RemoveAccount(OfAccount),
    // AI accounts (`ai`).
    ConnectAi(ConnectAi),
    ListAiModels(OfAccount),
    SetAiSettings(ai::SettingsChange),
    // Folder watches (`folders`).
    CreateWatch(folders::NewWatch),
    SetWatch(folders::WatchChange),
    RemoveWatch(OfWatch),
    PickFolder(PickFolder),
    // Import scans (`scans`).
    ScanList(OfWatch),
    // Fetch mail (`mail`).
    MailLabels(OfAccount),
    MailSearch(mail::SearchRequest),
    MailHeaders(mail::HeadersRequest),
    MailFetch(mail::FetchRequest),
    SetMailSync(mail::SyncChange),
    // Calendar sync (`calendar`).
    CalendarSync(calendar::SyncRequest),
    SetCalendarSync(calendar::SyncSetting),
    // The assistant's chats (`chat`).
    ChatOpen(Target),
    ChatSend(chat::SendRequest),
    ChatStop(OfConversation),
    ChatSelect(chat::SelectRequest),
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

/// `{id}`: the entity or job the command is about.
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

/// `{title, members}`
#[derive(Deserialize)]
struct NewThread {
    title: String,
    members: Vec<Id>,
}

/// `{title}`
#[derive(Deserialize)]
struct Title {
    title: String,
}

/// `{id, thread}`
#[derive(Deserialize)]
struct InThread {
    id: Id,
    thread: Id,
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

/// `{path}`: a file in the trash, relative to `.trash`.
#[derive(Deserialize)]
struct TrashPath {
    path: String,
}

/// `{id, to}`: `to` null removes the link.
#[derive(Deserialize)]
struct Relink {
    id: Id,
    to: Option<Id>,
}

/// `{ids, change}`: one change to every entity named.
#[derive(Deserialize)]
struct EditEntities {
    ids: Vec<Id>,
    change: Change,
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

/// `{account?, levels}`: no `account` connects a new one.
#[derive(Deserialize)]
struct Connect {
    #[serde(default)]
    account: Option<Id>,
    levels: BTreeMap<String, Level>,
}

/// `{account?, provider?, key?, address?}`: no `account` connects a new
/// one of `provider`.
#[derive(Deserialize)]
struct ConnectAi {
    #[serde(default)]
    account: Option<Id>,
    #[serde(default)]
    provider: Option<Provider>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    address: Option<String>,
}

/// `{conversation}`
#[derive(Deserialize)]
struct OfConversation {
    conversation: Id,
}

/// `{account}`
#[derive(Deserialize)]
struct OfAccount {
    account: Id,
}

/// `{account, calendar: {id, name, color}, selected}`
#[derive(Deserialize)]
struct CalendarChoice {
    account: Id,
    calendar: CalendarSource,
    selected: bool,
}

/// `{watch}`
#[derive(Deserialize)]
struct OfWatch {
    watch: Id,
}

/// `{title?}`: the folder picker's heading.
#[derive(Deserialize)]
struct PickFolder {
    #[serde(default)]
    title: Option<String>,
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
            "set_vault_settings" => Command::SetVaultSettings(from_value(args)?),
            "reveal_vault" => Command::RevealVault,
            "open_url" => Command::OpenUrl(from_value(args)?),
            "cancel_job" => Command::CancelJob(from_value(args)?),
            "close_ready" => Command::CloseReady(from_value(args)?),
            "load_doc_content" => Command::LoadDocContent(from_value(args)?),
            "save_doc_content" => Command::SaveDocContent(from_value(args)?),
            "check_files" => Command::CheckFiles(from_value(args)?),
            "set_editing" => Command::SetEditing(from_value(args)?),
            "create_entity" => Command::CreateEntity(from_value(args)?),
            "create_thread" => Command::CreateThread(from_value(args)?),
            "create_project" => Command::CreateProject(from_value(args)?),
            "join_thread" => Command::JoinThread(from_value(args)?),
            "leave_thread" => Command::LeaveThread(from_value(args)?),
            "join_project" => Command::JoinProject(from_value(args)?),
            "leave_project" => Command::LeaveProject(from_value(args)?),
            "set_title" => Command::SetTitle(from_value(args)?),
            "set_starred" => Command::SetStarred(from_value(args)?),
            "set_archived" => Command::SetArchived(from_value(args)?),
            "set_tags" => Command::SetTags(from_value(args)?),
            "set_link" => Command::SetLink(from_value(args)?),
            "edit_entities" => Command::EditEntities(from_value(args)?),
            "delete_entity" => Command::DeleteEntity(from_value(args)?),
            "reload_entity" => Command::ReloadEntity(from_value(args)?),
            "list_trash" => Command::ListTrash,
            "restore_entity" => Command::RestoreEntity(from_value(args)?),
            "delete_trashed" => Command::DeleteTrashed(from_value(args)?),
            "empty_trash" => Command::EmptyTrash,
            "connect_google" => Command::ConnectGoogle(from_value(args)?),
            "cancel_connect" => Command::CancelConnect,
            "list_calendars" => Command::ListCalendars(from_value(args)?),
            "set_calendar_source" => Command::SetCalendarSource(from_value(args)?),
            "disconnect_account" => Command::DisconnectAccount(from_value(args)?),
            "remove_account" => Command::RemoveAccount(from_value(args)?),
            "connect_ai" => Command::ConnectAi(from_value(args)?),
            "list_ai_models" => Command::ListAiModels(from_value(args)?),
            "set_ai_settings" => Command::SetAiSettings(from_value(args)?),
            "create_watch" => Command::CreateWatch(from_value(args)?),
            "set_watch" => Command::SetWatch(from_value(args)?),
            "remove_watch" => Command::RemoveWatch(from_value(args)?),
            "pick_folder" => Command::PickFolder(from_value(args)?),
            "scan_list" => Command::ScanList(from_value(args)?),
            "mail_labels" => Command::MailLabels(from_value(args)?),
            "mail_search" => Command::MailSearch(from_value(args)?),
            "mail_headers" => Command::MailHeaders(from_value(args)?),
            "mail_fetch" => Command::MailFetch(from_value(args)?),
            "set_mail_sync" => Command::SetMailSync(from_value(args)?),
            "calendar_sync" => Command::CalendarSync(from_value(args)?),
            "set_calendar_sync" => Command::SetCalendarSync(from_value(args)?),
            "chat_open" => Command::ChatOpen(from_value(args)?),
            "chat_send" => Command::ChatSend(from_value(args)?),
            "chat_stop" => Command::ChatStop(from_value(args)?),
            "chat_select" => Command::ChatSelect(from_value(args)?),
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
        Command::SetVaultSettings(a) => vault_settings::set_vault_settings(app, req, a),
        Command::RevealVault => api::reveal_vault(app, req),
        Command::OpenUrl(a) => api::open_url(app, req, a.url),
        Command::CancelJob(a) => jobs::cancel_job(app, req, a.id),
        Command::CloseReady(a) => close::close_ready(app, req, a),
        Command::LoadDocContent(a) => documents::load_doc_content(app, req, a.id),
        Command::SaveDocContent(a) => {
            documents::save_doc_content(app, req, a.id, a.markdown, a.overwrite)
        }
        Command::CheckFiles(a) => documents::check_files(app, req, a.ids),
        Command::SetEditing(a) => documents::set_editing(app, req, a.ids),
        Command::CreateEntity(a) => structure::create_entity(app, req, *a),
        Command::CreateThread(a) => structure::create_thread(app, req, a.title, a.members),
        Command::CreateProject(a) => structure::create_project(app, req, a.title),
        Command::JoinThread(a) => structure::membership(app, req, a.id, a.thread, S::join_thread),
        Command::LeaveThread(a) => structure::membership(app, req, a.id, a.thread, S::leave_thread),
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
        Command::EditEntities(a) => structure::edit_entities(app, req, a.ids, a.change),
        Command::DeleteEntity(a) => structure::delete_entity(app, req, a.id),
        Command::ReloadEntity(a) => structure::reload_entity(app, req, a.id),
        Command::ListTrash => trash::list_trash(app, req),
        Command::RestoreEntity(a) => trash::restore_entity(app, req, a.path),
        Command::DeleteTrashed(a) => trash::delete_trashed(app, req, a.path),
        Command::EmptyTrash => trash::empty_trash(app, req),
        Command::ConnectGoogle(a) => google::connect_google(app, req, a.account, a.levels),
        Command::CancelConnect => google::cancel_connect(app, req),
        Command::ListCalendars(a) => google::list_calendars(app, req, a.account),
        Command::SetCalendarSource(a) => {
            google::set_calendar_source(app, req, a.account, a.calendar, a.selected)
        }
        Command::DisconnectAccount(a) => accounts::disconnect_account(app, req, a.account),
        Command::RemoveAccount(a) => accounts::remove_account(app, req, a.account),
        Command::ConnectAi(a) => ai::connect_ai(app, req, a.account, a.provider, a.key, a.address),
        Command::ListAiModels(a) => ai::list_ai_models(app, req, a.account),
        Command::SetAiSettings(a) => ai::set_ai_settings(app, req, a),
        Command::CreateWatch(a) => folders::create_watch(app, req, a),
        Command::SetWatch(a) => folders::set_watch(app, req, a),
        Command::RemoveWatch(a) => folders::remove_watch(app, req, a.watch),
        Command::PickFolder(a) => folders::pick_folder(app, req, a.title),
        Command::ScanList(a) => scans::scan_list(app, req, a.watch),
        Command::MailLabels(a) => mail::mail_labels(app, req, a.account),
        Command::MailSearch(a) => mail::mail_search(app, req, a),
        Command::MailHeaders(a) => mail::mail_headers(app, req, a),
        Command::MailFetch(a) => mail::mail_fetch(app, req, a),
        Command::SetMailSync(a) => mail::set_mail_sync(app, req, a),
        Command::CalendarSync(a) => calendar::calendar_sync(app, req, a),
        Command::SetCalendarSync(a) => calendar::set_calendar_sync(app, req, a),
        Command::ChatOpen(a) => chat::chat_open(app, req, a.id),
        Command::ChatSend(a) => chat::chat_send(app, req, a),
        Command::ChatStop(a) => chat::chat_stop(app, req, a.conversation),
        Command::ChatSelect(a) => chat::chat_select(app, req, a),
    }
}

// ----------------------------------------------------------------------------
/// The answer to the page's request `id`, given exactly once: `ok`, `err`,
/// `fail` and `answer` consume it. Dropped unanswered -- its task aborted, a
/// branch that forgot it -- it answers `cancelled`, so every call on the
/// page settles. It is `Send`, to move into a worker's continuation: it
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
    /// error's own text: the host words what went wrong (e.g. "The key was
    /// rejected: ..."), the page still sees its code.
    pub fn fail(mut self, error: Error, message: impl Into<String>) {
        let err = json!({ "code": error.code(), "message": message.into() });
        let reply = json!({ "re": self.id, "err": err });
        self.send(reply);
    }

    // ------------------------------------------------------------------------
    /// Answer with `result`: its value (`ok`), or its error (`err`).
    pub fn answer<T: Serialize>(self, result: Result<T>) {
        match result {
            Ok(data) => self.ok(data),
            Err(error) => self.err(error),
        }
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
        h.app.req(2).answer(Ok(()));
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
