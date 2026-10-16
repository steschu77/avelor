//! The trash: entities deleted into it, restored, edited there and deleted for
//! good.

use std::fs;

use super::{Changes, Index};
use crate::schema::layout;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::id::Id;

// ----------------------------------------------------------------------------
impl Index {
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
