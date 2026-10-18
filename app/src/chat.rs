//! The assistant's chats (assistant.md, vault.md#conversation): each is a
//! conversation entity in the vault's `Conversations/`, linked to the document
//! or thread the panel showed when it started. The reply being written and the
//! failed replies live here only; a reply is written once it is complete or
//! stopped. The tree's rules are av-vault's (`conversation`).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use av_llm::{ChatRequest, ChatResponse, Client, Message, Provider, Role, StopReason};
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::{Conversation, ConversationMessage, Empty, Roles, Source};
use av_vault::markdown;
use av_vault::schema::layout;
use serde::{Deserialize, Serialize};

use crate::ai;
use crate::ai_accounts::Resolved;
use crate::ai_context;
use crate::app::{App, Task};
use crate::protocol::Req;
use av_vault::markdown::plain_title;

/// Titles are cut to this many characters, at a word boundary.
const TITLE_LEN: usize = 60;

const SYSTEM: &str = "You are the assistant in Avelor, the user's \
personal document manager. Answer in Markdown and keep it short unless asked \
for more.";

/// The user's turn before a conversation that starts with a reply (the mail
/// analysis's chat): providers want a request to start with the user, and
/// what it is about is in the system text.
const OPENING: &str = "Have a look at it.";

// ----------------------------------------------------------------------------
/// A reply being written.
struct Writing {
    id: Id,
    /// The user message it answers.
    parent: Id,
    account: Id,
    model: String,
    date: DateTime,
    /// The text so far.
    text: String,
    /// `None` until its worker started: `chat_send` answers first.
    task: Option<Task>,
}

// ----------------------------------------------------------------------------
/// A reply that failed: shown until the app closes, never written.
struct Failed {
    id: Id,
    parent: Id,
    model: String,
    date: DateTime,
    error: String,
}

// ----------------------------------------------------------------------------
/// What isn't in the vault: per conversation, the reply being written (one
/// at a time) and the failed replies; and the replies that were stopped.
#[derive(Default)]
pub struct ChatState {
    writing: HashMap<Id, Writing>,
    failed: HashMap<Id, Vec<Failed>>,
    stopped: HashSet<Id>,
}

// ----------------------------------------------------------------------------
/// `chat_send`'s args: a new or edited turn (`text`, after `parent`), or a
/// new reply to the user message `retry`, in `conversation` -- none starts
/// one, linked to `linked_to`. The host picks the account and model
/// (`AiAccounts::choose`).
#[derive(Deserialize)]
pub(crate) struct SendRequest {
    #[serde(default)]
    conversation: Option<Id>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    parent: Option<Id>,
    #[serde(default)]
    linked_to: Option<Id>,
    #[serde(default)]
    retry: Option<Id>,
}

// ----------------------------------------------------------------------------
/// `chat_select`'s args.
#[derive(Deserialize)]
pub(crate) struct SelectRequest {
    conversation: Id,
    message: Id,
}

// ----------------------------------------------------------------------------
/// A conversation as the page shows it: the `conversation` event, and in
/// the answers of `chat_open` and `chat_send`.
#[derive(Serialize)]
struct ConversationView<'a> {
    id: Id,
    title: &'a str,
    linked_to: Option<Id>,
    active: Option<Id>,
    messages: Vec<MessageView>,
}

#[derive(Serialize)]
struct MessageView {
    id: Id,
    parent: Option<Id>,
    /// `user` or `assistant`.
    role: &'static str,
    model: Option<String>,
    date: Option<DateTime>,
    content: String,
    html: String,
    /// `done`, `writing`, `stopped`, or `failed`.
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// Questions offered to ask next (vault.md#conversation).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suggestions: Vec<String>,
}

/// `chat_open`'s answer.
#[derive(Serialize)]
struct Opened<'a> {
    conversation: ConversationView<'a>,
}

/// `chat_send`'s answer.
#[derive(Serialize)]
struct Sent<'a> {
    id: Id,
    conversation: ConversationView<'a>,
}

/// The `ai_delta` event: the reply `message` so far, rendered.
#[derive(Serialize)]
struct Delta {
    conversation: Id,
    message: Id,
    html: String,
}

// ----------------------------------------------------------------------------
/// A reply ready to be written: what its worker needs.
struct Start {
    reply: Id,
    request: ChatRequest,
    client: Client,
    provider: Provider,
    address: String,
}

// ----------------------------------------------------------------------------
/// Refuse a request that does nothing, with `message` for the page.
fn refuse(req: Req, error: Error, message: &str) {
    log::info!("Chat request refused: {message}");
    req.fail(error, message);
}

// ----------------------------------------------------------------------------
/// Answer a conversation that couldn't be written.
fn save_failed(req: Req, error: Error) {
    log::warn!("Could not save the conversation: {error}");
    req.fail(error, "Could not save the conversation.");
}

// ----------------------------------------------------------------------------
/// `chat_open {id}`: the conversation `id` as the page shows it
/// (`ok {conversation}`); `not_found` if there is none, `wrong_kind` if
/// `id` is no conversation.
pub fn chat_open(app: &mut App, req: Req, id: Id) {
    match view(app, id) {
        Ok(conversation) => req.ok(Opened { conversation }),
        Err(e) => req.err(e),
    }
}

// ----------------------------------------------------------------------------
/// `chat_send {conversation?, text?, parent?, linked_to?, retry?}`: write
/// the user's message (creating the conversation if there is none), then
/// start the reply with the model picked for chats (the vault's settings,
/// accounts.md#model-choice). Answers
/// `ok {id, conversation}`, the reply being written, before the reply's
/// worker starts, so the page knows the conversation before its first
/// `ai_delta`. Refused in the host's words if nothing could be done --
/// also when the picked model can't answer (none picked, its account gone
/// or without its key, no longer offered, the spend limit), before anything
/// is written.
pub fn chat_send(app: &mut App, req: Req, request: SendRequest) {
    let existing = match request.conversation {
        None => None,
        Some(id) => match conversation_of(app, &id) {
            Ok(_) => Some(id),
            Err(e) => return refuse(req, e, "The conversation is gone."),
        },
    };
    if existing.is_some_and(|id| app.chat.writing.contains_key(&id)) {
        let what = "a second reply at once".to_string();
        let message = "A reply is still being written.";
        return refuse(req, Error::Unsupported { what }, message);
    }
    let picked = app.settings.chat_model();
    let resolved = match app.ai.choose(picked, "chats", DateTime::now()) {
        Ok(resolved) => resolved,
        Err(refusal) => return refuse(req, refusal.error, &refusal.message),
    };

    let (id, question) = match request.retry {
        // Answering again: the question becomes the shown path's end.
        Some(retry) => {
            let Some(id) = existing else {
                let arg = "conversation".to_string();
                let message = "The conversation to answer in is missing.";
                return refuse(req, Error::InvalidArgument { arg }, message);
            };
            match set_active(app, id, retry, true) {
                Ok(()) => (id, retry),
                // Gone, or a reply rather than the user's own message.
                Err(e @ (Error::NotFound | Error::WrongKind)) => {
                    return refuse(req, e, "The message is gone.");
                }
                Err(e) => return save_failed(req, e),
            }
        }
        None => {
            let text = request.text.as_deref().unwrap_or_default().trim();
            if text.is_empty() {
                let arg = "text".to_string();
                return refuse(req, Error::InvalidArgument { arg }, "The message is empty.");
            }
            let question = Id::generate();
            let message = ConversationMessage {
                id: Some(question),
                parent: request.parent,
                date: Some(DateTime::now()),
                content: Some(text.to_string()),
                ..Default::default()
            };
            let linked_to = request
                .linked_to
                .filter(|id| app.vault.index().entry(id).is_some());
            let written = match existing {
                Some(id) => add_message(app, id, message, true).map(|()| id),
                None => create(app, title_for(text), vec![message], linked_to),
            };
            match written {
                Ok(id) => (id, question),
                Err(e @ (Error::NotFound | Error::InvalidArgument { .. })) => {
                    let message = "The message it follows is gone.";
                    return refuse(req, e, message);
                }
                Err(e) => return save_failed(req, e),
            }
        }
    };
    log::info!("Chat {id}: answering {question} with {}", resolved.model);
    let start = prepare_reply(app, id, question, resolved);
    match view(app, id) {
        Ok(conversation) => req.ok(Sent { id, conversation }),
        Err(e) => req.err(e),
    }
    if let Some(start) = start {
        start_reply(app, id, start);
    }
}

// ----------------------------------------------------------------------------
/// `chat_stop {conversation}`: stop the reply being written; what it wrote
/// so far is kept as the reply. Answered after the `conversation` event,
/// and also when no reply is being written.
pub fn chat_stop(app: &mut App, req: Req, id: Id) {
    stop_reply(app, id);
    req.ok(());
}

// ----------------------------------------------------------------------------
/// Stop every reply being written, as `chat_stop` does: before another
/// vault opens, so they are written to the vault they belong to.
pub fn stop_all(app: &mut App) {
    let ids: Vec<Id> = app.chat.writing.keys().copied().collect();
    for id in ids {
        stop_reply(app, id);
    }
}

// ----------------------------------------------------------------------------
/// Stop the reply being written in the conversation `id`; what it wrote so
/// far is kept as the reply.
fn stop_reply(app: &mut App, id: Id) {
    let Some(writing) = app.chat.writing.remove(&id) else {
        return;
    };
    if let Some(task) = &writing.task {
        task.abort();
    }
    log::info!("Chat {id}: reply {} stopped", writing.id);
    if !writing.text.trim().is_empty() {
        app.chat.stopped.insert(writing.id);
        let message = reply_message(&writing, writing.text.clone());
        if let Err(e) = add_message(app, id, message, false) {
            let failed = Failed {
                id: writing.id,
                parent: writing.parent,
                model: writing.model,
                date: writing.date,
                error: format!("Could not save the reply: {e}"),
            };
            record_failed(app, id, failed);
        }
    }
    post_conversation(app, id);
}

// ----------------------------------------------------------------------------
/// `chat_select {conversation, message}`: show the path through `message`,
/// down to its newest leaf, and write that leaf as `active`. Answered after
/// the `conversation` event; `not_found`, `wrong_kind`, or the failed
/// write.
pub fn chat_select(app: &mut App, req: Req, request: SelectRequest) {
    let id = request.conversation;
    let conversation = match conversation_of(app, &id) {
        Ok((_, conversation)) => conversation,
        Err(e) => return req.err(e),
    };
    // A failed reply isn't in the file: its question stands in for it.
    let failed_parent = app
        .chat
        .failed
        .get(&id)
        .and_then(|failed| failed.iter().find(|f| f.id == request.message))
        .map(|f| f.parent);
    let Some(leaf) = conversation.leaf_from(failed_parent.unwrap_or(request.message)) else {
        return req.err(Error::NotFound);
    };
    if let Err(e) = set_active(app, id, leaf, false) {
        log::warn!("Chat {id}: could not save the branch: {e}");
        return req.fail(e, "Could not save the branch.");
    }
    post_conversation(app, id);
    req.ok(());
}

// ----------------------------------------------------------------------------
/// `id`'s document and its `conversation` block: `NotFound` if there is no
/// `id`, `WrongKind` if it is no conversation.
fn conversation_of<'a>(app: &'a App, id: &Id) -> Result<(&'a Document, &'a Conversation)> {
    let doc = &app.vault.index().entry(id).ok_or(Error::NotFound)?.doc;
    let conversation = doc.roles().conversation.as_ref().ok_or(Error::WrongKind)?;
    Ok((doc, conversation))
}

// ----------------------------------------------------------------------------
/// Write `conversation` as `id`'s block, sending what changed.
fn save(app: &mut App, id: Id, conversation: Conversation) -> Result<()> {
    let outcome =
        app.edit(|index, changes| index.set_conversation(&id, Some(conversation), changes))?;
    app.vault.require_written(outcome)
}

// ----------------------------------------------------------------------------
/// Write a new AI conversation titled `title`, of `messages`, `linked-to`
/// `linked_to`: the first starts it (no parent), each later one follows a
/// message before it (`Conversation::add`), and the last is `active`.
/// `Error::InvalidArgument` if there is none or the first has a parent,
/// `Error::NotFound` if a later one's parent is missing. A chat's first
/// message, and the mail analysis's chat (`enrich`).
pub fn create(
    app: &mut App,
    title: String,
    messages: Vec<ConversationMessage>,
    linked_to: Option<Id>,
) -> Result<Id> {
    if messages.first().is_none_or(|m| m.parent.is_some()) {
        let arg = "parent".to_string();
        return Err(Error::InvalidArgument { arg });
    }
    let mut conversation = Conversation::default();
    for message in messages {
        conversation.add(message, true)?;
    }
    let roles = Roles {
        conversation: Some(conversation),
        ..Default::default()
    };
    let mut doc = Document::create(
        roles,
        Source::Ai(Empty {}),
        title,
        Vec::new(),
        FileFormat::YAML,
    );
    doc.set_link(linked_to);
    app.edit(|index, changes| index.create(doc, Path::new(layout::CONVERSATIONS), changes))
}

// ----------------------------------------------------------------------------
/// Add `message` to the conversation `id` and write it
/// (`Conversation::add`). `Error::NotFound` if the conversation or the
/// message's parent is gone.
fn add_message(app: &mut App, id: Id, message: ConversationMessage, activate: bool) -> Result<()> {
    let mut conversation = conversation_of(app, &id)?.1.clone();
    conversation.add(message, activate)?;
    save(app, id, conversation)
}

// ----------------------------------------------------------------------------
/// Make `message` the conversation's `active` and write it, unless it
/// already is (`Conversation::activate`).
fn set_active(app: &mut App, id: Id, message: Id, user_only: bool) -> Result<()> {
    let mut conversation = conversation_of(app, &id)?.1.clone();
    if conversation.activate(message, user_only)? {
        save(app, id, conversation)?;
    }
    Ok(())
}

// ----------------------------------------------------------------------------
/// Get the reply to `question` ready, with the account and model
/// `resolved`: shown as being written from now on, and what its worker
/// needs. The request tells the model about the item the conversation is
/// linked to, as it is now (`ai_context`). `None` if the conversation is
/// gone.
fn prepare_reply(app: &mut App, id: Id, question: Id, resolved: Resolved) -> Option<Start> {
    let entry = app.vault.index().entry(&id)?;
    let conversation = entry.doc.roles().conversation.as_ref()?;
    let item = entry.doc.linked_to();
    let context = item.and_then(|item| ai_context::describe(app.vault.index(), item));
    let today = DateTime::today();
    let request = request_for(
        conversation,
        question,
        &resolved.model,
        today,
        context.as_deref(),
    );
    let reply = Id::generate();
    let writing = Writing {
        id: reply,
        parent: question,
        account: resolved.account,
        model: resolved.model,
        date: DateTime::now(),
        text: String::new(),
        task: None,
    };
    app.chat.writing.insert(id, writing);
    Some(Start {
        reply,
        request,
        client: resolved.client,
        provider: resolved.provider,
        address: resolved.address,
    })
}

// ----------------------------------------------------------------------------
/// Start the worker writing the reply of `start` in the conversation `id`.
/// It sends the text so far as it streams in, coalesced
/// (`Progress::send_latest`): at most one `ai_delta` per run of the UI
/// queue.
fn start_reply(app: &mut App, id: Id, start: Start) {
    let Start {
        reply,
        request,
        client,
        provider,
        address,
    } = start;
    let task = app.run_with_progress(
        move |progress, cancel| {
            let mut text = String::new();
            client.chat_stream(cancel, &request, |delta| {
                text.push_str(delta);
                progress.send_latest(text.clone());
            })
        },
        move |app, text: String| on_delta(app, id, reply, text),
        move |app, result| on_done(app, id, reply, provider, &address, result),
    );
    // The continuations run on this (the UI) thread, so they can't look
    // before this.
    if let Some(writing) = app.chat.writing.get_mut(&id).filter(|w| w.id == reply) {
        writing.task = Some(task);
    }
}

// ----------------------------------------------------------------------------
/// The reply `reply` so far: send it, rendered.
fn on_delta(app: &mut App, id: Id, reply: Id, text: String) {
    let Some(writing) = app.chat.writing.get_mut(&id).filter(|w| w.id == reply) else {
        return; // stopped meanwhile
    };
    let html = markdown::to_html(&text);
    writing.text = text;
    let delta = Delta {
        conversation: id,
        message: reply,
        html,
    };
    app.post_event("ai_delta", delta);
}

// ----------------------------------------------------------------------------
/// The reply `reply` is complete or failed: write it, or record the
/// failure; then send the conversation and the account's spending.
fn on_done(
    app: &mut App,
    id: Id,
    reply: Id,
    provider: Provider,
    address: &str,
    result: Result<ChatResponse>,
) {
    if app.chat.writing.get(&id).is_none_or(|w| w.id != reply) {
        return;
    }
    let Some(writing) = app.chat.writing.remove(&id) else {
        return;
    };
    let text = result.map(|resp| {
        let empty = resp.text.trim().is_empty();
        match resp.stop {
            _ if !empty => Ok(resp.text),
            StopReason::Refused => Err("The model declined to answer.".to_string()),
            _ => Err("The answer was empty.".to_string()),
        }
    });
    let outcome = match text {
        Ok(Ok(text)) => add_message(app, id, reply_message(&writing, text), false)
            .map_err(|e| format!("Could not save the reply: {e}")),
        Ok(Err(problem)) => Err(problem),
        Err(e) => Err(ai::chat_error(provider, address, &e)),
    };
    match outcome {
        Ok(()) => log::info!("Chat {id}: reply {reply} written"),
        Err(error) => {
            log::info!("Chat {id}: reply {reply} failed: {error}");
            let failed = Failed {
                id: reply,
                parent: writing.parent,
                model: writing.model.clone(),
                date: writing.date,
                error,
            };
            record_failed(app, id, failed);
        }
    }
    post_conversation(app, id);
    ai::account_used(app, writing.account);
}

// ----------------------------------------------------------------------------
/// The reply of `writing`, with `text`, as it is written to the file.
fn reply_message(writing: &Writing, text: String) -> ConversationMessage {
    ConversationMessage {
        id: Some(writing.id),
        parent: Some(writing.parent),
        model: Some(writing.model.clone()),
        account: Some(writing.account),
        date: Some(writing.date),
        content: Some(text),
        ..Default::default()
    }
}

// ----------------------------------------------------------------------------
/// Keep `failed` with the conversation `id`, to be shown.
fn record_failed(app: &mut App, id: Id, failed: Failed) {
    app.chat.failed.entry(id).or_default().push(failed);
}

// ----------------------------------------------------------------------------
/// Send the conversation `id` as the `conversation` event: a change the
/// page didn't ask for. Nothing if it is gone.
fn post_conversation(app: &App, id: Id) {
    if let Ok(view) = view(app, id) {
        app.post_event("conversation", view);
    }
}

// ----------------------------------------------------------------------------
/// The conversation `id` as the page shows it: its messages, the reply
/// being written, and the failed ones; as `conversation_of` if there is no
/// such conversation.
fn view(app: &App, id: Id) -> Result<ConversationView<'_>> {
    let (doc, conversation) = conversation_of(app, &id)?;
    Ok(view_of(&app.chat, id, doc, conversation))
}

// ----------------------------------------------------------------------------
fn view_of<'a>(
    chat: &ChatState,
    id: Id,
    doc: &'a Document,
    conversation: &Conversation,
) -> ConversationView<'a> {
    let mut messages: Vec<MessageView> = conversation
        .messages
        .iter()
        .filter_map(|m| {
            let message_id = m.id?;
            let content = m.content.clone().unwrap_or_default();
            let stopped = chat.stopped.contains(&message_id);
            Some(MessageView {
                id: message_id,
                parent: m.parent,
                role: if m.is_reply() { "assistant" } else { "user" },
                model: m.model.clone(),
                date: m.date,
                html: markdown::to_html(&content),
                content,
                status: if stopped { "stopped" } else { "done" },
                error: None,
                suggestions: m.suggestions.clone(),
            })
        })
        .collect();
    for failed in chat.failed.get(&id).into_iter().flatten() {
        messages.push(MessageView {
            id: failed.id,
            parent: Some(failed.parent),
            role: "assistant",
            model: Some(failed.model.clone()),
            date: Some(failed.date),
            content: String::new(),
            html: String::new(),
            status: "failed",
            error: Some(failed.error.clone()),
            suggestions: Vec::new(),
        });
    }
    if let Some(writing) = chat.writing.get(&id) {
        messages.push(MessageView {
            id: writing.id,
            parent: Some(writing.parent),
            role: "assistant",
            model: Some(writing.model.clone()),
            date: Some(writing.date),
            content: writing.text.clone(),
            html: markdown::to_html(&writing.text),
            status: "writing",
            error: None,
            suggestions: Vec::new(),
        });
    }
    ConversationView {
        id,
        title: doc.title(),
        linked_to: doc.linked_to(),
        active: conversation.active,
        messages,
    }
}

// ----------------------------------------------------------------------------
/// The request answering `question`: the path down to it, as alternating
/// turns (consecutive turns of one side merged, empty ones left out; a
/// leading reply follows `OPENING`); `context`, what the conversation is
/// about, follows the system text.
fn request_for(
    conversation: &Conversation,
    question: Id,
    model: &str,
    today: DateTime,
    context: Option<&str>,
) -> ChatRequest {
    let mut turns: Vec<(Role, String)> = Vec::new();
    for message in conversation.path_to(question) {
        let text = message.content.as_deref().unwrap_or_default().trim();
        let role = if message.is_reply() {
            Role::Assistant
        } else {
            Role::User
        };
        if text.is_empty() {
            continue;
        }
        if turns.is_empty() && role == Role::Assistant {
            turns.push((Role::User, OPENING.to_string()));
        }
        match turns.last_mut() {
            Some((last, joined)) if *last == role => {
                joined.push_str("\n\n");
                joined.push_str(text);
            }
            _ => turns.push((role, text.to_string())),
        }
    }
    let mut system = format!("{SYSTEM} Today is {}.", today.as_iso8601());
    if let Some(context) = context {
        system.push_str("\n\n");
        system.push_str(context);
    }
    let mut request = ChatRequest::new(model).system(system);
    for (role, text) in turns {
        request = request.message(match role {
            Role::User => Message::user(text),
            Role::Assistant => Message::assistant(text),
        });
    }
    request
}

// ----------------------------------------------------------------------------
/// A conversation's title: the first line of its first message, without
/// markdown markers, cut at a word boundary.
fn title_for(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    plain_title(line, TITLE_LEN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeOllama, Harness, parse, shape};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    fn message(
        id: Id,
        parent: Option<Id>,
        model: Option<&str>,
        content: &str,
    ) -> ConversationMessage {
        ConversationMessage {
            id: Some(id),
            parent,
            model: model.map(str::to_string),
            content: Some(content.to_string()),
            ..Default::default()
        }
    }

    /// Ids in creation order.
    fn ids<const N: usize>() -> [Id; N] {
        let mut ids = [(); N].map(|()| Id::generate());
        ids.sort();
        ids
    }

    fn turns(request: &ChatRequest) -> Vec<(Role, String)> {
        request
            .messages
            .iter()
            .map(|m| {
                let av_llm::Part::Text(text) = &m.parts[0];
                (m.role, text.clone())
            })
            .collect()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_request_follows_the_path_and_alternates() {
        let [q1, a1, a1b, q2, q3] = ids();
        let conversation = Conversation {
            messages: vec![
                message(q1, None, None, "What is due?"),
                message(a1, Some(q1), Some("m"), "Nothing."),
                message(a1b, Some(q1), Some("m"), "Other branch."),
                // A question after a failed reply follows the question.
                message(q2, Some(a1), None, "And next month?"),
                message(q3, Some(q2), None, " Also the gas bill. "),
            ],
            active: Some(q3),
        };
        let request = request_for(&conversation, q3, "gpt-5-mini", DateTime::today(), None);
        assert_eq!(request.model, "gpt-5-mini");
        assert!(request.system.as_deref().unwrap().contains("Today is"));
        assert_eq!(
            turns(&request),
            [
                (Role::User, "What is due?".to_string()),
                (Role::Assistant, "Nothing.".to_string()),
                (
                    Role::User,
                    "And next month?\n\nAlso the gas bill.".to_string()
                ),
            ]
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_leading_reply_follows_the_opening_and_empty_turns_are_left_out() {
        let [a0, a1, q1, a2, q2] = ids();
        let conversation = Conversation {
            messages: vec![
                message(a0, None, Some("m"), "- The amount is new."),
                message(a1, Some(a0), Some("m"), "Thanks, paid."),
                message(q1, Some(a1), None, "Hi"),
                message(a2, Some(q1), Some("m"), "  "),
                message(q2, Some(a2), None, "Still there?"),
            ],
            active: Some(q2),
        };
        let request = request_for(&conversation, q2, "m", DateTime::today(), Some("About X."));
        assert!(
            request
                .system
                .as_deref()
                .unwrap()
                .ends_with(".\n\nAbout X.")
        );
        assert_eq!(
            turns(&request),
            [
                (Role::User, OPENING.to_string()),
                (
                    Role::Assistant,
                    "- The amount is new.\n\nThanks, paid.".to_string()
                ),
                (Role::User, "Hi\n\nStill there?".to_string()),
            ]
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn titles_come_from_the_first_line() {
        assert_eq!(
            title_for("\n  What do I still owe Telekom?\nMore"),
            "What do I still owe Telekom?"
        );
        assert_eq!(title_for("## **Draft** a `reply`"), "Draft a reply");
        assert_eq!(title_for("1. First point"), "First point");
        assert_eq!(title_for("- item"), "item");
        let long =
            "Please summarize every letter from the tax office that arrived since the spring";
        let title = title_for(long);
        assert_eq!(
            title,
            "Please summarize every letter from the tax office that\u{2026}"
        );
        assert!(title.chars().count() <= TITLE_LEN + 1);
        assert_eq!(title_for(""), "");
    }

    // ------------------------------------------------------------------------
    // Dispatch tests, against a fake Ollama.

    const OLLAMA_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b40";
    const NOTE_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b41";
    const CHAT_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b42";
    const Q1: &str = "01a0e1cc-a24c-7490-82fe-e75669985b43";
    const A1: &str = "01a0e1cc-a24c-7490-82fe-e75669985b44";
    const A1B: &str = "01a0e1cc-a24c-7490-82fe-e75669985b45";

    // ------------------------------------------------------------------------
    /// A vault with an Ollama account at `address`, its model `m` picked
    /// for chats, a note, and a conversation about it: a question with two
    /// answers, the second shown.
    fn with_chat(name: &str, address: &str) -> Harness {
        let account = format!("id: {OLLAMA_ID}\nollama:\n  address: {address}\n  offered: [m]\n");
        with_accounts(name, &[(".avelor/accounts/ollama.yaml", account), picked()])
    }

    // ------------------------------------------------------------------------
    /// The vault's settings with the model `m` of `OLLAMA_ID` picked for
    /// chats.
    fn picked() -> (&'static str, String) {
        let settings = format!("chat-model:\n  account: {OLLAMA_ID}\n  model: m\n");
        (".avelor/vault.yaml", settings)
    }

    // ------------------------------------------------------------------------
    /// The vault of `with_chat` with the account files `accounts`.
    fn with_accounts(name: &str, accounts: &[(&str, String)]) -> Harness {
        let note = format!(
            "---\nid: {NOTE_ID}\ntitle: Telekom invoice\ndocument: {{}}\n---\nPay 40 EUR.\n"
        );
        let chat = format!(
            "id: {CHAT_ID}\ntitle: What is due?\nlinked-to: {NOTE_ID}\nai: {{}}\nconversation:\n  messages:\n  - id: {Q1}\n    content: What is due?\n  - id: {A1}\n    parent: {Q1}\n    model: m\n    content: 40 EUR.\n  - id: {A1B}\n    parent: {Q1}\n    model: m\n    content: Forty euros.\n  active: {A1B}\n"
        );
        let mut files = vec![
            ("Notes/telekom.md", note),
            ("Conversations/due.cvn.yaml", chat),
        ];
        files.extend(accounts.iter().cloned());
        Harness::with_vault(name, &files)
    }

    // ------------------------------------------------------------------------
    /// The data of every `ev` event among `sent`.
    fn events<'a>(sent: &'a [Value], ev: &str) -> Vec<&'a Value> {
        sent.iter()
            .filter(|m| m["ev"] == ev)
            .map(|m| &m["data"])
            .collect()
    }

    // ------------------------------------------------------------------------
    /// The conversation `id` as it is in the vault.
    fn conversation(h: &Harness, id: Id) -> Conversation {
        let entry = h.app.vault.index().entry(&id).unwrap();
        entry.doc.roles().conversation.clone().unwrap()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_first_message_starts_a_conversation_and_its_reply_streams_in() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let address = FakeOllama::new(&["m"])
            .replying(&["Pay ", "40 EUR ", "by Friday."])
            .recording(&requests)
            .start();
        let mut h = with_chat("chat-send", &address);
        let args = json!({ "text": " What do I owe? ", "linked_to": NOTE_ID });
        let request = json!({ "id": 1, "cmd": "chat_send", "args": args });
        h.app.on_web_message(request.to_string());
        let is_done = |t: &String| t.contains(r#""ev":"account""#);
        let sent = parse(&h.wait_for(|sent| sent.iter().any(is_done)));

        // The question is written, then the answer shows the reply writing.
        assert_eq!(shape(&sent)[..2], ["entity", "reply"]);
        assert_eq!(sent[0]["data"]["kind"], "conversation");
        assert_eq!(sent[0]["data"]["linked_to"], NOTE_ID);
        let ok = &sent[1]["ok"];
        let id: Id = serde_json::from_value(ok["id"].clone()).unwrap();
        assert_eq!(ok["conversation"]["id"], ok["id"]);
        assert_eq!(ok["conversation"]["title"], "What do I owe?");
        let messages = ok["conversation"]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(
            (&messages[0]["role"], &messages[0]["content"]),
            (&json!("user"), &json!("What do I owe?"))
        );
        assert_eq!(messages[1]["status"], "writing");
        let reply = messages[1]["id"].clone();

        // The reply streams in, is written, and ends the conversation's
        // change; the account's spending follows.
        let deltas = events(&sent, "ai_delta");
        let last = deltas.last().expect("at least one delta");
        assert_eq!(
            (&last["conversation"], &last["message"]),
            (&ok["id"], &reply)
        );
        assert!(
            last["html"]
                .as_str()
                .unwrap()
                .contains("Pay 40 EUR by Friday."),
            "{last}"
        );
        let done = events(&sent, "conversation");
        assert_eq!(done.len(), 1);
        let messages = done[0]["messages"].as_array().unwrap();
        assert_eq!(
            (&messages[1]["status"], &messages[1]["content"]),
            (&json!("done"), &json!("Pay 40 EUR by Friday."))
        );
        assert_eq!(shape(&sent).last(), Some(&"account"));

        let written = conversation(&h, id);
        assert_eq!(written.messages.len(), 2);
        assert_eq!(written.active, written.messages[1].id);
        assert_eq!(
            written.messages[1]
                .account
                .map(|a| a.to_string())
                .as_deref(),
            Some(OLLAMA_ID)
        );
        assert_eq!(written.messages[1].model.as_deref(), Some("m"));

        // The model was told about the note the chat is linked to.
        let requests = requests.lock().unwrap();
        assert_eq!(requests[0]["model"], "m");
        let system = requests[0]["messages"][0]["content"].as_str().unwrap();
        assert!(
            system.contains("asking about this document")
                && system.contains("title: Telekom invoice")
                && system.contains("Pay 40 EUR."),
            "{system}"
        );
        assert_eq!(requests[0]["messages"][1]["content"], "What do I owe?");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_conversation_opens_and_shows_another_branch() {
        let mut h = with_chat("chat-select", "http://127.0.0.1:9");
        let sent = h.call("chat_open", json!({ "id": CHAT_ID }));
        let view = &sent[0]["ok"]["conversation"];
        assert_eq!(
            (&view["id"], &view["active"]),
            (&json!(CHAT_ID), &json!(A1B))
        );
        assert_eq!(view["linked_to"], NOTE_ID);
        assert_eq!(view["messages"].as_array().unwrap().len(), 3);

        let sent = h.call(
            "chat_select",
            json!({ "conversation": CHAT_ID, "message": A1 }),
        );
        assert_eq!(shape(&sent), ["entity", "conversation", "reply"]);
        assert_eq!(sent[1]["data"]["active"], A1);
        assert_eq!(sent[2]["ok"], Value::Null);
        let id = Id::parse(CHAT_ID).unwrap();
        assert_eq!(conversation(&h, id).active, Id::parse(A1).ok());

        let unknown = Id::generate();
        let sent = h.call(
            "chat_select",
            json!({ "conversation": CHAT_ID, "message": unknown }),
        );
        assert_eq!(sent[0]["err"]["code"], "not_found");
        let sent = h.call("chat_open", json!({ "id": unknown }));
        assert_eq!(sent[0]["err"]["code"], "not_found");
        let sent = h.call("chat_open", json!({ "id": NOTE_ID }));
        assert_eq!(sent[0]["err"]["code"], "wrong_kind", "not a conversation");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn sends_that_can_do_nothing_are_refused_in_words() {
        let mut h = with_chat("chat-refused", "http://127.0.0.1:9");
        let send = |h: &mut Harness, args: Value| h.call("chat_send", args);
        let sent = send(
            &mut h,
            json!({ "conversation": CHAT_ID, "text": "  ", "parent": A1B }),
        );
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        assert_eq!(sent[0]["err"]["message"], "The message is empty.");
        let gone = Id::generate();
        let sent = send(&mut h, json!({ "conversation": gone, "text": "Hi" }));
        assert_eq!(sent[0]["err"]["message"], "The conversation is gone.");
        let sent = send(
            &mut h,
            json!({ "conversation": CHAT_ID, "text": "Hi", "parent": gone }),
        );
        assert_eq!(sent[0]["err"]["message"], "The message it follows is gone.");
        let sent = send(&mut h, json!({ "conversation": CHAT_ID, "retry": A1 }));
        assert_eq!(sent[0]["err"]["message"], "The message is gone.", "a reply");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn without_an_account_to_answer_nothing_is_written() {
        let args = json!({ "conversation": CHAT_ID, "text": "And then?", "parent": A1B });
        let id = Id::parse(CHAT_ID).unwrap();

        let mut h = with_accounts("chat-no-account", &[]);
        let sent = h.call("chat_send", args.clone());
        assert_eq!(shape(&sent), ["reply"], "nothing written");
        assert_eq!(sent[0]["err"]["code"], "not_connected");
        assert_eq!(
            sent[0]["err"]["message"],
            "For chats, add an AI provider under Accounts."
        );
        assert_eq!(conversation(&h, id).messages.len(), 3);

        // An account, but no model picked: not the account's first one.
        let account = format!("id: {OLLAMA_ID}\nollama:\n  offered: [m]\n");
        let mut h = with_accounts("chat-no-pick", &[(".avelor/accounts/ollama.yaml", account)]);
        let sent = h.call("chat_send", args.clone());
        assert_eq!(shape(&sent), ["reply"], "nothing written");
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        assert_eq!(sent[0]["err"]["message"], "For chats, pick a model.");

        // The picked account at its spend limit this month: no other is
        // tried.
        let (year, month, _) = DateTime::now().date.to_ymd();
        let limited = format!(
            "id: {OLLAMA_ID}\nollama:\n  address: http://127.0.0.1:9\n  offered: [m]\n  spend-limit: 1\n"
        );
        let spend = format!(
            "account: {OLLAMA_ID}\nmonths:\n  {year:04}-{month:02}:\n    m:\n      calls: 3\n      cost: 1.5\n"
        );
        let other = format!(
            "id: {}\nollama:\n  address: http://127.0.0.1:9\n  offered: [m]\n",
            Id::generate()
        );
        let accounts = [
            (".avelor/accounts/ollama.yaml", limited),
            (".avelor/accounts/ollama.spend.yaml", spend),
            (".avelor/accounts/ollama_2.yaml", other),
            picked(),
        ];
        let mut h = with_accounts("chat-limit", &accounts);
        let sent = h.call("chat_send", args);
        assert_eq!(shape(&sent), ["reply"], "nothing written");
        assert_eq!(sent[0]["err"]["code"], "spend_limit");
        assert_eq!(
            sent[0]["err"]["message"],
            "Ollama has reached its spend limit of $1.00 this month; raise it under Accounts."
        );
        assert_eq!(conversation(&h, id).messages.len(), 3);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_stopped_reply_keeps_what_it_wrote() {
        let address = FakeOllama::new(&["m"])
            .replying(&["Forty", " euros"])
            .holding()
            .start();
        let mut h = with_chat("chat-stop", &address);
        let args = json!({ "conversation": CHAT_ID, "retry": Q1 });
        let request = json!({ "id": 1, "cmd": "chat_send", "args": args });
        h.app.on_web_message(request.to_string());
        let sent = parse(&h.wait_for(|sent| sent.iter().any(|t| t.contains("ai_delta"))));
        let reply = sent.iter().find(|m| m["re"] == 1).unwrap()["ok"].clone();
        let messages = reply["conversation"]["messages"].as_array().unwrap();
        assert_eq!(reply["conversation"]["active"], Q1, "answering again");
        let writing = messages.last().unwrap()["id"].clone();

        let args = json!({ "conversation": CHAT_ID, "text": "More?", "parent": writing });
        let sent = h.call("chat_send", args);
        assert_eq!(sent[0]["err"]["code"], "unsupported");
        assert_eq!(sent[0]["err"]["message"], "A reply is still being written.");

        let sent = h.call("chat_stop", json!({ "conversation": CHAT_ID }));
        assert_eq!(shape(&sent), ["entity", "conversation", "reply"]);
        let messages = sent[1]["data"]["messages"].as_array().unwrap();
        let stopped = messages.iter().find(|m| m["id"] == writing).unwrap();
        assert_eq!(
            (&stopped["status"], &stopped["content"]),
            (&json!("stopped"), &json!("Forty"))
        );
        let id = Id::parse(CHAT_ID).unwrap();
        assert_eq!(conversation(&h, id).messages.len(), 4);

        let sent = h.call("chat_stop", json!({ "conversation": CHAT_ID }));
        assert_eq!(shape(&sent), ["reply"], "nothing is being written");
    }
}
