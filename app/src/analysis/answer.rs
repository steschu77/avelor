//! The model's answers read: Lite's and Medium's, an email's and a document's.

use av_utils::datetime::DateTime;
use av_utils::id::Id;
use serde_json::Value;

use super::context::{Context, DocText};
use super::{AnalysisLevel, same};
use crate::texts::Text;

const DAY_SECONDS: i64 = 86_400;
/// The most labels and projects one email gets.
const LABELS_PER_EMAIL: usize = 3;
const PROJECTS_PER_EMAIL: usize = 2;
/// A longer title is cut.
pub(super) const TITLE_CHARS: usize = 120;
/// The most points to check and questions one email gets, and how long
/// each may be; a longer quick reply is cut too.
pub(super) const CHECKS_PER_EMAIL: usize = 3;
const QUESTIONS_PER_EMAIL: usize = 3;
pub(super) const POINT_CHARS: usize = 300;
const REPLY_CHARS: usize = 4000;
/// The types the analysis gives an email (email.md#analysis).
pub const EMAIL_TYPES: [&str; 2] = ["newsletter", "notification"];

// ----------------------------------------------------------------------------
/// What Lite found for an email or an attachment, in the vault's terms.
#[derive(Debug, Default, PartialEq)]
pub struct Lite {
    /// `None`: the title stays.
    pub title: Option<String>,
    /// Labels in use, as the vault spells them.
    pub labels: Vec<String>,
    /// Active projects.
    pub projects: Vec<Id>,
    /// An email's: one of `EMAIL_TYPES`; an attachment's: one of
    /// `Context::types`.
    pub doc_type: Option<String>,
    /// An attachment's own date.
    pub date: Option<DateTime>,
}

// ----------------------------------------------------------------------------
/// What Medium found beyond Lite, for the email's conversation: a quick reply or a
/// point to check, at least.
#[derive(Debug, Default, PartialEq)]
pub struct Medium {
    pub check: Vec<String>,
    pub reply: Option<String>,
    pub questions: Vec<String>,
}

// ----------------------------------------------------------------------------
/// Read the `answer` of the analysis at `level` against `ctx` for an email
/// titled `title`: the JSON object in it (code fences and text around it
/// ignored); a title trimmed and cut, dropped if empty or the email's own;
/// a type from `EMAIL_TYPES`, in any case; the labels and projects from the lists
/// only, matched in any case, each once, the first few kept. Medium's
/// points, reply and questions trimmed and cut, the first few kept; none
/// for a newsletter, no reply for a notification, and no conversation without a
/// reply or a point. An answer without such an object is refused, in
/// words.
pub fn parse(
    answer: &str,
    level: AnalysisLevel,
    ctx: &Context,
    title: &str,
) -> std::result::Result<(Lite, Option<Medium>), Text> {
    let object = object_in(answer)?;
    let doc_type = object["type"]
        .as_str()
        .and_then(|doc_type| EMAIL_TYPES.into_iter().find(|known| same(known, doc_type)))
        .map(str::to_string);
    let lite = Lite {
        doc_type,
        ..shared_part(&object, ctx, title)
    };
    let medium = match level {
        AnalysisLevel::Medium => medium_of(&object, lite.doc_type.as_deref()),
        _ => None,
    };
    Ok((lite, medium))
}

// ----------------------------------------------------------------------------
/// Read the `answer` of the analysis at `level` for the document `doc`
/// against `ctx` as `parse` reads an email's: its type from the list only, in any case;
/// its date as `YYYY-MM-DD`, that day at 12:00 UTC, the same day in every
/// time zone, dropped before 1900 or after the day after the document's
/// date (its email's; now without one).
pub fn parse_document(
    answer: &str,
    level: AnalysisLevel,
    ctx: &Context,
    doc: &DocText,
) -> std::result::Result<(Lite, Option<Medium>), Text> {
    let object = object_in(answer)?;
    let doc_type = object["type"]
        .as_str()
        .and_then(|doc_type| ctx.types.iter().find(|known| same(known, doc_type)))
        .cloned();
    let day = |date: DateTime| date.as_unix().div_euclid(DAY_SECONDS);
    let latest = day(doc.date.unwrap_or_else(DateTime::now)) + 1;
    let date = object["date"]
        .as_str()
        .and_then(day_of)
        .filter(|date| day(*date) <= latest);
    let lite = Lite {
        doc_type,
        date,
        ..shared_part(&object, ctx, &doc.title)
    };
    let medium = doc.medium(level).then(|| document_medium_of(&object));
    Ok((lite, medium.flatten()))
}

// ----------------------------------------------------------------------------
/// The day `text` names as `YYYY-MM-DD` (anything after it ignored), at
/// 12:00 UTC; `None` if it names none, or one before 1900.
fn day_of(text: &str) -> Option<DateTime> {
    let mut parts = text.trim().get(..10)?.split('-');
    let mut next = || parts.next()?.parse::<i32>().ok();
    let (year, month, day) = (next()?, next()?, next()?);
    if year < 1900 {
        return None;
    }
    DateTime::from_ymd_hms(year, month, day, 12, 0, 0).ok()
}

// ----------------------------------------------------------------------------
/// The JSON object in `answer`, code fences and text around it ignored; an
/// answer without one is refused, in words.
fn object_in(answer: &str) -> std::result::Result<Value, Text> {
    let refused = || Text::new("analysis.not_json");
    let (Some(start), Some(end)) = (answer.find('{'), answer.rfind('}')) else {
        return Err(refused());
    };
    match answer.get(start..=end).map(serde_json::from_str) {
        Some(Ok(object @ Value::Object(_))) => Ok(object),
        _ => Err(refused()),
    }
}

// ----------------------------------------------------------------------------
/// What an email's answer `object` and an attachment's share: a title trimmed
/// and cut, dropped if empty or `title`; the labels and projects from the
/// lists of `ctx` only, matched in any case, each once, the first few kept.
fn shared_part(object: &Value, ctx: &Context, title: &str) -> Lite {
    let new_title = object["title"]
        .as_str()
        .map(|t| cut(t, TITLE_CHARS))
        .filter(|t| !t.is_empty() && t != title.trim());

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

    Lite {
        title: new_title,
        labels,
        projects,
        ..Lite::default()
    }
}

// ----------------------------------------------------------------------------
/// Medium's part of the answer `object` for an email of the type `doc_type`
/// (see `parse`).
fn medium_of(object: &Value, doc_type: Option<&str>) -> Option<Medium> {
    if doc_type == Some("newsletter") {
        return None;
    }
    let points = |key: &str, most: usize| points(object, key, most);
    let reply = object["reply"]
        .as_str()
        .map(|reply| cut(reply, REPLY_CHARS))
        .filter(|reply| !reply.is_empty() && doc_type != Some("notification"));
    let medium = Medium {
        check: points("check", CHECKS_PER_EMAIL),
        reply,
        questions: points("questions", QUESTIONS_PER_EMAIL),
    };
    (medium.reply.is_some() || !medium.check.is_empty()).then_some(medium)
}

// ----------------------------------------------------------------------------
/// Medium's part of the answer `object` for a document without an email: its
/// points to check and questions, no reply; no conversation without a point.
fn document_medium_of(object: &Value) -> Option<Medium> {
    let medium = Medium {
        check: points(object, "check", CHECKS_PER_EMAIL),
        reply: None,
        questions: points(object, "questions", QUESTIONS_PER_EMAIL),
    };
    (!medium.check.is_empty()).then_some(medium)
}

// ----------------------------------------------------------------------------
/// The points under `key` of the answer `object`: trimmed, cut at
/// `POINT_CHARS`, the first `most` that aren't empty.
fn points(object: &Value, key: &str, most: usize) -> Vec<String> {
    let points = names(&object[key]).into_iter().map(|p| cut(p, POINT_CHARS));
    points.filter(|p| !p.is_empty()).take(most).collect()
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
