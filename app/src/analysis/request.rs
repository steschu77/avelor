//! The requests to the model: each level's rules and its answer's shape.

use av_llm::{ChatRequest, Message};

use super::context::{Context, EmailText};
use crate::ai;
use crate::google::accounts::AnalysisLevel;

/// The JSON object each level asks for.
pub(super) const LITE_SHAPE: &str =
    "{\"kind\": \"...\", \"title\": \"...\", \"labels\": [\"...\"], \"projects\": [\"...\"]}";
pub(super) const MEDIUM_SHAPE: &str = "{\"kind\": \"...\", \"title\": \"...\", \"labels\": [\"...\"], \
\"projects\": [\"...\"], \"check\": [\"...\"], \"reply\": \"...\", \"questions\": [\"...\"]}";

const LITE_RULES: &str = "
- kind: what the email is: \"request\" (it asks its recipient to do, answer, pay or decide \
something), \"reply\" (it answers an email of the recipient's), \"notification\" (sent \
automatically: receipts, shipping, account and security alerts, reminders), \"newsletter\" (news, \
offers or marketing sent to many) or \"other\".
- title: a clean title for the email, in the email's language, at most 80 characters: what it is \
about, without \"Re:\", \"Fwd:\", ticket or reference numbers, tags in brackets or the sender's \
name.
- labels: up to 3 labels from the list of labels that clearly fit the email; none if unsure.
- projects: up to 2 projects from the list of projects the email clearly belongs to; none if \
unsure.";

const MEDIUM_RULES: &str = "
- check: up to 3 points a closer look at the email should check, one sentence each: an amount, a \
deadline, a claim, a condition; none if there is nothing to check, and none for a newsletter.
- reply: a short reply to the email in its language, ready to send, without a signature or \
placeholders; only if the email expects an answer from its recipient, else null; never for a \
notification or a newsletter.
- questions: 1 to 3 short questions the recipient might ask about this email; none for a \
newsletter.
Write the points and the questions in the email's language too.";

const LISTS_ONLY: &str = "\nUse only labels and projects from the lists, spelled as given.";

// ----------------------------------------------------------------------------
/// The request of the analysis at `level` for `email` to `model`: the
/// instructions, then the lists of `ctx` and the email.
pub fn request_for(
    level: AnalysisLevel,
    model: &str,
    ctx: &Context,
    email: &EmailText,
) -> ChatRequest {
    let list = |names: Vec<&str>| match names.is_empty() {
        true => "(none)\n".to_string(),
        false => names.iter().map(|n| format!("- {n}\n")).collect(),
    };
    let labels = list(ctx.labels.iter().map(String::as_str).collect());
    let projects = list(ctx.projects.iter().map(|(_, t)| t.as_str()).collect());
    let date = email.date.map(|d| d.as_iso8601()).unwrap_or_default();
    let text = format!(
        "Labels:\n{labels}\nProjects:\n{projects}\nThe email:\nSubject: {}\nFrom: {}\nTo: {}\nDate: {date}\n\n{}",
        email.subject, email.from, email.to, email.body
    );
    let medium = level == AnalysisLevel::Medium;
    let task = match medium {
        true => "You sort incoming email and prepare it for its recipient.",
        false => "You sort incoming email.",
    };
    let shape = shape(level);
    let mut system =
        format!("{task} Answer with one JSON object and nothing else: {shape}.{LITE_RULES}");
    if medium {
        system.push_str(MEDIUM_RULES);
    }
    system.push_str(LISTS_ONLY);
    ChatRequest::new(model)
        .system(system)
        .message(Message::user(text))
        // Sorting an email needs no reasoning, and on a small context window
        // the reasoning leaves no room for the answer.
        .think(false)
        .context(ai::CONTEXT_TOKENS)
}

// ----------------------------------------------------------------------------
/// The JSON object the analysis at `level` asks for.
fn shape(level: AnalysisLevel) -> &'static str {
    match level {
        AnalysisLevel::Medium => MEDIUM_SHAPE,
        _ => LITE_SHAPE,
    }
}

// ----------------------------------------------------------------------------
/// `request` asked again after `answer`, which wasn't the JSON object: the
/// answer and a reminder of the object `level` asks for follow it. An empty
/// answer is just asked again (providers refuse an empty turn).
pub fn again(request: ChatRequest, answer: &str, level: AnalysisLevel) -> ChatRequest {
    if answer.trim().is_empty() {
        return request;
    }
    let reminder = format!(
        "That wasn't the JSON object asked for. Answer again with only the JSON object {}, \
nothing before or after it.",
        shape(level)
    );
    request
        .message(Message::assistant(answer))
        .message(Message::user(reminder))
}
