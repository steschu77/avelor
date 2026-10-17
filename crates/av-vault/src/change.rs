//! One change to entities (`api.md#structure`'s `edit_entities`): a
//! document's type, or one of a task's fields. [`Index::edit_entities`]
//! writes it to every entity named that can take it.
//!
//! [`Index::edit_entities`]: crate::vault::Index::edit_entities

use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use serde::Deserialize;

use crate::document::Document;
use crate::envelope::DocumentType;
use crate::shared::{ContactRef, DocType};

// ----------------------------------------------------------------------------
/// The change, as the page sends it: `{"type": …}` or `{"task":
/// {"status" | "due" | "assignee": …}}`. A null or empty value removes the
/// key; a status can't be empty ([`Change::check`]).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    /// A document's type (vault.md#document).
    #[serde(rename = "type")]
    DocType(Option<String>),
    /// One of a task's fields (vault.md#task).
    Task(TaskChange),
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskChange {
    /// `done` also writes when it was marked done.
    Status(String),
    Due(Option<DateTime>),
    /// A contact of the vault (`@<uuid>`) or a name.
    Assignee(Option<String>),
}

impl Change {
    // ------------------------------------------------------------------------
    /// Whether the change can be written at all: a status can't be empty.
    pub fn check(&self) -> Result<()> {
        match self {
            Change::Task(TaskChange::Status(status)) if status.trim().is_empty() => {
                Err(Error::InvalidArgument {
                    arg: "status".into(),
                })
            }
            _ => Ok(()),
        }
    }

    // ------------------------------------------------------------------------
    /// Whether an entity of `kind` can take the change: a type a
    /// document, a task's field a task.
    pub fn fits(&self, kind: DocumentType) -> bool {
        match self {
            Change::DocType(_) => kind == DocumentType::Document,
            Change::Task(_) => kind == DocumentType::Task,
        }
    }

    // ------------------------------------------------------------------------
    /// Apply the change to `doc`, one that [`Change::fits`] it; `now` is
    /// when a task is marked done.
    pub fn apply(&self, doc: &mut Document, now: DateTime) {
        match self {
            Change::DocType(doc_type) => {
                doc.set_doc_type(text(doc_type).map(DocType::parse));
            }
            Change::Task(TaskChange::Status(status)) => {
                doc.set_task_status(status.trim().to_string(), now)
            }
            Change::Task(TaskChange::Due(due)) => doc.set_task_due(*due),
            Change::Task(TaskChange::Assignee(assignee)) => {
                doc.set_task_assignee(text(assignee).map(ContactRef::parse))
            }
        }
    }
}

// ----------------------------------------------------------------------------
/// `value` trimmed; none if it is empty.
fn text(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::FileFormat;
    use av_utils::id::Id;

    fn task(text: &str) -> Document {
        Document::load(format!("---\ntask:\n{text}---\n"), FileFormat::Markdown)
    }

    fn date(iso: &str) -> DateTime {
        DateTime::from_iso8601(iso).unwrap()
    }

    const NOW: &str = "2026-10-12T08:30:00Z";

    // ------------------------------------------------------------------------
    #[test]
    fn an_empty_status_is_refused_and_the_rest_fits_its_kind() {
        let empty = Change::Task(TaskChange::Status(" ".into()));
        assert!(matches!(empty.check(), Err(Error::InvalidArgument { .. })));
        let doc_type = Change::DocType(Some(String::new()));
        assert!(doc_type.check().is_ok());
        assert!(doc_type.fits(DocumentType::Document) && !doc_type.fits(DocumentType::Task));
        let due = Change::Task(TaskChange::Due(None));
        assert!(due.fits(DocumentType::Task) && !due.fits(DocumentType::Email));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_due_date_is_set_and_removed_keeping_its_time() {
        let mut doc = task("  status: todo\n");
        let due = Some(date("2026-10-20T15:00:00Z"));
        Change::Task(TaskChange::Due(due)).apply(&mut doc, date(NOW));
        assert!(
            doc.save().contains("due: 2026-10-20T15:00:00Z"),
            "{}",
            doc.save()
        );
        Change::Task(TaskChange::Due(None)).apply(&mut doc, date(NOW));
        assert!(!doc.save().contains("due"), "{}", doc.save());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_assignee_is_a_contact_or_a_name_and_empty_removes_it() {
        let assign = |doc: &mut Document, to: &str| {
            Change::Task(TaskChange::Assignee(Some(to.into()))).apply(doc, date(NOW));
            doc.roles().task.as_ref().unwrap().assignee.clone()
        };
        let contact = Id::generate();
        let mut doc = task("  status: todo\n");
        assert_eq!(
            assign(&mut doc, &format!("@{contact}")),
            Some(ContactRef::Id(contact))
        );
        assert_eq!(
            assign(&mut doc, " Jane Doe "),
            Some(ContactRef::Name("Jane Doe".into()))
        );
        assert_eq!(assign(&mut doc, ""), None);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_type_is_parsed_and_a_status_trimmed() {
        let mut doc = Document::load("---\ndocument: {}\n---\n".into(), FileFormat::Markdown);
        Change::DocType(Some(" Contract ".into())).apply(&mut doc, date(NOW));
        let doc_type = doc.roles().document.as_ref().unwrap().doc_type.clone();
        assert_eq!(doc_type, Some(DocType::Contract));

        let mut doc = task("  status: todo\n");
        Change::Task(TaskChange::Status(" done ".into())).apply(&mut doc, date(NOW));
        let task = doc.roles().task.clone().unwrap();
        assert_eq!(
            (task.status.as_deref(), task.done),
            (Some("done"), Some(date(NOW)))
        );
    }
}
