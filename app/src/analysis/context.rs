//! What the model is given: the context of an email or a document, and the
//! texts it reads.

use std::collections::HashMap;

use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_vault::document::Document;
use av_vault::envelope::DocumentType;

use av_vault::vault::Index;

/// The most of an email's body that is sent, in characters.
pub(super) const BODY_CHARS: usize = 8000;
/// The most labels and projects the request lists.
const LABELS_LISTED: usize = 100;
const PROJECTS_LISTED: usize = 50;

// ----------------------------------------------------------------------------
/// What the vault offers an analysis to pick from: the labels in use and
/// the active projects.
#[derive(Debug, Default)]
pub struct Context {
    /// As the vault spells them, the most used first.
    pub labels: Vec<String>,
    /// Id and title, in the vault's order.
    pub projects: Vec<(Id, String)>,
}

// ----------------------------------------------------------------------------
/// The context of the vault `index`: its labels by how many entities carry them (a
/// label in other case counts as the same, spelled as first seen), and its
/// projects that aren't archived; each list capped (the labels used most,
/// the first projects).
pub fn context(index: &Index) -> Context {
    let mut labels: Vec<(String, usize)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut projects = Vec::new();
    for entry in index.all() {
        let doc = &entry.doc;
        for label in doc.labels() {
            let key = label.to_lowercase();
            match seen.get(&key) {
                Some(&i) => labels[i].1 += 1,
                None => {
                    seen.insert(key, labels.len());
                    labels.push((label.clone(), 1));
                }
            }
        }
        if doc.kind() == DocumentType::Project && !doc.archived() {
            projects.push((doc.id(), doc.title().to_string()));
        }
    }
    labels.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    labels.truncate(LABELS_LISTED);
    projects.truncate(PROJECTS_LISTED);
    Context {
        labels: labels.into_iter().map(|(label, _)| label).collect(),
        projects,
    }
}

// ----------------------------------------------------------------------------
/// An email as the analysis sends it.
#[derive(Debug, Clone)]
pub struct EmailText {
    pub id: Id,
    /// Its title when the analysis was planned: the subject, for an email
    /// just fetched.
    pub title: String,
    /// The subject as the email came: `email.subject`, else the title.
    pub subject: String,
    pub from: String,
    /// The recipients, joined.
    pub to: String,
    pub date: Option<DateTime>,
    /// Cut at `BODY_CHARS`.
    pub body: String,
}

impl EmailText {
    // ------------------------------------------------------------------------
    pub fn of(doc: &Document) -> Self {
        let email = doc.roles().email.as_ref();
        let content = doc.content();
        let mut body: String = content.chars().take(BODY_CHARS).collect();
        if body.len() < content.len() {
            body.push_str("\n[\u{2026}]");
        }
        let subject = email.and_then(|e| e.subject.as_deref());
        Self {
            id: doc.id(),
            title: doc.title().to_string(),
            subject: subject.unwrap_or(doc.title()).to_string(),
            from: email.and_then(|e| e.from.clone()).unwrap_or_default(),
            to: email
                .map(|e| e.to.as_slice().join(", "))
                .unwrap_or_default(),
            date: doc.date(),
            body,
        }
    }
}
