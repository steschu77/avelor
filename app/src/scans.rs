//! Import scans (import.md#import-scans): the files of an images folder watch.
//!
//! The page lists the files; the commands of the typed protocol
//! (api.md#scans).
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

use crate::app::App;
use crate::app::Task;
use crate::protocol::Req;
use crate::sys;
use crate::texts::Text;
use crate::watches::{self, FolderFile};

mod previews;

pub(crate) use previews::{PreviewRequest, scan_previews};

// ----------------------------------------------------------------------------
/// What the app keeps for Import scans.
#[derive(Default)]
pub struct ScanState {
    /// Each file's preview, by its path: a JPEG `data:` URL, or empty if
    /// the file couldn't be decoded.
    previews: HashMap<PathBuf, (Stamp, String)>,
    /// The previews being made; a newer request aborts them.
    preview_batch: Option<(Id, Task)>,
}

impl ScanState {
    // ------------------------------------------------------------------------
    /// The preview of `file`, if it was made of the file as it is now.
    fn preview(&self, file: &FolderFile) -> Option<&str> {
        let (stamp, url) = self.previews.get(&file.path)?;
        stamp.matches(file).then_some(url.as_str())
    }
}

// ----------------------------------------------------------------------------
/// A file as it was when something was made of it (its preview):
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
}

// ----------------------------------------------------------------------------
/// `scan_list`'s answer.
#[derive(Serialize)]
struct Listing<'a> {
    listed: DateTime,
    files: Vec<FileInfo<'a>>,
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
/// `scan_list {watch}`: list the watch's files on a worker. Answers
/// `{listed, files}`, each file with what is known of it.
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
    let files = files
        .iter()
        .map(|file| FileInfo {
            file: &file.name,
            size: file.size,
            taken: file.taken,
            modified: DateTime::from_system_time(file.modified),
            in_vault: in_vault.get(&file.name).copied(),
        })
        .collect();
    Listing {
        listed: DateTime::now(),
        files,
    }
}

// ----------------------------------------------------------------------------
/// `scan_open {watch, file}`: open the file in its default app (an image in
/// the Photos app), once the handler has returned (`ShellExecuteW` may run
/// a modal loop, as for `open_url`); only a file inside the watch's folder.
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
