//! The trash: entities deleted into it, restored, edited there and deleted for
//! good.

use std::fs;

use super::{Changes, Index, SaveOutcome};
use crate::envelope::DocumentType;
use crate::schema::layout;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::id::Id;

// ----------------------------------------------------------------------------
impl Index {
    /// Delete the entity `id`: its file moves to `.trash` ([`Index::trash`])
    /// unchanged, so a restore ([`Index::restore`]) brings it back as it was
    /// -- with any change made outside the app. Every thread it leaves behind with
    /// fewer than two members is dissolved, as in [`Index::leave_thread`];
    /// a project's members leave it first, so nothing refers to it any
    /// more. The files rewritten must be writable and unchanged on disk, or
    /// nothing is written. Threads aren't deleted this way -- they dissolve
    /// when their members leave.
    pub fn delete(&mut self, id: &Id, changes: &mut Changes) -> Result<SaveOutcome> {
        let Some(entry) = self.entry(id) else {
            return Err(Error::NotFound);
        };
        let kind = entry.doc.kind();
        if kind == DocumentType::Thread {
            return Err(Error::InvalidArgument { arg: "id".into() });
        }
        if !entry.is_deletable() {
            return Err(Error::NotWritable {
                path: entry.local.clone(),
            });
        }
        let threads = entry
            .doc
            .roles()
            .message
            .as_ref()
            .map(|m| m.threads.clone())
            .unwrap_or_default();
        let mut dissolve = Vec::new();
        for thread in threads {
            if let Some(remaining) = self.dissolving(&thread, id)? {
                dissolve.push((thread, remaining));
            }
        }
        let leaving: Vec<Id> = match kind {
            DocumentType::Project => self.members_of(id).iter().map(|e| e.doc.id()).collect(),
            _ => Vec::new(),
        };
        for member in &leaving {
            self.check_writable(member)?;
        }
        let rewritten: Vec<Id> = dissolve
            .iter()
            .flat_map(|(_, r)| r.iter().copied())
            .chain(leaving.iter().copied())
            .collect();
        if let Some(changed) = self.first_changed(&rewritten)? {
            return Ok(SaveOutcome::Conflict(changed));
        }

        for member in &leaving {
            if let conflict @ SaveOutcome::Conflict(_) =
                self.update(member, changes, |d| d.leave_project(id))?
            {
                return Ok(conflict);
            }
        }
        for (thread, remaining) in &dissolve {
            if let conflict @ SaveOutcome::Conflict(_) =
                self.dissolve(thread, remaining, changes)?
            {
                return Ok(conflict);
            }
        }
        self.trash(id, changes)?;
        Ok(SaveOutcome::Saved)
    }

    /// Move `id`'s file to `<root>/.trash/<its vault-relative path>` and drop
    /// it from the index. The indexing skips dot-folders, so it stays gone until
    /// it is restored ([`Index::restore`]). An already trashed file of the
    /// same name is kept: the new one gets a timestamp prefix.
    pub fn trash(&mut self, id: &Id, changes: &mut Changes) -> Result<()> {
        let Some(entry) = self.entry(id) else {
            return Err(Error::NotFound);
        };
        if entry.is_unsaved() {
            return Err(Error::Unsaved);
        }
        let mut target = self.root.join(layout::TRASH).join(&entry.local);
        if target.exists() {
            let name = entry
                .local
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            let stamp = DateTime::now().as_timestamp();
            target.set_file_name(format!("{stamp}.{name}"));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&entry.path, &target)?;
        self.remove(id);
        changes.trashed.push(*id);
        Ok(())
    }
}
