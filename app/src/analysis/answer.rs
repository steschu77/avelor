//! The model's answers read: Lite's and Medium's, an email's and a document's.

use av_utils::id::Id;
use serde_json::Value;

use super::context::Context;
use super::same;
use crate::google::accounts::AnalysisLevel;
use crate::texts::Text;

/// The most labels and projects one email gets.
const LABELS_PER_EMAIL: usize = 3;
const PROJECTS_PER_EMAIL: usize = 2;
/// A longer title is cut.
pub(super) const TITLE_CHARS: usize = 120;
/// The types the analysis gives an email (email.md#analysis).
pub const EMAIL_TYPES: [&str; 2] = ["newsletter", "notification"];

// ----------------------------------------------------------------------------
/// What Lite found for an email, in the vault's terms.
#[derive(Debug, Default, PartialEq)]
pub struct Lite {
    /// `None`: the title stays.
    pub title: Option<String>,
    /// One of `EMAIL_TYPES`.
    pub doc_type: Option<String>,
    /// Labels in use, as the vault spells them.
    pub labels: Vec<String>,
    /// Active projects.
    pub projects: Vec<Id>,
}

// ----------------------------------------------------------------------------
/// Read the `answer` of the analysis at `level` against `ctx` for an email
/// titled `title`: the JSON object in it (code fences and text around it
/// ignored); a title trimmed and cut, dropped if empty or the email's own;
/// a type from `EMAIL_TYPES`, in any case; the labels and projects from the lists
/// only, matched in any case, each once, the first few kept. An answer
/// without such an object is refused, in words.
pub fn parse(
    answer: &str,
    _level: AnalysisLevel,
    ctx: &Context,
    title: &str,
) -> std::result::Result<Lite, Text> {
    let refused = || Text::new("analysis.not_json");
    let (Some(start), Some(end)) = (answer.find('{'), answer.rfind('}')) else {
        return Err(refused());
    };
    let object: Value = match answer.get(start..=end).map(serde_json::from_str) {
        Some(Ok(object @ Value::Object(_))) => object,
        _ => return Err(refused()),
    };

    let new_title = object["title"]
        .as_str()
        .map(|t| cut(t, TITLE_CHARS))
        .filter(|t| !t.is_empty() && t != title.trim());
    let doc_type = object["type"]
        .as_str()
        .and_then(|doc_type| EMAIL_TYPES.into_iter().find(|known| same(known, doc_type)))
        .map(str::to_string);

    let mut labels: Vec<String> = Vec::new();
    for name in names(&object["labels"]) {
        let found = ctx.labels.iter().find(|l| same(l, name));
        if let Some(label) = found.filter(|l| !labels.contains(l)) {
            labels.push(label.clone());
        }
    }
    labels.truncate(LABELS_PER_EMAIL);

    let mut projects: Vec<Id> = Vec::new();
    for name in names(&object["projects"]) {
        let found = ctx.projects.iter().find(|(_, t)| same(t, name));
        if let Some((id, _)) = found.filter(|(id, _)| !projects.contains(id)) {
            projects.push(*id);
        }
    }
    projects.truncate(PROJECTS_PER_EMAIL);

    Ok(Lite {
        title: new_title,
        doc_type,
        labels,
        projects,
    })
}

// ----------------------------------------------------------------------------
/// `text` trimmed and cut at `most` characters.
fn cut(text: &str, most: usize) -> String {
    let cut: String = text.trim().chars().take(most).collect();
    cut.trim_end().to_string()
}

// ----------------------------------------------------------------------------
/// The names in `value`: a list of strings, or a single one.
fn names(value: &Value) -> Vec<&str> {
    match value {
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        Value::String(name) => vec![name.as_str()],
        _ => Vec::new(),
    }
}
