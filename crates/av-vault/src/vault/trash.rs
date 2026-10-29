//! The trash: entities deleted into it, restored, edited there and deleted for
//! good.

use std::fs;
use std::path::Component;
use std::path::{Path, PathBuf};

use super::{
    Changes, Entry, FileState, FileStep, Found, InTrash, Index, SaveOutcome, load, original_file,
    walk,
};
use crate::document::{Document, FileFormat};
use crate::envelope::DocumentType;
use crate::schema::layout;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::fs::FileStamp;
use av_utils::fs::write_atomic;
use av_utils::id::Id;

// ----------------------------------------------------------------------------
/// A file in the trash ([`Index::trashed`], vault.md#trash): where it
/// is, relative to `.trash` -- its vault-relative path, unless a clash gave
/// it a prefix -- and what it holds, or why it couldn't be read.
#[derive(Debug)]
pub struct Trashed {
    pub local: PathBuf,
    pub doc: std::result::Result<Document, String>,
}

impl Trashed {
    /// The date of what it holds, if it could be read and has one.
    pub fn date(&self) -> Option<DateTime> {
        self.doc.as_ref().ok().and_then(Document::date)
    }
}

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
    /// it from the index; its original goes along, next to it. The indexing
    /// skips dot-folders, so it stays gone until it is restored
    /// ([`Index::restore`]). An already trashed file of the same name is
    /// kept: the new ones get a timestamp prefix.
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
        let original = original_file(&entry.doc)
            .map(|file| (entry.path.with_file_name(file), file.to_string()))
            .filter(|(path, _)| path.is_file());
        let mut prefix = String::new();
        let taken = |file: &str| target.with_file_name(file).exists();
        if target.exists() || original.as_ref().is_some_and(|(_, file)| taken(file)) {
            let name = entry
                .local
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            prefix = format!("{}.", DateTime::now().as_timestamp());
            target.set_file_name(format!("{prefix}{name}"));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let original = original.map(|(path, file)| {
            let trashed = target.with_file_name(format!("{prefix}{file}"));
            (path, trashed)
        });
        move_with_original(&entry.path, &target, original.as_ref())?;
        if let Some(entry) = self.remove(id) {
            changes.trashed_files.push(entry.local);
        }
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

    /// Whether the original of a step's file in the trash, if it has one
    /// there, can go back next to `local`: nothing is in its place.
    pub(super) fn original_free(&self, trashed: &InTrash, local: &Path) -> bool {
        let from = self.root.join(&trashed.at);
        let format = FileFormat::of_path(local).unwrap_or(FileFormat::Markdown);
        let Ok(entry) = load(from.clone(), local.to_path_buf(), false, format) else {
            return false;
        };
        trashed_original(&entry.doc, &from, &self.root.join(local))
            .is_none_or(|(_, back)| !back.exists())
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
        let from = self.root.join(&trashed.at);
        let readonly = fs::metadata(&from)?.permissions().readonly();
        let format = FileFormat::of_path(local).unwrap_or(FileFormat::Markdown);
        let mut entry = load(from.clone(), local.to_path_buf(), readonly, format)?;
        entry.doc.known_as(*id);
        let original = trashed_original(&entry.doc, &from, &self.root.join(local));
        changes.saved.push(entry.doc.id());
        self.out_of(&from, entry, original.as_ref())?;
        Ok(())
    }

    /// The files in the trash, newest first: the markdown and YAML files
    /// under `<root>/.trash`, read as the indexing reads the vault.
    pub fn trashed(&self) -> Vec<Trashed> {
        let mut files: Vec<Trashed> = self
            .trash_files()
            .into_iter()
            .map(|found| Trashed {
                doc: load(
                    found.path,
                    found.local.clone(),
                    found.readonly,
                    found.format,
                )
                .map(|entry| entry.doc)
                .map_err(|err| err.to_string()),
                local: found.local,
            })
            .collect();
        files.sort_by(|a, b| b.date().cmp(&a.date()).then_with(|| a.local.cmp(&b.local)));
        files
    }

    /// Move the trashed file `local` (relative to `.trash`, as
    /// [`Index::trashed`] lists it) back to where it was -- its name without
    /// the prefix [`Index::trash`] adds on a clash -- with its original, and
    /// add it to the index; returns its id. Refused with
    /// [`Error::FileExists`] when a file is in its place or its original's,
    /// or an entity with the file's `id` is in the vault. The trash folders
    /// this leaves empty are removed.
    pub fn restore(&mut self, local: &Path, changes: &mut Changes) -> Result<Id> {
        let (from, format) = self.trashed_file(local)?;
        let target = untrashed(local);
        let to = self.root.join(&target);
        if to.try_exists()? {
            return Err(Error::FileExists { path: target });
        }
        let readonly = fs::metadata(&from)?.permissions().readonly();
        // Read where it is: the move keeps its stamp (time and size).
        let entry = load(from.clone(), target.clone(), readonly, format)?;
        let id = entry.doc.id();
        if entry.doc.adopted()
            && let Some(other) = self.entry(&id)
        {
            return Err(Error::FileExists {
                path: other.local.clone(),
            });
        }
        let original = trashed_original(&entry.doc, &from, &to);
        if let Some((_, back)) = &original
            && back.try_exists()?
        {
            let name = back.file_name().unwrap_or_default();
            return Err(Error::FileExists {
                path: target.with_file_name(name),
            });
        }
        let state = FileState::of(&entry.doc);
        let trashed = InTrash {
            at: from.strip_prefix(&self.root).unwrap_or(&from).to_path_buf(),
            stamp: entry.doc.stamp(),
        };
        self.out_of(&from, entry, original.as_ref())?;
        changes.wrote(FileStep::restored(id, target, trashed, state));
        changes.created = Some(id);
        Ok(id)
    }

    /// Move the trashed file `from` back to its place, `entry`'s, with its
    /// `original` (`trashed_original`), and `entry`, read where it was, into
    /// the index: the move keeps its stamp. Folders it leaves empty in the
    /// trash go.
    fn out_of(
        &mut self,
        from: &Path,
        mut entry: Entry,
        original: Option<&(PathBuf, PathBuf)>,
    ) -> Result<()> {
        let to = self.root.join(&entry.local);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent)?;
        }
        move_with_original(from, &to, original)?;
        prune_empty(&self.root.join(layout::TRASH), from.parent());
        entry.path = to;
        self.insert(entry);
        Ok(())
    }

    /// Apply `change` to the document of the trashed file `local` and write
    /// it back if that changed anything -- e.g. a calendar sync marking an
    /// event it deleted in Google. The file stays in the trash; a read-only
    /// one, or one whose frontmatter didn't parse, isn't written
    /// ([`Error::NotWritable`]).
    pub fn edit_trashed(
        &self,
        local: &Path,
        change: impl FnOnce(&mut Document),
    ) -> Result<SaveOutcome> {
        let (path, format) = self.trashed_file(local)?;
        let readonly = fs::metadata(&path)?.permissions().readonly();
        let mut entry = load(path, local.to_path_buf(), readonly, format)?;
        if !entry.is_writable() {
            return Err(Error::NotWritable {
                path: local.to_path_buf(),
            });
        }
        change(&mut entry.doc);
        if !entry.doc.is_dirty() {
            return Ok(SaveOutcome::Unchanged);
        }
        write_atomic(&entry.path, entry.file_text().as_bytes())?;
        Ok(SaveOutcome::Saved)
    }

    /// Delete the trashed file `local` for good, also a read-only one, with
    /// its original, and the trash folders this leaves empty.
    pub fn delete_trashed(&self, local: &Path) -> Result<()> {
        let (path, format) = self.trashed_file(local)?;
        let original = load(path.clone(), local.to_path_buf(), false, format)
            .ok()
            .and_then(|entry| {
                let file = original_file(&entry.doc)?;
                Some(path.with_file_name(format!("{}{file}", trash_prefix(local))))
            })
            .filter(|original| original.is_file());
        if let Some(original) = original {
            remove_for_good(&original)?;
        }
        remove_for_good(&path)?;
        prune_empty(&self.root.join(layout::TRASH), path.parent());
        Ok(())
    }

    /// Delete every file [`Index::trashed`] lists for good, and the folders
    /// this leaves empty; anything else in the trash stays. Returns how many
    /// files were deleted, and stops at the first that can't be.
    pub fn empty_trash(&self) -> Result<usize> {
        let files = self.trash_files();
        for found in &files {
            self.delete_trashed(&found.local)?;
        }
        Ok(files.len())
    }

    /// The markdown and YAML files under `<root>/.trash`, as the indexing's walk
    /// finds them, `local` relative to the trash. None without a vault.
    fn trash_files(&self) -> Vec<Found> {
        let mut files = Vec::new();
        if self.open_root().is_err() {
            return files;
        }
        walk(
            &self.root.join(layout::TRASH),
            Path::new(""),
            &mut |found| files.push(found),
        );
        files
    }

    /// The trashed file `local` and its format: `local` must be a relative
    /// path that stays inside the trash ([`Error::InvalidPath`]) and name a
    /// markdown or YAML file ([`Error::InvalidFileType`]) that is there
    /// ([`Error::FileNotFound`]). Without a vault, every path is invalid.
    fn trashed_file(&self, local: &Path) -> Result<(PathBuf, FileFormat)> {
        let inside = local
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
        if self.open_root().is_err() || local.as_os_str().is_empty() || !inside {
            return Err(Error::InvalidPath {
                path: local.to_path_buf(),
            });
        }
        let Some(format) = FileFormat::of_path(local) else {
            return Err(Error::InvalidFileType);
        };
        let path = self.root.join(layout::TRASH).join(local);
        if !path.is_file() {
            return Err(Error::FileNotFound {
                path: local.to_path_buf(),
            });
        }
        Ok((path, format))
    }
}

// ----------------------------------------------------------------------------
/// Move the file `from` to `to` with its `original`, `(from, to)`: the
/// original first, and back if the file can't follow, so a failure leaves
/// both where they were.
fn move_with_original(from: &Path, to: &Path, original: Option<&(PathBuf, PathBuf)>) -> Result<()> {
    if let Some((original_from, original_to)) = original {
        fs::rename(original_from, original_to)?;
    }
    if let Err(err) = fs::rename(from, to) {
        if let Some((original_from, original_to)) = original {
            fs::rename(original_to, original_from).ok();
        }
        return Err(err.into());
    }
    Ok(())
}

// ----------------------------------------------------------------------------
/// The original `doc` names, in the trash next to its file `from` (with the
/// same prefix, [`trash_prefix`]), and its place next to `to`; `None` if it
/// has none there.
fn trashed_original(doc: &Document, from: &Path, to: &Path) -> Option<(PathBuf, PathBuf)> {
    let file = original_file(doc)?;
    let trashed = from.with_file_name(format!("{}{file}", trash_prefix(from)));
    trashed
        .is_file()
        .then(|| (trashed, to.with_file_name(file)))
}

// ----------------------------------------------------------------------------
/// Delete the file at `path`, also a read-only one: only the trash's
/// files, about to be deleted for good.
fn remove_for_good(path: &Path) -> Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    if permissions.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)?;
    }
    fs::remove_file(path)?;
    Ok(())
}

// ----------------------------------------------------------------------------
/// Where the trashed file `local` goes back to: its vault-relative path,
/// without the prefix [`Index::trash`] adds on a clash ([`trash_prefix`]).
pub(super) fn untrashed(local: &Path) -> PathBuf {
    let prefix = trash_prefix(local);
    if prefix.is_empty() {
        return local.to_path_buf();
    }
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    local.with_file_name(&name[prefix.len()..])
}

/// The `YYYYMMDD_hhmmss.` prefix [`Index::trash`] gives the trashed file
/// `local` on a clash, with its dot; empty without one. The names the app
/// gives files start `YYYYMMDDThhmm`, never like that.
pub(super) fn trash_prefix(local: &Path) -> &str {
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    match name.split_once('.') {
        Some((stamp, rest)) if is_trash_stamp(stamp) && !rest.is_empty() => &name[..=stamp.len()],
        _ => "",
    }
}

/// Whether `text` is a timestamp as [`DateTime::as_timestamp`] writes one
/// for a moment: `YYYYMMDD_hhmmss`.
fn is_trash_stamp(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 15
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 8 {
                *b == b'_'
            } else {
                b.is_ascii_digit()
            }
        })
}

// ----------------------------------------------------------------------------
/// Remove the folder `dir` and its parents below `top` while they are
/// empty; `fs::remove_dir` fails on one that isn't, which ends it.
fn prune_empty(top: &Path, mut dir: Option<&Path>) {
    while let Some(d) = dir {
        if d == top || !d.starts_with(top) || fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}
