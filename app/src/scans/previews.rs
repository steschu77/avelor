//! The pages' previews, scaled for the import page.

use av_utils::base64::to_base64;
use av_utils::id::Id;
use serde::Deserialize;
use serde_json::json;

use super::Stamp;
use crate::app::App;
use crate::protocol::Req;
use crate::sys;
use crate::watches::FolderFile;

/// The previews' size: twice the page's large preview (180 x 254), so they
/// stay sharp on a high-DPI screen; thumbnails show them smaller.
const PREVIEW_W: u32 = 360;
const PREVIEW_H: u32 = 508;

// ----------------------------------------------------------------------------
/// `scan_previews`' args: the files, in the order the page wants them.
#[derive(Deserialize)]
pub(crate) struct PreviewRequest {
    pub(super) watch: Id,
    pub(super) files: Vec<String>,
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
