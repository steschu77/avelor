//! Membership in threads and projects, and dissolving what is left with one
//! member.

use super::{Changes, Index, SaveOutcome};
use av_utils::error::Result;
use av_utils::id::Id;

// ----------------------------------------------------------------------------
impl Index {
    pub fn join_thread(
        &mut self,
        doc: &Id,
        thread: &Id,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_thread(thread)?;
        self.check_writable(doc)?;
        self.update(doc, changes, |d| d.join_thread(*thread))
    }

    /// Remove `doc` from the thread `thread`. A thread left with fewer than
    /// two members is dissolved: the remaining member leaves as well, and
    /// the thread's file is moved to `.trash` ([`Index::trash`]).
    pub fn leave_thread(
        &mut self,
        doc: &Id,
        thread: &Id,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_thread(thread)?;
        self.check_writable(doc)?;
        let dissolve = self.dissolving(thread, doc)?;
        let rewritten = [
            std::slice::from_ref(doc),
            dissolve.as_deref().unwrap_or_default(),
        ];
        if let Some(changed) = self.first_changed(&rewritten.concat())? {
            return Ok(SaveOutcome::Conflict(changed));
        }

        let outcome = self.update(doc, changes, |d| d.leave_thread(thread))?;
        match dissolve {
            Some(remaining) if !matches!(outcome, SaveOutcome::Conflict(_)) => {
                Ok(outcome.and(self.dissolve(thread, &remaining, changes)?))
            }
            _ => Ok(outcome),
        }
    }

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

    /// The members `thread` would be left with once `leaving` is gone, if
    /// that is fewer than two and so dissolves it; `None` if it survives. A
    /// dissolve rewrites those members, so they must be writable. A thread
    /// not in the index (a dangling reference) is never dissolved.
    pub(super) fn dissolving(&self, thread: &Id, leaving: &Id) -> Result<Option<Vec<Id>>> {
        if self.get(thread).is_none() {
            return Ok(None);
        }
        let remaining: Vec<Id> = self
            .thread_members(thread)
            .iter()
            .map(|e| e.doc.id())
            .filter(|id| id != leaving)
            .collect();
        if remaining.len() >= 2 {
            return Ok(None);
        }
        for id in &remaining {
            self.check_writable(id)?;
        }
        Ok(Some(remaining))
    }

    /// Dissolve `thread`: `remaining` leave it, then its file is trashed.
    /// Stops at the first conflict, leaving the thread in place.
    pub(super) fn dissolve(
        &mut self,
        thread: &Id,
        remaining: &[Id],
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        for id in remaining {
            if let conflict @ SaveOutcome::Conflict(_) =
                self.update(id, changes, |d| d.leave_thread(thread))?
            {
                return Ok(conflict);
            }
        }
        self.trash(thread, changes)?;
        Ok(SaveOutcome::Saved)
    }
}
