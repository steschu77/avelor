//! The answers applied to an entity: its type, title, date, labels and
//! projects, Medium's messages.

use av_vault::document::Document;
use av_vault::shared::DocType;

use super::answer::Lite;
use super::same;

// ----------------------------------------------------------------------------
/// Write `lite` into the email `doc`: a new title, the old one kept as
/// `email.subject` if it has none; its type (`document: {type}` beside
/// `email:`), unless it has one; the labels added to its own (in any case
/// once); the projects joined.
pub fn apply_lite(doc: &mut Document, lite: &Lite) {
    if let Some(mut email) = doc.roles().email.clone() {
        let before = email.clone();
        if lite.title.is_some() && email.subject.is_none() {
            email.subject = Some(doc.title().to_string());
        }
        if email != before {
            doc.set_email(Some(email));
        }
    }
    if let Some(title) = &lite.title {
        doc.set_title(title.clone());
    }
    add_doc_type(doc, lite);
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
/// `lite`'s type given to `doc`, unless it has one.
fn add_doc_type(doc: &mut Document, lite: &Lite) {
    let role = doc.roles().document.as_ref();
    let has_type = role.is_some_and(|role| role.doc_type.is_some());
    if let Some(doc_type) = lite.doc_type.as_deref().filter(|_| !has_type) {
        doc.set_doc_type(Some(DocType::parse(doc_type)));
    }
}
