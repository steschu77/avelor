//! New entities written: a thread, a project, a document with its original
//! file.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{Changes, Entry, Index, SaveOutcome, original_file};
use crate::document::{Document, FileFormat};
use crate::envelope::DocumentType;
use crate::envelope::Original;
use crate::envelope::Thread;
use crate::envelope::{Empty, Project, Roles, Source};
use crate::filename;
use crate::schema::layout;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::fs::FileStamp;
use av_utils::id::Id;
use av_utils::text::{LineEnding, unique_trimmed};
use serde::Deserialize;

// ----------------------------------------------------------------------------
/// An entity to make in the app (New, [`Index::create_new`]): its role
/// blocks, which decide its type, and the well-known fields it starts with.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct NewEntity {
    pub roles: Roles,
    pub title: String,
    /// Not given: now; an event's is its `start`.
    pub date: Option<DateTime>,
    pub projects: Vec<Id>,
    pub labels: Vec<String>,
    /// The entity it is linked to (`linked-to`).
    pub linked_to: Option<Id>,
    /// The markdown body; a list or a record has none.
    pub content: String,
}

// ----------------------------------------------------------------------------
impl Index {
    // Structural operations: each validates everything it will touch first
    // -- including that no file to rewrite changed outside the app, so a
    // conflict writes nothing -- then changes and saves entity by entity,
    // recording each file it wrote in `changes`. A failed save restores that
    // entity's in-memory state, so the index always matches the disk.

    /// Create a new thread titled `title` with `members` as its members;
    /// its id is `changes.created`. It has no projects of its own: its
    /// projects are its members' (vault.md#thread). The thread's file gets
    /// its generated name (`filename::generated_name`) in the first
    /// member's folder, and is never written over an existing file.
    pub fn create_thread(
        &mut self,
        title: String,
        members: &[Id],
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        let Some(first) = members.first() else {
            return Err(Error::InvalidArgument {
                arg: "members".into(),
            });
        };
        for id in members {
            self.check_writable(id)?;
        }
        if let Some(changed) = self.first_changed(members)? {
            return Ok(SaveOutcome::Conflict(changed));
        }
        let folder = self.entry(first).and_then(|e| e.local.parent());
        let folder = folder.map(Path::to_path_buf).unwrap_or_default();

        let roles = Roles {
            thread: Some(Thread {}),
            ..Default::default()
        };
        let doc = Document::create(
            roles,
            Source::Created(Empty {}),
            title,
            Vec::new(),
            FileFormat::YAML,
        );
        let id = self.create(doc, &folder, changes)?;

        for member in members {
            if let conflict @ SaveOutcome::Conflict(_) =
                self.update(member, changes, |doc| doc.join_thread(id))?
            {
                return Ok(conflict);
            }
        }
        Ok(SaveOutcome::Saved)
    }

    /// Create a new project titled `title`, its block `project` (a
    /// blueprint's, or empty). Its file gets its generated name
    /// (`filename::generated_name`) in `Projects/`, the conventional folder
    /// (vault.md#vault-layout), and is markdown, so its body can serve as
    /// the project's readme.
    pub fn create_project(
        &mut self,
        title: String,
        project: Project,
        changes: &mut Changes,
    ) -> Result<Id> {
        let roles = Roles {
            project: Some(project),
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

    /// Make the entity `new` (New in the app): a document, task, event,
    /// contact, list or record, as its roles derive -- the other types have
    /// flows of their own, and are refused. It is created here (`created:
    /// {}`), a list or record as YAML without a body, the others as
    /// markdown, in the conventional folder of its type
    /// ([`layout`]). Its `date` is an event's start, else the one given,
    /// else now. Nothing is written if the title is empty, an event has no
    /// start or names a calendar that isn't one of the vault's, one of
    /// `projects` isn't a project, or `linked_to` isn't in the vault.
    pub fn create_new(&mut self, new: NewEntity, changes: &mut Changes) -> Result<Id> {
        let invalid = |arg: &str| Error::InvalidArgument { arg: arg.into() };
        let (format, folder) = match new.roles.derived_type() {
            Some(DocumentType::Document) => (FileFormat::Markdown, layout::DOCUMENTS),
            Some(DocumentType::Task) => (FileFormat::Markdown, layout::TASKS),
            Some(DocumentType::Event) => (FileFormat::Markdown, layout::CREATED_EVENTS),
            Some(DocumentType::Contact) => (FileFormat::Markdown, layout::CONTACTS),
            Some(DocumentType::List) => (FileFormat::YAML, layout::LISTS),
            Some(DocumentType::Record) => (FileFormat::YAML, layout::RECORDS),
            _ => return Err(invalid("roles")),
        };
        let title = new.title.trim().to_string();
        if title.is_empty() {
            return Err(invalid("title"));
        }
        let date = match &new.roles.event {
            Some(event) => Some(event.start.ok_or_else(|| invalid("event.start"))?),
            None => new.date,
        };
        let is_a = |id: &Id, kind| self.get(id).is_some_and(|d| d.kind() == kind);
        let calendar = new.roles.event.as_ref().and_then(|e| e.calendar);
        if calendar.is_some_and(|id| !is_a(&id, DocumentType::Calendar)) {
            return Err(invalid("event.calendar"));
        }
        let is_project = |id: &Id| is_a(id, DocumentType::Project);
        if !new.projects.iter().all(is_project) {
            return Err(invalid("projects"));
        }
        if new.linked_to.is_some_and(|to| self.get(&to).is_none()) {
            return Err(invalid("linked_to"));
        }

        let mut doc = Document::create(
            new.roles,
            Source::Created(Empty {}),
            title,
            new.projects,
            format,
        );
        if let Some(date) = date {
            doc.set_date(date);
        }
        doc.set_labels(unique_trimmed(new.labels));
        doc.set_link(new.linked_to);
        let body = new.content.trim_end();
        if format == FileFormat::Markdown && !body.is_empty() {
            doc.set_content(format!("{body}\n"));
        }
        self.create(doc, Path::new(folder), changes)
    }

    /// Write the new entity `doc` to a file with its generated name in
    /// `folder` (vault-relative), never over an existing file, and add it to
    /// the index. Without an open vault (no root) it is `InvalidPath`.
    pub fn create(&mut self, doc: Document, folder: &Path, changes: &mut Changes) -> Result<Id> {
        self.open_root()?;
        let local = folder.join(filename::generated_name(&doc));
        let path = self.root.join(&local);
        write_new(&path, doc.save().as_bytes())?;
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

    /// Write the new document `doc` with its original (vault.md#document):
    /// `bytes`, the file that came as `name`. The file is written first,
    /// named after the document ([`filename::original_name`]), then the
    /// document as [`Index::create`] writes it; if that fails, the file is
    /// removed again. Flags of the original `doc` has already
    /// (`Original::encrypted`) are kept.
    pub fn create_with_original(
        &mut self,
        mut doc: Document,
        folder: &Path,
        name: &str,
        bytes: &[u8],
        changes: &mut Changes,
    ) -> Result<Id> {
        self.open_root()?;
        let mut original = Original {
            file: None,
            name: Some(name.to_string()),
            size: Some(bytes.len() as u64),
            ..doc.original().cloned().unwrap_or_default()
        };
        // Named once the role is set: the role decides the type in the name.
        doc.set_original(original.clone());
        let ext = Path::new(name).extension().and_then(|e| e.to_str());
        let file = filename::original_name(&doc, ext.unwrap_or_default());
        let path = self.root.join(folder).join(&file);
        write_new(&path, bytes)?;
        original.file = Some(file);
        doc.set_original(original);
        self.create(doc, folder, changes).inspect_err(|_| {
            fs::remove_file(&path).ok();
        })
    }

    /// Where `id`'s original is: its file next to the entity's
    /// ([`original_file`]); `None` without one.
    pub fn original_path(&self, id: &Id) -> Option<PathBuf> {
        let entry = self.entry(id)?;
        Some(entry.path.with_file_name(original_file(&entry.doc)?))
    }
}

// ----------------------------------------------------------------------------
/// Write `bytes` to a new file at `path`, creating its folder; fails rather
/// than replace a file that already exists. A failure is
/// `Error::FileWrite` with `path`.
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let written = (|| {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(bytes)
    })();
    written.map_err(|err| Error::FileWrite {
        path: path.to_path_buf(),
        err,
    })
}
