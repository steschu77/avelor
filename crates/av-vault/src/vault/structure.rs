//! Membership in threads and projects, and dissolving what is left with one
//! member.

use super::{Changes, Index, SaveOutcome};
use av_utils::error::Result;
use av_utils::id::Id;

// ----------------------------------------------------------------------------
impl Index {
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
