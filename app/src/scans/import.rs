//! Scans imported: their pages gathered into a document, written or replacing
//! one.

use std::path::Path;

use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::{Document as DocumentRole, Roles, Scan, Source};
use av_vault::schema::layout;
use av_vault::shared::OneOrMany;
use serde::Deserialize;
use serde::Serialize;

use crate::analysis::{self, Analysis, Batch};
use crate::app::App;
use crate::protocol::Req;
use crate::texts::Text;
use crate::watches::FolderFile;

// ----------------------------------------------------------------------------
/// `scan_import`'s args: the documents, each its pages in order.
#[derive(Deserialize)]
pub(crate) struct ImportRequest {
    pub(super) watch: Id,
    pub(super) docs: Vec<NewDocument>,
}

#[derive(Deserialize)]
pub(crate) struct NewDocument {
    /// The title the user gave; empty: the first page's proposed title.
    #[serde(default)]
    pub(super) title: String,
    pub(super) files: Vec<String>,
}

// ----------------------------------------------------------------------------
/// `scan_import`'s answer.
#[derive(Default, Serialize)]
struct ImportReply<'a> {
    pub(super) imported: Vec<Imported<'a>>,
    pub(super) failed: Vec<NotImported<'a>>,
}

#[derive(Serialize)]
pub(super) struct Imported<'a> {
    pub(super) files: &'a [String],
    pub(super) id: Id,
}

#[derive(Serialize)]
pub(super) struct NotImported<'a> {
    pub(super) files: &'a [String],
    pub(super) error: Text,
}

// ----------------------------------------------------------------------------
/// `scan_import {watch, docs}`: import each document into `Scans/` from its
/// pages' text (`scan_document`), one of one image with the image beside
/// it if the watch keeps originals. Answers `{imported, failed}` after an
/// `entity` event per document written; then, with the watch's analysis
/// on, the documents written are analysed as a job, a title the user gave
/// kept (`analysis::analyse`).
pub fn scan_import(app: &mut App, req: Req, request: ImportRequest) {
    let (watch, root, name, keep, analysis) = match app.watches.get(request.watch) {
        Ok(watch) => {
            let settings = watch.settings();
            let analysis = settings.analysis.clone().filter(Analysis::is_on);
            let keep = settings.keep_original;
            (
                watch.id(),
                settings.root(),
                settings.display_name(),
                keep,
                analysis,
            )
        }
        Err(e) => return req.err(e),
    };
    let mut reply = ImportReply::default();
    let mut titled = Vec::new();
    for doc in &request.docs {
        match write_document(app, watch, &root, doc, keep) {
            Ok(id) => {
                if !doc.title.trim().is_empty() {
                    titled.push(id);
                }
                reply.imported.push(Imported {
                    files: &doc.files,
                    id,
                });
            }
            Err(error) => {
                log::info!("Scan import: {:?} not imported: {error}", doc.files);
                reply.failed.push(NotImported {
                    files: &doc.files,
                    error,
                });
            }
        }
    }
    let (imported, not) = (reply.imported.len(), reply.failed.len());
    log::info!("Scan import of {watch}: {imported} imported, {not} failed");
    let documents: Vec<Id> = reply.imported.iter().map(|doc| doc.id).collect();
    req.ok(reply);
    if let Some(analysis) = analysis {
        analysis::analyse(app, Batch::scans(&name, documents, titled), &analysis);
    }
}

// ----------------------------------------------------------------------------
/// Write the document of `doc`'s pages, with its image beside it if `keep`
/// and it has one page (`Index::create_with_original`); or say why not.
fn write_document(
    app: &mut App,
    watch: Id,
    root: &Path,
    doc: &NewDocument,
    keep: bool,
) -> Result<Id, Text> {
    let (document, original) = {
        let pages = gather_pages(app, watch, root, doc)?;
        let original = match &pages[..] {
            [page] if keep => Some(original_of(root, page.file)?),
            _ => None,
        };
        (scan_document(watch, &doc.title, &pages), original)
    };
    let folder = Path::new(layout::SCANS);
    app.edit(|index, changes| match &original {
        Some((name, bytes)) => index.create_with_original(document, folder, name, bytes, changes),
        None => index.create(document, folder, changes),
    })
    .map_err(|e| {
        log::warn!("Scan import: could not write the document: {e}");
        Text::new("scans.write_failed")
    })
}

// ----------------------------------------------------------------------------
/// The image `file` of the watch's folder `root` to keep beside its
/// document: its name, without the subfolders, and its bytes.
fn original_of(root: &Path, file: &str) -> Result<(String, Vec<u8>), Text> {
    let path = root.join(file);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let bytes = std::fs::read(&path).map_err(|e| {
        log::warn!("Scan import: could not read {file} to keep it: {e}");
        Text::new("scans.keep_unread").arg("file", file)
    })?;
    Ok((name.unwrap_or_else(|| file.to_string()), bytes))
}

// ----------------------------------------------------------------------------
/// A page of a document to import: its file, read.
pub(super) struct ScanPage<'a> {
    /// The file, relative to the watch's folder.
    pub(super) file: &'a str,
    /// When the file was made (`FolderFile::taken`).
    pub(super) taken: DateTime,
    pub(super) text: &'a str,
    /// The title proposed from its text.
    pub(super) title: &'a str,
}

// ----------------------------------------------------------------------------
/// The pages of `doc`: each not in the vault yet, still in the folder, and
/// read as it is now; or why the document can't be imported.
fn gather_pages<'a>(
    app: &'a App,
    watch: Id,
    root: &Path,
    doc: &'a NewDocument,
) -> Result<Vec<ScanPage<'a>>, Text> {
    if doc.files.is_empty() {
        return Err(Text::new("scans.no_page"));
    }
    let imported = app.vault.index().scan_origins(&watch);
    let page = |name: &'a String| {
        if imported.contains_key(name) {
            return Err(Text::new("scans.in_vault").arg("name", name.as_str()));
        }
        let file = FolderFile::read(root, name)
            .map_err(|_| Text::new("scans.gone").arg("name", name.as_str()))?;
        let Some(ocr) = app.scans.cached(&file) else {
            return Err(Text::new("scans.not_read").arg("name", name.as_str()));
        };
        Ok(ScanPage {
            file: name,
            taken: file.taken,
            text: &ocr.text,
            title: &ocr.title,
        })
    };
    doc.files.iter().map(page).collect()
}

// ----------------------------------------------------------------------------
/// The document of `pages`, in order, imported from the watch `watch`:
/// titled `title`, else as its first page proposes, else after its first
/// file; dated by its first page; its text the pages' text, a blank line
/// between them; its source `scan: {watch, origin: [files]}`.
pub(super) fn scan_document(watch: Id, title: &str, pages: &[ScanPage]) -> Document {
    let first = pages.first();
    let stem = first
        .and_then(|page| Path::new(page.file).file_stem())
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let title = [title.trim(), first.map_or("", |page| page.title)]
        .into_iter()
        .find(|title| !title.is_empty())
        .map_or(stem, str::to_string);

    let roles = Roles {
        document: Some(DocumentRole::default()),
        ..Default::default()
    };
    let files = pages.iter().map(|page| page.file.to_string()).collect();
    let source = Source::Scan(Scan {
        watch: Some(watch),
        origin: OneOrMany::from_vec(files),
        ..Scan::default()
    });
    let mut document = Document::create(roles, source, title, Vec::new(), FileFormat::Markdown);
    if let Some(first) = first {
        document.set_date(first.taken);
    }
    let texts: Vec<&str> = pages.iter().map(|page| page.text).collect();
    document.set_content(format!("{}\n", texts.join("\n\n")));
    document
}
