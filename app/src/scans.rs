//! Import scans (import.md#import-scans): the files of a folder watch, images
//! or PDFs, read through `reader` (an image and a PDF's scanned pages by an AI
//! account's model, a PDF's text layer without one) and imported as documents
//! into `Scans/`.
//!
//! The page lists the files, has the ticked ones read (a job reading one file
//! after the other, which `cancel_job` stops), arranges the pages into
//! documents and imports them; the commands of the typed protocol
//! (api.md#scans). What a file read to is kept in memory while the app runs.
//! Every document written carries `scan: {watch, origin: [files]}`, which is
//! how a later listing tells the files already in the vault
//! (`Index::scan_origins`). A watch's analysis takes the documents an import
//! wrote (`analysis::analyse`), and with Keep original a document of one file
//! keeps the file beside it (`document.original`), one of several images
//! (JPEG, PNG) a PDF of them; an encrypted PDF always keeps its file, as it has no text.
//!
//! The page shows the files as previews: `scan_previews` renders them
//! (`sys::preview`: WIC on Windows, gdk-pixbuf on Linux) on a worker and keeps
//! them while the app runs; `scan_open` opens a file in its default app.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::id::Id;
use serde::Deserialize;
use serde::Serialize;

use crate::analysis;
use crate::api::may_open;
use crate::app::App;
use crate::app::Task;
use crate::protocol::Req;
use crate::sys;
use crate::texts::Text;
use crate::watches::{self, FolderFile, WatchSettings};
use av_utils::is_false;
use read::ScanRead;

mod import;
mod previews;
mod read;

pub(crate) use import::{ImportRequest, scan_import};
pub(crate) use previews::{PreviewRequest, scan_previews};
pub(crate) use read::{ReadRequest, scan_read, stop_read};

// ----------------------------------------------------------------------------
/// What the app keeps for Import scans.
#[derive(Default)]
pub struct ScanState {
    /// What each file read to, by its path.
    cache: HashMap<PathBuf, Ocr>,
    /// The read in progress; one at a time.
    read: Option<ScanRead>,
    /// Each file's preview, by its path: a JPEG `data:` URL, or empty if
    /// the file couldn't be decoded.
    previews: HashMap<PathBuf, (Stamp, String)>,
    /// The previews being made; a newer request aborts them.
    preview_batch: Option<(Id, Task)>,
}

impl ScanState {
    // ------------------------------------------------------------------------
    /// What `file` read to, if it was read as it is now.
    fn cached(&self, file: &FolderFile) -> Option<&Ocr> {
        self.cache
            .get(&file.path)
            .filter(|ocr| ocr.stamp.matches(file))
    }

    // ------------------------------------------------------------------------
    /// The preview of `file`, if it was made of the file as it is now.
    fn preview(&self, file: &FolderFile) -> Option<&str> {
        let (stamp, url) = self.previews.get(&file.path)?;
        stamp.matches(file).then_some(url.as_str())
    }

    // ------------------------------------------------------------------------
    /// The job reading the files of `watch`, if one is running.
    fn job_of(&self, watch: Id) -> Option<Id> {
        self.read
            .as_ref()
            .filter(|r| r.watch == watch)
            .map(|r| r.job)
    }

    // ------------------------------------------------------------------------
    /// `queued` or `reading` while the read of `watch` has the file `name`.
    fn status(&self, watch: Id, name: &str) -> Option<&'static str> {
        let read = self.read.as_ref().filter(|r| r.watch == watch)?;
        if read.queue.current().is_some_and(|current| current == name) {
            Some("reading")
        } else if read.queue.waiting().any(|queued| queued == name) {
            Some("queued")
        } else {
            None
        }
    }
}

// ----------------------------------------------------------------------------
/// A file as it was when something was made of it (its text, its preview):
/// what was made is valid while the file still is.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stamp {
    size: u64,
    modified: SystemTime,
}

impl Stamp {
    // ------------------------------------------------------------------------
    fn of(file: &FolderFile) -> Self {
        Self {
            size: file.size,
            modified: file.modified,
        }
    }

    // ------------------------------------------------------------------------
    fn matches(&self, file: &FolderFile) -> bool {
        *self == Self::of(file)
    }
}

// ----------------------------------------------------------------------------
/// A file's text, as it was read.
struct Ocr {
    /// The file as it was read.
    stamp: Stamp,
    text: String,
    words: usize,
    title: String,
    /// An encrypted PDF: no text, imported with the file beside it.
    encrypted: bool,
}

// ----------------------------------------------------------------------------
/// One file of `scan_list`'s answer.
#[derive(Serialize)]
struct FileInfo<'a> {
    file: &'a str,
    size: u64,
    taken: DateTime,
    /// Tells a changed file (and its preview) from the one before.
    modified: DateTime,
    /// The document the file was imported into.
    in_vault: Option<Id>,
    /// What it read to, if it was read as it is now.
    read: Option<PageRead<'a>>,
    /// `queued` or `reading` while a read has it.
    status: Option<&'static str>,
}

// ----------------------------------------------------------------------------
#[derive(Serialize)]
struct PageRead<'a> {
    words: usize,
    title: &'a str,
    #[serde(skip_serializing_if = "is_false")]
    encrypted: bool,
}

// ----------------------------------------------------------------------------
/// `scan_list`'s answer.
#[derive(Serialize)]
struct Listing<'a> {
    listed: DateTime,
    files: Vec<FileInfo<'a>>,
    /// The job reading this watch's files, if one is running.
    job: Option<Id>,
}

// ----------------------------------------------------------------------------
/// `scan_open`'s args.
#[derive(Deserialize)]
pub(crate) struct OpenRequest {
    watch: Id,
    file: String,
}

// ----------------------------------------------------------------------------
/// Refuse a request that does nothing, with `message` for the page.
fn refuse(req: Req, error: Error, text: Text) {
    log::info!("Scan request refused: {text}");
    req.fail(error, text);
}

// ----------------------------------------------------------------------------
/// Refuse to read or import a statements watch's files as scans: they go
/// into their account (`bank::statements_import`).
fn refuse_statements(req: Req) {
    let what = "reading statements as scans".to_string();
    let text = Text::new("scans.statements_watch");
    refuse(req, Error::Unsupported { what }, text);
}

// ----------------------------------------------------------------------------
/// Whether the analysis of the watch of `settings` can run, if it is on;
/// else the refused pick: a watch neither reads nor imports while its
/// analysis waits for a model that works, as Fetch email doesn't fetch.
fn analysis_works(app: &App, settings: &WatchSettings) -> Result<(), (Error, Text)> {
    let Some(analysis) = &settings.analysis else {
        return Ok(());
    };
    let what = analysis::scan_analysis(settings.kind);
    analysis
        .check(&app.ai, what)
        .map_err(|refusal| (refusal.error, refusal.text))
}

// ----------------------------------------------------------------------------
/// `scan_list {watch}`: list the watch's files on a worker. Answers
/// `{listed, files, job}`, each file with what is known of it, and the job
/// reading this watch's files, if one is running.
pub fn scan_list(app: &mut App, req: Req, id: Id) {
    let settings = match app.watches.get(id) {
        Ok(watch) => watch.settings().clone(),
        Err(e) => return req.err(e),
    };
    app.run(
        move |_cancel| watches::list_files(&settings),
        move |app, result| match result {
            // Removed meanwhile, or another vault opened.
            Ok(_) if app.watches.get(id).is_err() => req.err(Error::NotFound),
            Ok(files) => req.ok(listing(app, id, &files)),
            Err(Error::FileNotFound { path }) => {
                let text = Text::new("watch.no_folder").arg("path", path.display().to_string());
                refuse(req, Error::FileNotFound { path }, text);
            }
            Err(e) => {
                log::warn!("Could not list the folder of the watch {id}: {e}");
                refuse(req, e, Text::new("watch.list_failed"));
            }
        },
    );
}

// ----------------------------------------------------------------------------
/// `scan_list`'s answer for the files of `watch`.
fn listing<'a>(app: &'a App, watch: Id, files: &'a [FolderFile]) -> Listing<'a> {
    let in_vault = app.vault.index().scan_origins(&watch);
    let scans = &app.scans;
    let files = files
        .iter()
        .map(|file| FileInfo {
            file: &file.name,
            size: file.size,
            taken: file.taken,
            modified: DateTime::from_system_time(file.modified),
            in_vault: in_vault.get(&file.name).copied(),
            read: scans.cached(file).map(|ocr| PageRead {
                words: ocr.words,
                title: &ocr.title,
                encrypted: ocr.encrypted,
            }),
            status: scans.status(watch, &file.name),
        })
        .collect();
    Listing {
        listed: DateTime::now(),
        files,
        job: scans.job_of(watch),
    }
}

// ----------------------------------------------------------------------------
/// `scan_open {watch, file}`: open the file in its default app (an image in
/// the Photos app), once the handler has returned (`ShellExecuteW` may run
/// a modal loop, as for `open_url`); only a file inside the watch's folder,
/// and only a PDF or an image (`api::may_open`): the folder may be
/// Downloads.
pub fn scan_open(app: &mut App, req: Req, request: OpenRequest) {
    let root = match app.watches.get(request.watch) {
        Ok(watch) => watch.settings().root(),
        Err(e) => return req.err(e),
    };
    let name = request.file;
    let path = root.join(&name);
    if !inside(&root, &path) {
        let text = Text::new("scans.outside").arg("name", name.as_str());
        return refuse(req, Error::InvalidPath { path }, text);
    }
    if !may_open(&path) {
        let arg = format!("a type the app doesn't open: {name}");
        let text = Text::new("open.other_type").arg("name", name.as_str());
        return refuse(req, Error::InvalidArgument { arg }, text);
    }
    app.on_ui(
        move |_| sys::open_path(&path),
        move |_, opened| match opened {
            Ok(()) => req.ok(()),
            Err(e) => {
                log::warn!("Could not open {name}: {e}");
                refuse(
                    req,
                    e,
                    Text::new("open.file_failed").arg("name", name.as_str()),
                );
            }
        },
    );
}

// ----------------------------------------------------------------------------
/// Whether `path` is a file in `root` or in a folder below it, once both
/// are resolved (no `..` out of it, no link to elsewhere).
fn inside(root: &Path, path: &Path) -> bool {
    match (root.canonicalize(), path.canonicalize()) {
        (Ok(root), Ok(path)) => path.starts_with(root) && path.is_file(),
        _ => false,
    }
}

#[cfg(test)]
mod tests;
