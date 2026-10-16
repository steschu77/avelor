//! An entity's properties changed: its title, labels, links, a transaction's
//! fields, its body.

use super::{Changes, Index, SaveOutcome};
use crate::change::Change;
use crate::document::Document;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::id::Id;
use av_utils::text::unique_trimmed;

// ----------------------------------------------------------------------------
impl Index {
    /// Set the title of `id`, which may be any entity. An empty title removes
    /// it ([`Document::set_title`]). Only the file's content changes; the file
    /// is not renamed to match (vault.md#filenames).
    pub fn set_title(
        &mut self,
        id: &Id,
        title: String,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_title(title))
    }

    /// Set the labels (`tags`) of `id`, which may be any entity: each one
    /// trimmed, empty ones dropped, a repeated one kept once where it first
    /// appears. No labels removes the key.
    pub fn set_labels(
        &mut self,
        id: &Id,
        labels: Vec<String>,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        let labels = unique_trimmed(labels);
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_labels(labels))
    }

    /// Link `id`, which may be any entity, to `to`, another entity in the
    /// vault, replacing an older link; `None` removes the `linked-to` key
    /// ([`Document::set_link`]). Deleting an entity leaves the links to it
    /// in place, so restoring it brings them back.
    pub fn set_link(
        &mut self,
        id: &Id,
        to: Option<Id>,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_link(id, to.as_ref())?;
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_link(to))
    }

    /// Star or unstar `id`, which may be any entity; unstarring removes the
    /// key ([`Document::set_starred`]). Nothing is written if it already is.
    pub fn set_starred(
        &mut self,
        id: &Id,
        starred: bool,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_starred(starred))
    }

    /// Archive `id`, which may be any entity, or take it out of the archive;
    /// unarchiving removes the key ([`Document::set_archived`]). Nothing is
    /// written if it already is.
    pub fn set_archived(
        &mut self,
        id: &Id,
        archived: bool,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_archived(archived))
    }

    /// Apply `change` to `id`, which may be any entity, as one write: for
    /// edits that touch several properties at once. Nothing is written if
    /// nothing changed.
    pub fn edit(
        &mut self,
        id: &Id,
        changes: &mut Changes,
        change: impl FnOnce(&mut Document),
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, change)
    }

    /// Write `change` to each of `ids` as its own write: the outcome per
    /// id, in their order. An entity the change doesn't fit ([`Change::fits`])
    /// is refused (`InvalidArgument`), one that can't be written as by
    /// [`Index::edit`]; either way the others are still written. `now` is
    /// when a task is marked done.
    pub fn edit_entities(
        &mut self,
        ids: &[Id],
        change: &Change,
        now: DateTime,
        changes: &mut Changes,
    ) -> Vec<(Id, Result<SaveOutcome>)> {
        ids.iter()
            .map(|id| {
                let outcome = match self.get(id) {
                    None => Err(Error::NotFound),
                    Some(doc) if !change.fits(doc.kind()) => Err(Error::InvalidArgument {
                        arg: "change".into(),
                    }),
                    Some(_) => self.edit(id, changes, |d| change.apply(d, now)),
                };
                (*id, outcome)
            })
            .collect()
    }
}
