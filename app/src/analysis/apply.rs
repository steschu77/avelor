//! The answers applied to an entity: its kind, title, date, labels and
//! projects, Medium's messages.

use av_vault::document::Document;

use super::answer::Lite;
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
