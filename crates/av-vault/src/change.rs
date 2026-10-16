//! One change to entities (`api.md#structure`'s `edit_entities`): one of a
//! task's fields. [`Index::edit_entities`] writes it to every entity named
//! that can take it.
//!
//! [`Index::edit_entities`]: crate::vault::Index::edit_entities

use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use serde::Deserialize;

use crate::document::Document;
use crate::envelope::DocumentType;

// ----------------------------------------------------------------------------
/// The change, as the page sends it: `{"task": {"status": …}}`; a status
/// can't be empty ([`Change::check`]).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    /// One of a task's fields (vault.md#task).
    Task(TaskChange),
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskChange {
    /// `done` also writes when it was marked done.
    Status(String),
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
    /// Whether an entity of `kind` can take the change: a task's field a
    /// task.
    pub fn fits(&self, kind: DocumentType) -> bool {
        match self {
            Change::Task(_) => kind == DocumentType::Task,
        }
    }

    // ------------------------------------------------------------------------
    /// Apply the change to `doc`, one that [`Change::fits`] it; `now` is
    /// when a task is marked done.
    pub fn apply(&self, doc: &mut Document, now: DateTime) {
        match self {
            Change::Task(TaskChange::Status(status)) => {
                doc.set_task_status(status.trim().to_string(), now)
            }
        }
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::FileFormat;

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
        let done = Change::Task(TaskChange::Status("done".into()));
        assert!(done.check().is_ok());
        assert!(done.fits(DocumentType::Task) && !done.fits(DocumentType::Email));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_status_is_trimmed() {
        let mut doc = task("  status: todo\n");
        Change::Task(TaskChange::Status(" done ".into())).apply(&mut doc, date(NOW));
        let task = doc.roles().task.clone().unwrap();
        assert_eq!(
            (task.status.as_deref(), task.done),
            (Some("done"), Some(date(NOW)))
        );
    }
}
