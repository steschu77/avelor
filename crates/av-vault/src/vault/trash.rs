//! The trash: entities deleted into it, restored, edited there and deleted for
//! good.

use std::fs;
use std::path::Path;

use super::{Changes, FileState, FileStep, InTrash, Index, load};
use crate::document::FileFormat;
use crate::schema::layout;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::fs::FileStamp;
use av_utils::id::Id;

// ----------------------------------------------------------------------------
impl Index {
    /// Move `id`'s file to `<root>/.trash/<its vault-relative path>` and drop
    /// it from the index. The indexing skips dot-folders, so it stays gone until
    /// it is restored ([`Index::restore`]). An already trashed file of the
    /// same name is kept: the new one gets a timestamp prefix.
    pub fn trash(&mut self, id: &Id, changes: &mut Changes) -> Result<()> {
        let entry = self.entry(id).ok_or(Error::NotFound)?;
        let (local, state) = (entry.local.clone(), FileState::of(&entry.doc));
        let trashed = self.move_to_trash(id, changes)?;
        changes.wrote(FileStep::trashed(*id, local, state, trashed));
        Ok(())
    }

    /// `trash`'s move: where the file went.
    pub(super) fn move_to_trash(&mut self, id: &Id, changes: &mut Changes) -> Result<InTrash> {
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
        let at = target
            .strip_prefix(&self.root)
            .unwrap_or(&target)
            .to_path_buf();
        let stamp = FileStamp::read(&target).ok().flatten();
        Ok(InTrash { at, stamp })
    }

    /// Whether a step's file is still in the trash as it went in.
    pub(super) fn in_trash(&self, trashed: &InTrash) -> bool {
        let path = self.root.join(&trashed.at);
        path.exists() && FileStamp::read(&path).ok().flatten() == trashed.stamp
    }

    /// Move a step's file, `id`'s, out of the trash to `local`,
    /// vault-relative, and into the index again, an undo or a redo of its
    /// move there.
    pub(super) fn out_of_trash(
        &mut self,
        id: &Id,
        trashed: &InTrash,
        local: &Path,
        changes: &mut Changes,
    ) -> Result<()> {
        let path = self.root.join(local);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(self.root.join(&trashed.at), &path)?;
        let readonly = fs::metadata(&path)?.permissions().readonly();
        let format = FileFormat::of_path(local).unwrap_or(FileFormat::Markdown);
        let mut entry = load(path, local.to_path_buf(), readonly, format)?;
        entry.doc.known_as(*id);
        self.insert(entry);
        changes.saved.push(*id);
        Ok(())
    }
}
