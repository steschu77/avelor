//! An entity's properties changed: its title, labels, links, a transaction's
//! fields, its body.

use super::{Changes, Index, SaveOutcome};
use av_utils::error::Result;
use av_utils::id::Id;
use av_utils::text::unique_trimmed;

// ----------------------------------------------------------------------------
impl Index {
    /// Set the title of `id`, which may be any entity. An empty title removes
    /// it ([`Document::set_title`]). Only the file's content changes; the file
    /// is not renamed to match.
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
}
