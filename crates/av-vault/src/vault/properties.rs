//! An entity's properties changed: its title, labels, links, a transaction's
//! fields, its body.

use super::{Changes, Index, SaveOutcome};
use av_utils::error::Result;
use av_utils::id::Id;

// ----------------------------------------------------------------------------
impl Index {
    // Structural operations: each validates everything it will touch first
    // -- including that no file to rewrite changed outside the app, so a
    // conflict writes nothing -- then changes and saves entity by entity,
    // recording each file it wrote in `changes`. A failed save restores that
    // entity's in-memory state, so the index always matches the disk.

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
