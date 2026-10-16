//! Membership in threads and projects, and dissolving what is left with one
//! member.

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

    pub fn join_project(
        &mut self,
        doc: &Id,
        project: &Id,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_project(project)?;
        self.check_writable(doc)?;
        self.update(doc, changes, |d| d.join_project(*project))
    }

    pub fn leave_project(
        &mut self,
        doc: &Id,
        project: &Id,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_project(project)?;
        self.check_writable(doc)?;
        self.update(doc, changes, |d| d.leave_project(project))
    }
}
