//! Import scans (import.md#import-scans): the files of an images folder watch.
//!
//! The page lists the files; the commands of the typed protocol
//! (api.md#scans).

use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::id::Id;
use serde::Serialize;

use crate::app::App;
use crate::protocol::Req;
use crate::watches::{self, FolderFile};

// ----------------------------------------------------------------------------
/// One file of `scan_list`'s answer.
#[derive(Serialize)]
struct FileInfo<'a> {
    file: &'a str,
    size: u64,
    taken: DateTime,
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
            in_vault: in_vault.get(&file.name).copied(),
        })
        .collect();
    Listing {
        listed: DateTime::now(),
        files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Harness;
    use serde_json::{Value, json};
    use std::fs;
    use std::path::PathBuf;

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
    fn requests_for_unknown_watches_are_refused() {
        let (mut h, _folder) = with_scans("scan-refused");
        let unknown = Id::generate();
        let requests = [("scan_list", json!({ "watch": unknown }))];
        for (cmd, args) in requests {
            let sent = h.call(cmd, args);
            assert_eq!(sent[0]["err"]["code"], "not_found", "{cmd}");
        }
    }
}
