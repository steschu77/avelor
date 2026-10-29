//! What the model is given: the context of an email or a document, and the
//! texts it reads.

use std::collections::HashMap;

use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_utils::text::cut_marked;
use av_vault::document::Document;
use av_vault::envelope::DocumentType;
use av_vault::shared::DocType;

use super::same;
use av_vault::vault::Index;

/// The most of an email's body that is sent, in characters.
pub(super) const BODY_CHARS: usize = 8000;
/// The most of an attachment's text that goes with its email, in characters,
/// and the most attachments that do.
const ATTACHMENT_CHARS: usize = 2000;
const ATTACHMENTS_SENT: usize = 3;
/// What a body or an attachment's text cut at its count ends with.
const CUT: &str = "\n[\u{2026}]";
/// The type never offered: a note is the user's own (it puts a document
/// under Notes).
const NOTE: &str = "note";
/// The most labels and projects the request lists.
const LABELS_LISTED: usize = 100;
const PROJECTS_LISTED: usize = 50;

// ----------------------------------------------------------------------------
/// What the vault offers an analysis to pick from: the labels in use, the
/// active projects, and the types for a document.
#[derive(Debug, Default)]
pub struct Context {
    /// As the vault spells them, the most used first.
    pub labels: Vec<String>,
    /// Id and title, in the vault's order.
    pub projects: Vec<(Id, String)>,
    /// The well-known types, then those the vault's documents use, each
    /// as first seen; never `NOTE`.
    pub types: Vec<String>,
}

// ----------------------------------------------------------------------------
/// The context of the vault `index`: its labels by how many entities carry them (a
/// label in other case counts as the same, spelled as first seen), and its
/// projects that aren't archived; each list capped (the labels used most,
/// the first projects); the types a document may get.
pub fn context(index: &Index) -> Context {
    let mut labels: Vec<(String, usize)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut projects = Vec::new();
    let mut types: Vec<String> = DocType::well_known().map(str::to_string).collect();
    for entry in index.all() {
        let doc = &entry.doc;
        let role = doc.roles().document.as_ref();
        if let Some(doc_type) = role.and_then(|role| role.doc_type.as_ref()) {
            types.push(doc_type.as_str().to_string());
        }
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
    let mut offered: Vec<String> = Vec::new();
    for doc_type in types {
        if !same(&doc_type, NOTE) && !offered.iter().any(|g| same(g, &doc_type)) {
            offered.push(doc_type);
        }
    }
    Context {
        labels: labels.into_iter().map(|(label, _)| label).collect(),
        projects,
        types: offered,
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
    /// Its attachments' names and read text (`attachment_texts`).
    pub attachments: Vec<(String, String)>,
}

impl EmailText {
    // ------------------------------------------------------------------------
    /// `doc` as an email, without its attachments.
    pub fn of(doc: &Document) -> Self {
        let email = doc.roles().email.as_ref();
        let body = cut_marked(doc.content(), BODY_CHARS, CUT).into_owned();
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
            attachments: Vec::new(),
        }
    }
}

// ----------------------------------------------------------------------------
/// What goes with the email `email` of its attachments: the documents linked
/// to it that have an original and text, each with the name its file came
/// with and its text cut at `ATTACHMENT_CHARS`; the first
/// `ATTACHMENTS_SENT`.
pub(super) fn attachment_texts(index: &Index, email: Id) -> Vec<(String, String)> {
    index
        .attachments_of(email)
        .filter_map(|entry| {
            let original = entry.doc.original()?;
            let text = entry.doc.content().trim();
            let name = original.name.clone();
            let name = name.unwrap_or_else(|| entry.doc.title().to_string());
            (!text.is_empty()).then(|| (name, cut_marked(text, ATTACHMENT_CHARS, CUT).into_owned()))
        })
        .take(ATTACHMENTS_SENT)
        .collect()
}

// ----------------------------------------------------------------------------
/// An attachment as the analysis sends it: its read text, and the email it
/// came with.
#[derive(Debug, Clone)]
pub struct DocText {
    pub id: Id,
    /// Its title when the analysis was planned: its file's name, for one
    /// just saved.
    pub title: String,
    /// The name its file came with.
    pub name: String,
    /// Its date when the analysis was planned: its email's, for one just
    /// saved.
    pub date: Option<DateTime>,
    /// The subject and sender of the email it is linked to.
    pub email: Option<(String, String)>,
    /// Cut at `BODY_CHARS`.
    pub body: String,
}

impl DocText {
    // ------------------------------------------------------------------------
    /// `doc`, an attachment, with the email of `index` it is linked to.
    pub fn of(index: &Index, doc: &Document) -> Self {
        let email = doc.linked_to().and_then(|id| index.get(&id));
        let email = email
            .filter(|email| email.roles().email.is_some())
            .map(|email| {
                let email = EmailText::of(email);
                (email.subject, email.from)
            });
        let name = doc.original().and_then(|original| original.name.clone());
        Self {
            id: doc.id(),
            title: doc.title().to_string(),
            name: name.unwrap_or_else(|| doc.title().to_string()),
            date: doc.date(),
            email,
            body: cut_marked(doc.content(), BODY_CHARS, CUT).into_owned(),
        }
    }
}
