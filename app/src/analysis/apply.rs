//! The answers applied to an entity: its kind, title, date, labels and
//! projects, Medium's messages.

use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_vault::document::Document;
use av_vault::envelope::ConversationMessage;

use super::answer::{Lite, Medium};
use super::same;

// ----------------------------------------------------------------------------
/// Write `lite` into the email `doc`: a new title, the old one kept as
/// `email.subject` if it has none; its kind as `email.kind`; the labels
/// added to its own (in any case once); the projects joined.
pub fn apply_lite(doc: &mut Document, lite: &Lite) {
    if let Some(mut email) = doc.roles().email.clone() {
        let before = email.clone();
        if lite.title.is_some() && email.subject.is_none() {
            email.subject = Some(doc.title().to_string());
        }
        if lite.kind.is_some() {
            email.kind = lite.kind.clone();
        }
        if email != before {
            doc.set_email(Some(email));
        }
    }
    if let Some(title) = &lite.title {
        doc.set_title(title.clone());
    }
    let mut labels = doc.labels().to_vec();
    let before = labels.len();
    for label in &lite.labels {
        if !labels.iter().any(|t| same(t, label)) {
            labels.push(label.clone());
        }
    }
    if labels.len() > before {
        doc.set_labels(labels);
    }
    for project in &lite.projects {
        doc.join_project(*project);
    }
}

// ----------------------------------------------------------------------------
/// The messages of Medium's conversation about an email: the points to check as a
/// list, then the quick reply as a message of its own, each following the
/// one before; the questions are offered under the last. Written by `model`
/// through `account` at `now`.
pub fn medium_messages(
    medium: &Medium,
    model: &str,
    account: Id,
    now: DateTime,
) -> Vec<ConversationMessage> {
    let points = medium.check.iter().map(|p| format!("- {p}"));
    let points = Some(points.collect::<Vec<_>>().join("\n")).filter(|p| !p.is_empty());
    let mut messages: Vec<ConversationMessage> = Vec::new();
    for content in [points, medium.reply.clone()].into_iter().flatten() {
        messages.push(ConversationMessage {
            id: Some(Id::generate()),
            parent: messages.last().and_then(|m| m.id),
            model: Some(model.to_string()),
            account: Some(account),
            date: Some(now),
            content: Some(content),
            ..Default::default()
        });
    }
    if let Some(last) = messages.last_mut() {
        last.suggestions = medium.questions.clone();
    }
    messages
}
