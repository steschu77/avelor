//! New entities written: a thread, a project, a document with its original
//! file.

use std::fs;
use std::io::Write;
use std::path::Path;

use super::{Changes, Entry, Index};
use crate::document::{Document, FileFormat};
use crate::envelope::{Empty, Project, Roles, Source};
use crate::filename;
use crate::schema::layout;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::fs::FileStamp;
use av_utils::id::Id;
use av_utils::text::LineEnding;

// ----------------------------------------------------------------------------
impl Index {
    // Structural operations: each validates everything it will touch first
    // -- including that no file to rewrite changed outside the app, so a
    // conflict writes nothing -- then changes and saves entity by entity,
    // recording each file it wrote in `changes`. A failed save restores that
    // entity's in-memory state, so the index always matches the disk.

    /// Create a new project titled `title`. Its file gets its generated
    /// name (`filename::generated_name`) in `Projects/`, the conventional
    /// folder (vault.md#vault-layout), and is markdown, so its body
    /// can serve as the project's readme.
    pub fn create_project(&mut self, title: String, changes: &mut Changes) -> Result<Id> {
        let roles = Roles {
            project: Some(Project {}),
            ..Default::default()
        };
        let doc = Document::create(
            roles,
            Source::Created(Empty {}),
            title,
            Vec::new(),
            FileFormat::Markdown,
        );
        self.create(doc, Path::new(layout::PROJECTS), changes)
    }

    /// Write the new entity `doc` to a file with its generated name in
    /// `folder` (vault-relative), never over an existing file, and add it to
    /// the index. Without an open vault (no root) it is `InvalidPath`.
    pub fn create(&mut self, doc: Document, folder: &Path, changes: &mut Changes) -> Result<Id> {
        self.open_root()?;
        let local = folder.join(filename::generated_name(&doc));
        let path = self.root.join(&local);
        write_new(&path, &doc.save())?;
        let stamp = FileStamp::read(&path).ok().flatten();

        let id = doc.id();
        let mut entry = Entry {
            path,
            local,
            readonly: false,
            bom: String::new(),
            crlf: LineEnding::default(),
            written: true,
            doc,
        };
        entry.doc.written(stamp);
        self.insert(entry);

        changes.created = Some(id);
        Ok(id)
    }
}

// ----------------------------------------------------------------------------
/// Write `text` to a new file at `path`, creating its folder; fails rather
/// than replace a file that already exists. A failure is
/// `Error::FileWrite` with `path`.
fn write_new(path: &Path, text: &str) -> Result<()> {
    let written = (|| {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(text.as_bytes())
    })();
    written.map_err(|err| Error::FileWrite {
        path: path.to_path_buf(),
        err,
    })
}
