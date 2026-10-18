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

use av_utils::base64::to_base64;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::id::Id;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::app::App;
use crate::app::Task;
use crate::protocol::Req;
use crate::sys;
use crate::watches::{self, FolderFile};

/// The previews' size: twice the page's large preview (180 x 254), so they
/// stay sharp on a high-DPI screen.
const PREVIEW_W: u32 = 360;
const PREVIEW_H: u32 = 508;

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
/// `scan_previews`' args: the files, in the order the page wants them.
#[derive(Deserialize)]
pub(crate) struct PreviewRequest {
    watch: Id,
    files: Vec<String>,
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
fn refuse(req: Req, error: Error, message: impl Into<String>) {
    let message = message.into();
    log::info!("Scan request refused: {message}");
    req.fail(error, message);
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
                let message = format!("There's no folder at {}.", path.display());
                refuse(req, Error::FileNotFound { path }, message);
            }
            Err(e) => {
                log::warn!("Could not list the folder of the watch {id}: {e}");
                refuse(req, e, "Could not list the folder.");
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
/// The preview of `file` of `watch`: the `scan_preview` event; `url` is a
/// JPEG `data:` URL, or empty if the file has no preview.
fn post_preview(app: &App, watch: Id, file: &str, url: &str) {
    let preview = json!({ "watch": watch, "file": file, "url": url });
    app.post_event("scan_preview", preview);
}

// ----------------------------------------------------------------------------
/// `scan_previews {watch, files}`: previews of the files, at most
/// `PREVIEW_W` x `PREVIEW_H` and upright, a `scan_preview` event per file
/// in the order asked. Previews made before (of the files as they are now)
/// come at once; the others are made on a worker, one after the other. A
/// newer request stops what is left of the one before. Answered once the
/// work is queued.
pub fn scan_previews(app: &mut App, req: Req, request: PreviewRequest) {
    let (watch, root) = match app.watches.get(request.watch) {
        Ok(watch) => (watch.id(), watch.settings().root()),
        Err(e) => return req.err(e),
    };
    req.ok(());
    if let Some((_, task)) = app.scans.preview_batch.take() {
        task.abort();
    }

    let mut todo = Vec::new();
    for name in request.files {
        let Ok(file) = FolderFile::read(&root, &name) else {
            post_preview(app, watch, &name, "");
            continue;
        };
        match app.scans.preview(&file) {
            Some(url) => post_preview(app, watch, &name, url),
            None => todo.push(file),
        }
    }
    if todo.is_empty() {
        return;
    }
    let batch = Id::generate();
    let task = app.run_with_progress(
        move |progress, cancel| {
            let _thread = sys::ImageThread::new();
            for file in todo {
                if cancel.is_cancelled() {
                    break;
                }
                let preview = sys::preview(&file.path, PREVIEW_W, PREVIEW_H);
                progress.send((file, preview));
            }
        },
        move |app, (file, preview)| on_preview(app, batch, watch, file, preview),
        move |app, ()| {
            let current = app.scans.preview_batch.as_ref();
            if current.is_some_and(|(b, _)| *b == batch) {
                app.scans.preview_batch = None;
            }
        },
    );
    app.scans.preview_batch = Some((batch, task));
}

// ----------------------------------------------------------------------------
/// A preview was made, or the file couldn't be decoded: keep it and send
/// it, unless a newer request took over.
fn on_preview(
    app: &mut App,
    batch: Id,
    watch: Id,
    file: FolderFile,
    preview: av_utils::error::Result<Vec<u8>>,
) {
    let current = app.scans.preview_batch.as_ref();
    if current.is_none_or(|(b, _)| *b != batch) {
        return;
    }
    let url = match preview {
        Ok(jpeg) => format!("data:image/jpeg;base64,{}", to_base64(&jpeg)),
        Err(e) => {
            log::info!("No preview of {}: {e}", file.path.display());
            String::new()
        }
    };
    post_preview(app, watch, &file.name, &url);
    let stamp = Stamp::of(&file);
    app.scans.previews.insert(file.path, (stamp, url));
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
        let message = format!("{name} isn't in the watch's folder.");
        return refuse(req, Error::InvalidPath { path }, message);
    }
    app.on_ui(
        move |_| sys::open_path(&path),
        move |_, opened| match opened {
            Ok(()) => req.ok(()),
            Err(e) => {
                log::warn!("Could not open {name}: {e}");
                refuse(req, e, format!("Could not open {name}."));
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
mod tests {
    use super::*;
    use crate::testing::Harness;
    use serde_json::Value;
    use std::fs;

    const WATCH_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b40";
    const IMPORTED_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b41";

    // ------------------------------------------------------------------------
    /// A harness whose vault has an images watch on the folder `scans`
    /// beside it, which holds `a.jpg`, `b.jpg` and `c.jpg`; `c.jpg` is in the
    /// vault already. And that folder.
    fn with_scans(name: &str) -> (Harness, PathBuf) {
        let mut h = Harness::new(name);
        let root = h.vault();
        let folder = root.with_file_name("scans");
        fs::create_dir_all(&folder).unwrap();
        for file in ["a.jpg", "b.jpg", "c.jpg"] {
            fs::write(folder.join(file), file).unwrap();
        }
        let watch = format!(
            "id: {WATCH_ID}\nfolder-watch:\n  type: images\n  folder: '{}'\n  patterns: ['*.jpg']\n",
            folder.display()
        );
        let imported = format!(
            "---\nid: {IMPORTED_ID}\ntitle: Old\ndocument: {{}}\nscan:\n  watch: {WATCH_ID}\n  origin: c.jpg\n---\nOld\n"
        );
        let files = [
            (".avelor/watches/watch_images.yaml", watch),
            ("Scans/old.md", imported),
        ];
        for (path, text) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        crate::api::switch_to(&mut h.app, root.clone());
        h.wait_scanned();
        h.take();
        (h, folder)
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_listing_tells_what_is_in_the_vault() {
        let (mut h, _folder) = with_scans("scan-list");
        let sent = h.call_waiting("scan_list", json!({ "watch": WATCH_ID }));
        let listing = &sent.last().unwrap()["ok"];
        let files = listing["files"].as_array().unwrap();
        let names: Vec<&str> = files.iter().map(|f| f["file"].as_str().unwrap()).collect();
        assert_eq!(names, ["a.jpg", "b.jpg", "c.jpg"]);
        assert_eq!(files[0]["in_vault"], Value::Null);
        assert_eq!(files[2]["in_vault"], IMPORTED_ID);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn requests_for_unknown_watches_and_files_are_refused() {
        let (mut h, folder) = with_scans("scan-refused");
        let unknown = Id::generate();
        let requests = [
            ("scan_list", json!({ "watch": unknown })),
            ("scan_previews", json!({ "watch": unknown, "files": [] })),
            ("scan_open", json!({ "watch": unknown, "file": "a.jpg" })),
        ];
        for (cmd, args) in requests {
            let sent = h.call(cmd, args);
            assert_eq!(sent[0]["err"]["code"], "not_found", "{cmd}");
        }

        fs::write(folder.with_file_name("outside.jpg"), "x").unwrap();
        let outside = Path::new("..").join("outside.jpg");
        let sent = h.call("scan_open", json!({ "watch": WATCH_ID, "file": outside }));
        assert_eq!(sent[0]["err"]["code"], "invalid_path");
        let sent = h.call(
            "scan_open",
            json!({ "watch": WATCH_ID, "file": "gone.jpg" }),
        );
        assert_eq!(sent[0]["err"]["code"], "invalid_path");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn only_files_in_the_folder_open_and_stamps_tell_changes() {
        let dir = std::env::temp_dir().join(format!("avelor-scan-test-{}", Id::generate()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("a.jpg"), "a").unwrap();
        std::fs::write(dir.with_extension("outside"), "x").unwrap();
        let file = FolderFile::read(&dir, "sub/a.jpg").unwrap();
        let stamp = Stamp::of(&file);
        let in_sub = inside(&dir, &dir.join("sub").join("a.jpg"));
        let up = inside(
            &dir.join("sub"),
            &dir.join("sub").join("..").join("sub").join("a.jpg"),
        );
        let escaped = inside(
            &dir.join("sub"),
            &dir.join("sub").join("..").join("..").join("x"),
        );
        let folder = inside(&dir, &dir.join("sub"));
        std::fs::write(dir.join("sub").join("a.jpg"), "changed").unwrap();
        let changed = FolderFile::read(&dir, "sub/a.jpg").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_file(dir.with_extension("outside")).unwrap();

        assert!(in_sub && up, "inside, also by way of ..");
        assert!(!escaped && !folder, "nothing outside, no folders");
        assert!(stamp.matches(&file) && !stamp.matches(&changed));
    }
}
