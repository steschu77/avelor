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
//! wrote (`enrich::analyse`), and with Keep original a document of one file
//! keeps the file beside it (`document.original`); an encrypted PDF always
//! does, as it has no text.
//!
//! The page shows the files as previews: `scan_previews` renders them
//! (`sys::preview`: WIC on Windows, gdk-pixbuf on Linux) on a worker and keeps
//! them while the app runs; `scan_open` opens a file in its default app.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use av_utils::base64::to_base64;
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::{Document as DocumentRole, Original, Roles, Scan, Source};
use av_vault::schema::layout;
use av_vault::shared::OneOrMany;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::ai;
use crate::app::App;
use crate::app::Task;
use crate::config::is_false;
use crate::enrich::{self, Analysis, Batch};
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use crate::protocol::Req;
use crate::reader::{self, Engine, Kind, ReadError};
use crate::sys;
use crate::watches::{self, FolderFile, READ_STEP, WatchKind, WatchSettings};
use av_vault::markdown::plain_title;

/// The longest proposed title, in characters.
const TITLE_LEN: usize = 80;
/// The previews' size: twice the page's large preview (180 x 254), so they
/// stay sharp on a high-DPI screen; thumbnails show them smaller.
const PREVIEW_W: u32 = 360;
const PREVIEW_H: u32 = 508;

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
        if read.current.as_deref() == Some(name) {
            Some("reading")
        } else if read.queue.iter().any(|queued| queued == name) {
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
/// A read in progress: files of one watch, read one after the other.
struct ScanRead {
    job: Id,
    watch: Id,
    /// The watch's folder, which the files' names are relative to.
    root: PathBuf,
    kind: WatchKind,
    /// The account of the model, if one reads.
    account: Option<Id>,
    engine: Engine,
    queue: VecDeque<String>,
    /// The file being read; stopping the job cancels its read.
    current: Option<String>,
    total: usize,
    counts: ReadCounts,
}

// ----------------------------------------------------------------------------
#[derive(Default)]
struct ReadCounts {
    read: usize,
    failed: usize,
    stopped: usize,
    /// Stopped at the account's spend limit.
    limit: bool,
}

impl ReadCounts {
    // ------------------------------------------------------------------------
    /// "5 read · 1 failed": the job's summary note; "· spend limit reached"
    /// if that stopped it.
    fn summary(&self) -> String {
        let counts = [
            (self.read, "read", "read"),
            (self.failed, "failed", "failed"),
            (self.stopped, "stopped", "stopped"),
        ];
        let summary = count_summary(&counts, "nothing to read");
        if self.limit {
            format!("{summary} \u{b7} spend limit reached")
        } else {
            summary
        }
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
/// Where a file stands in a read (the `scan_page` event).
#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
enum PageStatus<'a> {
    Queued,
    /// `words`: read so far.
    Reading {
        words: usize,
    },
    Read {
        words: usize,
        title: &'a str,
        #[serde(skip_serializing_if = "is_false")]
        encrypted: bool,
    },
    Failed {
        error: &'a str,
    },
    Stopped,
}

// ----------------------------------------------------------------------------
/// The `scan_page` event: `{watch, file, status, …}`.
#[derive(Serialize)]
struct PageEvent<'a> {
    watch: Id,
    file: &'a str,
    #[serde(flatten)]
    status: &'a PageStatus<'a>,
}

// ----------------------------------------------------------------------------
/// `scan_read`'s args.
#[derive(Deserialize)]
pub(crate) struct ReadRequest {
    watch: Id,
    files: Vec<String>,
    /// Read files that were read before too.
    #[serde(default)]
    again: bool,
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
/// `scan_import`'s args: the documents, each its pages in order.
#[derive(Deserialize)]
pub(crate) struct ImportRequest {
    watch: Id,
    docs: Vec<NewDocument>,
}

#[derive(Deserialize)]
struct NewDocument {
    /// The title the user gave; empty: the first page's proposed title.
    #[serde(default)]
    title: String,
    files: Vec<String>,
    /// The document in the vault whose pages these are, read again: its
    /// text is replaced.
    #[serde(default)]
    replace: Option<Id>,
}

// ----------------------------------------------------------------------------
/// `scan_import`'s answer.
#[derive(Default, Serialize)]
struct ImportReply<'a> {
    imported: Vec<Imported<'a>>,
    failed: Vec<NotImported<'a>>,
}

#[derive(Serialize)]
struct Imported<'a> {
    files: &'a [String],
    id: Id,
}

#[derive(Serialize)]
struct NotImported<'a> {
    files: &'a [String],
    error: String,
}

// ----------------------------------------------------------------------------
/// Refuse a request that does nothing, with `message` for the page.
fn refuse(req: Req, error: Error, message: impl Into<String>) {
    let message = message.into();
    log::info!("Scan request refused: {message}");
    req.fail(error, message);
}

// ----------------------------------------------------------------------------
/// Whether the analysis of the watch of `settings` can run, if it is on;
/// else the refused pick: a watch neither reads nor imports while its
/// analysis waits for a model that works, as Fetch mail doesn't fetch.
fn analysis_works(app: &App, settings: &WatchSettings) -> Result<(), (Error, String)> {
    let Some(analysis) = settings.analysis.as_ref().filter(|a| a.is_on()) else {
        return Ok(());
    };
    let what = enrich::scan_analysis(settings.kind);
    match app
        .ai
        .choose(analysis.model.as_ref(), what, DateTime::now())
    {
        Ok(_) => Ok(()),
        Err(refusal) => Err((refusal.error, refusal.message)),
    }
}

// ----------------------------------------------------------------------------
/// Where `file` stands in a read of `watch`: the `scan_page` event.
fn post_page(app: &App, watch: Id, file: &str, status: &PageStatus) {
    app.post_event(
        "scan_page",
        PageEvent {
            watch,
            file,
            status,
        },
    );
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
/// `scan_read {watch, files, again?}`: read the files with the model the
/// watch picked for OCR (`AiAccounts::choose`), as a job: a `scan_page` event per
/// file as it is queued, read and done. Files read before (and unchanged)
/// are answered at once, unless `again`. One read at a time: a request for
/// the watch being read adds to it. Answers `{job}`, the read of this watch
/// (null if there is none: every file was read before). Before each file
/// the account's spend limit is checked; reached, the read stops.
pub fn scan_read(app: &mut App, req: Req, request: ReadRequest) {
    let (root, watch_name, picked, kind) = match app.watches.get(request.watch) {
        Ok(watch) => {
            let settings = watch.settings();
            if let Err((error, message)) = analysis_works(app, settings) {
                return refuse(req, error, message);
            }
            let picked = settings.model(READ_STEP).cloned();
            (
                settings.root(),
                settings.display_name(),
                picked,
                settings.kind,
            )
        }
        Err(e) => return req.err(e),
    };
    let watch = request.watch;
    if app.scans.read.as_ref().is_some_and(|r| r.watch != watch) {
        let what = "reading a second folder".to_string();
        let message = "Another folder is still being read; stop it or wait for it.";
        return refuse(req, Error::Unsupported { what }, message);
    }
    let what = match kind {
        WatchKind::Images => "OCR",
        WatchKind::Pdf => "the scanned pages",
    };
    // A PDF watch reads text layers without a model.
    let (engine, account) = match app.ai.choose(picked.as_ref(), what, DateTime::now()) {
        Ok(resolved) => {
            let engine = Engine::Ai {
                client: resolved.client,
                provider: resolved.provider,
                address: resolved.address,
                model: resolved.model,
            };
            (engine, Some(resolved.account))
        }
        Err(refusal) if kind == WatchKind::Pdf => {
            let why = refusal.message;
            (Engine::None { why }, None)
        }
        Err(refusal) => return refuse(req, refusal.error, refusal.message),
    };

    let mut queue = Vec::new();
    for name in request.files {
        if app.scans.status(watch, &name).is_some() || queue.contains(&name) {
            continue;
        }
        if let Err(error) = readable(kind, &name) {
            post_page(app, watch, &name, &PageStatus::Failed { error });
            continue;
        }
        let file = FolderFile::read(&root, &name).ok();
        let cached = file.as_ref().and_then(|f| app.scans.cached(f));
        match cached {
            Some(ocr) if !request.again => {
                let (words, title, encrypted) = (ocr.words, ocr.title.as_str(), ocr.encrypted);
                let status = PageStatus::Read {
                    words,
                    title,
                    encrypted,
                };
                post_page(app, watch, &name, &status);
            }
            _ => {
                post_page(app, watch, &name, &PageStatus::Queued);
                queue.push(name);
            }
        }
    }
    let total = queue.len();
    if total == 0 {
        // Nothing new to read.
    } else if let Some(read) = &mut app.scans.read {
        read.total += total;
        read.queue.extend(queue);
        post_progress(app);
    } else {
        let label = match kind {
            WatchKind::Images => format!("Reading scans · {watch_name}"),
            WatchKind::Pdf => format!("Reading PDFs · {watch_name}"),
        };
        let job = app.start_job(JobKind::ScanRead, label, total);
        log::info!("Scan read {job}: reading {total} files of {watch_name}");
        app.scans.read = Some(ScanRead {
            job,
            watch,
            root,
            kind,
            account,
            engine,
            queue: queue.into(),
            current: None,
            total,
            counts: ReadCounts::default(),
        });
        pump(app);
    }
    req.ok(json!({ "job": app.scans.job_of(watch) }));
}

// ----------------------------------------------------------------------------
/// Start reading the next file, or finish the read if none is left.
fn pump(app: &mut App) {
    let Some(read) = &mut app.scans.read else {
        return;
    };
    if read.current.is_some() {
        return;
    }
    if read.queue.is_empty() {
        return finish(app);
    }
    // The pages read so far are counted (`account_used`): at its limit the
    // account takes no more.
    if let Some(account) = read.account {
        if let Err(refusal) = app.ai.check_limit(account, DateTime::now()) {
            return stop_at_limit(app, &refusal.message);
        }
    }
    let Some(read) = &mut app.scans.read else {
        return;
    };
    let Some(name) = read.queue.pop_front() else {
        return finish(app);
    };
    let (job, watch, root) = (read.job, read.watch, read.root.clone());
    let engine = read.engine.clone();
    let kind = readable(read.kind, &name).unwrap_or(Kind::Image);
    post_page(app, watch, &name, &PageStatus::Reading { words: 0 });

    let (file, progress_file, done_file) = (name.clone(), name.clone(), name.clone());
    app.run_job_with_progress(
        job,
        move |progress, cancel| {
            read_file(&engine, cancel, &root, &file, kind, |words| {
                progress.send_latest(words)
            })
        },
        move |app, words: usize| on_progress(app, job, &progress_file, words),
        move |app, result| on_read(app, job, done_file, result),
    );
    // The continuations run on this (the UI) thread, so they can't look
    // before this.
    if let Some(read) = &mut app.scans.read {
        read.current = Some(name);
    }
}

// ----------------------------------------------------------------------------
/// The kind of file the watch of `kind` reads `name` as; or why it doesn't
/// read it.
fn readable(kind: WatchKind, name: &str) -> Result<Kind, &'static str> {
    match (kind, Kind::of(name)) {
        (WatchKind::Images, Some(Kind::Image)) => Ok(Kind::Image),
        (WatchKind::Images, _) => Err("Only JPEG, PNG, GIF and WebP images can be read."),
        (WatchKind::Pdf, Some(Kind::Pdf)) => Ok(Kind::Pdf),
        (WatchKind::Pdf, _) => Err("Only PDF files can be read."),
    }
}

// ----------------------------------------------------------------------------
/// Read the file `name` in `root`, of the kind `kind`, with `engine`
/// (`reader::read`): the file as it was read, and its text. `on_words` gets
/// the words read so far.
fn read_file(
    engine: &Engine,
    cancel: &Cancel,
    root: &Path,
    name: &str,
    kind: Kind,
    on_words: impl FnMut(usize),
) -> Result<(FolderFile, Result<String, ReadError>), Error> {
    // The file's facts first: a change while it is read makes the text stale.
    let file = FolderFile::read(root, name)?;
    let data = std::fs::read(&file.path)?;
    let text = reader::read(engine, cancel, name, kind, data, on_words);
    Ok((file, text))
}

// ----------------------------------------------------------------------------
/// The words read so far of the file being read.
fn on_progress(app: &mut App, job: Id, name: &str, words: usize) {
    let Some(read) = &app.scans.read else {
        return;
    };
    if read.job == job && read.current.as_deref() == Some(name) {
        post_page(app, read.watch, name, &PageStatus::Reading { words });
    }
}

// ----------------------------------------------------------------------------
/// A file was read: keep its text and report it, or the failure; then read
/// the next. Ignored if the read is gone (stopped).
fn on_read(
    app: &mut App,
    job: Id,
    name: String,
    result: Result<(FolderFile, Result<String, ReadError>), Error>,
) {
    let Some(read) = app.scans.read.as_mut().filter(|r| r.job == job) else {
        return;
    };
    read.current = None;
    let page = match result {
        Ok((file, Ok(text))) => Ok(page_of(file, text, false)),
        Ok((file, Err(ReadError::Encrypted))) => Ok(page_of(file, String::new(), true)),
        Ok((_, Err(e))) => Err(e.words(&read.engine)),
        Err(e) => {
            log::warn!("Scan read {job}: could not read {name}: {e}");
            Err("Could not read the file.".to_string())
        }
    };
    let (watch, account) = (read.watch, read.account);
    match page {
        Ok((path, ocr)) => {
            read.counts.read += 1;
            let status = PageStatus::Read {
                words: ocr.words,
                title: &ocr.title,
                encrypted: ocr.encrypted,
            };
            post_page(app, watch, &name, &status);
            app.scans.cache.insert(path, ocr);
        }
        Err(error) => {
            read.counts.failed += 1;
            log::info!("Scan read {job}: {name} failed: {error}");
            post_page(app, watch, &name, &PageStatus::Failed { error: &error });
        }
    }
    // Saved per page, so a long read keeps what it spent.
    if let Some(account) = account {
        ai::account_used(app, account);
    }
    post_progress(app);
    pump(app);
}

// ----------------------------------------------------------------------------
/// The page `file` read to, `text` (none for an `encrypted` PDF), by the
/// file's path.
fn page_of(file: FolderFile, text: String, encrypted: bool) -> (PathBuf, Ocr) {
    let ocr = Ocr {
        stamp: Stamp::of(&file),
        words: reader::word_count(&text),
        title: title_of(&text),
        text,
        encrypted,
    };
    (file.path, ocr)
}

// ----------------------------------------------------------------------------
fn post_progress(app: &mut App) {
    let Some(read) = &app.scans.read else {
        return;
    };
    let done = read.counts.read + read.counts.failed;
    let (job, total) = (read.job, read.total);
    app.update_job(job, done, total);
}

// ----------------------------------------------------------------------------
/// Stop the read (`jobs::stop`, from `cancel_job` or a vault switch, has
/// cancelled the file being read): it and the files waiting report
/// `stopped`, and the job ends.
pub fn stop_read(app: &mut App) {
    let Some(read) = &mut app.scans.read else {
        return;
    };
    let mut stopped: Vec<String> = read.queue.drain(..).collect();
    if let Some(name) = read.current.take() {
        stopped.insert(0, name);
    }
    read.counts.stopped += stopped.len();
    let watch = read.watch;
    log::info!("Scan read {}: stopped", read.job);
    for name in &stopped {
        post_page(app, watch, name, &PageStatus::Stopped);
    }
    finish(app);
}

// ----------------------------------------------------------------------------
/// The read's account has reached its spend limit (`message` says so):
/// say it once, and stop the read as Stop does, the files left `stopped`.
fn stop_at_limit(app: &mut App, message: &str) {
    let Some(read) = &mut app.scans.read else {
        return;
    };
    read.counts.limit = true;
    log::info!("Scan read {}: {message}", read.job);
    app.notice(message);
    stop_read(app);
}

// ----------------------------------------------------------------------------
/// The read is done: report it, and the account's spending (what the
/// file being read when it stopped cost, too).
fn finish(app: &mut App) {
    let Some(read) = app.scans.read.take() else {
        return;
    };
    let note = read.counts.summary();
    log::info!("Scan read {}: {note}", read.job);
    app.end_job(read.job, &note);
    if let Some(account) = read.account {
        ai::account_used(app, account);
    }
}

// ----------------------------------------------------------------------------
/// `scan_import {watch, docs}`: import each document into `Scans/` from its
/// pages' text (`scan_document`), one of one image with the image beside
/// it if the watch keeps originals. Answers `{imported, failed}` after an
/// `entity` event per document written; then, with the watch's analysis
/// on, the documents written are analysed as a job, a title the user gave
/// kept (`enrich::analyse`).
pub fn scan_import(app: &mut App, req: Req, request: ImportRequest) {
    let (watch, root, name, kind, keep, analysis) = match app.watches.get(request.watch) {
        Ok(watch) => {
            let settings = watch.settings();
            if let Err((error, message)) = analysis_works(app, settings) {
                return refuse(req, error, message);
            }
            let analysis = settings.analysis.clone().filter(Analysis::is_on);
            let keep = settings.keep_original;
            (
                watch.id(),
                settings.root(),
                settings.display_name(),
                settings.kind,
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
        let batch = Batch::scans(kind, &name, documents, titled);
        enrich::analyse(app, batch, &analysis);
    }
}

// ----------------------------------------------------------------------------
/// Write the document of `doc`'s pages, with its file beside it if `keep`
/// and it has one page (`Index::create_with_original`); an encrypted PDF
/// always keeps it, marked so. Or replace the text of the document they
/// were imported into (`replace_document`). Or say why not.
fn write_document(
    app: &mut App,
    watch: Id,
    root: &Path,
    doc: &NewDocument,
    keep: bool,
) -> Result<Id, String> {
    if let Some(id) = doc.replace {
        return replace_document(app, watch, root, doc, id);
    }
    let (document, original) = {
        let pages = gather_pages(app, watch, root, doc)?;
        let original = match &pages[..] {
            [page] if keep || page.encrypted => Some(original_of(root, page.file)?),
            _ => None,
        };
        let mut document = scan_document(watch, &doc.title, &pages);
        if pages.iter().any(|page| page.encrypted) {
            document.set_original(Original {
                encrypted: true,
                ..Original::default()
            });
        }
        (document, original)
    };
    let folder = Path::new(layout::SCANS);
    app.edit(|index, changes| match &original {
        Some((name, bytes)) => index.create_with_original(document, folder, name, bytes, changes),
        None => index.create(document, folder, changes),
    })
    .map_err(|e| {
        log::warn!("Scan import: could not write the document: {e}");
        "Could not write the document.".to_string()
    })
}

// ----------------------------------------------------------------------------
/// Replace the text of the document `id` with `doc`'s pages read again: its
/// pages must be the document's own (`scan.origin`). Its frontmatter stays,
/// a title the user gave replaces its title, and its kept file's encrypted
/// mark follows the new read.
fn replace_document(
    app: &mut App,
    watch: Id,
    root: &Path,
    doc: &NewDocument,
    id: Id,
) -> Result<Id, String> {
    let index = app.vault.index();
    let Some(document) = index.get(&id) else {
        return Err("The document it replaces is gone.".to_string());
    };
    let mut origin: Vec<&str> = match document.source() {
        Some(Source::Scan(scan)) => scan.origin.as_slice().iter().map(String::as_str).collect(),
        _ => Vec::new(),
    };
    let mut files: Vec<&str> = doc.files.iter().map(String::as_str).collect();
    origin.sort_unstable();
    files.sort_unstable();
    if origin != files {
        let title = document.title();
        return Err(format!(
            "\u{201c}{title}\u{201d} has other pages; read them all again and import them together."
        ));
    }
    let (text, encrypted) = {
        let pages = gather_pages(app, watch, root, doc)?;
        (pages_text(&pages), pages.iter().any(|page| page.encrypted))
    };
    let title = doc.title.trim().to_string();
    let outcome = app.edit(|index, changes| {
        index.edit(&id, changes, |d| {
            d.set_content(text);
            if !title.is_empty() {
                d.set_title(title);
            }
            if let Some(mut original) = d.original().cloned() {
                original.encrypted = encrypted;
                d.set_original(original);
            }
        })
    });
    match outcome.and_then(|outcome| app.vault.require_written(outcome)) {
        Ok(()) => Ok(id),
        Err(e) => {
            log::warn!("Scan import: could not replace the text of {id}: {e}");
            Err("Could not write the document.".to_string())
        }
    }
}

// ----------------------------------------------------------------------------
/// The file `file` of the watch's folder `root` to keep beside its
/// document: its name, without the subfolders, and its bytes.
fn original_of(root: &Path, file: &str) -> Result<(String, Vec<u8>), String> {
    let path = root.join(file);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let bytes = std::fs::read(&path).map_err(|e| {
        log::warn!("Scan import: could not read {file} to keep it: {e}");
        format!("Could not read {file} to keep it.")
    })?;
    Ok((name.unwrap_or_else(|| file.to_string()), bytes))
}

// ----------------------------------------------------------------------------
/// A page of a document to import: its file, read.
struct ScanPage<'a> {
    /// The file, relative to the watch's folder.
    file: &'a str,
    /// When the file was made (`FolderFile::taken`).
    taken: DateTime,
    text: &'a str,
    /// The title proposed from its text.
    title: &'a str,
    /// An encrypted PDF, without text.
    encrypted: bool,
}

// ----------------------------------------------------------------------------
/// The pages of `doc`: each not in the vault yet (or in the document it
/// replaces), still in the folder, and read as it is now; or why the
/// document can't be imported.
fn gather_pages<'a>(
    app: &'a App,
    watch: Id,
    root: &Path,
    doc: &'a NewDocument,
) -> Result<Vec<ScanPage<'a>>, String> {
    if doc.files.is_empty() {
        return Err("A document needs a page.".to_string());
    }
    let imported = app.vault.index().scan_origins(&watch);
    let page = |name: &'a String| {
        if imported
            .get(name)
            .is_some_and(|id| doc.replace != Some(*id))
        {
            return Err(format!("{name} is in the vault already."));
        }
        let file = FolderFile::read(root, name)
            .map_err(|_| format!("{name} is no longer in the folder."))?;
        let Some(ocr) = app.scans.cached(&file) else {
            return Err(format!(
                "{name} isn't read, or changed since; read it again."
            ));
        };
        Ok(ScanPage {
            file: name,
            taken: file.taken,
            text: &ocr.text,
            title: &ocr.title,
            encrypted: ocr.encrypted,
        })
    };
    doc.files.iter().map(page).collect()
}

// ----------------------------------------------------------------------------
/// The document of `pages`, in order, imported from the watch `watch`:
/// titled `title`, else as its first page proposes, else after its first
/// file; dated by its first page; its text the pages' text, a blank line
/// between them; its source `scan: {watch, origin: [files]}`.
fn scan_document(watch: Id, title: &str, pages: &[ScanPage]) -> Document {
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
    });
    let mut document = Document::create(roles, source, title, Vec::new(), FileFormat::Markdown);
    if let Some(first) = first {
        document.set_date(first.taken);
    }
    document.set_content(pages_text(pages));
    document
}

// ----------------------------------------------------------------------------
/// The pages' text, in order, a blank line between them.
fn pages_text(pages: &[ScanPage]) -> String {
    let texts: Vec<&str> = pages.iter().map(|page| page.text).collect();
    format!("{}\n", texts.join("\n\n"))
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

// ----------------------------------------------------------------------------
/// A title proposed for a page: its first heading, else its first line,
/// as plain text (`plain_title`).
fn title_of(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let heading = lines.clone().find(|line| line.starts_with('#'));
    let line = heading.or_else(|| lines.next()).unwrap_or_default();
    // A table row's cells, as words.
    let line: Vec<&str> = line
        .split(['|', ' ', '\t'])
        .filter(|w| !w.is_empty())
        .collect();
    plain_title(&line.join(" "), TITLE_LEN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Harness;
    use crate::testing::{FakeOllama, TestPage, encrypted, parse, pdf_of, shape};
    use serde_json::Value;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    const WATCH_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b40";
    const IMPORTED_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b41";
    const OLLAMA_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b42";

    // ------------------------------------------------------------------------
    /// A harness whose vault has an images watch on the folder `scans`
    /// beside it, which holds `a.jpg`, `b.jpg` and `c.jpg`; `c.jpg` is in the
    /// vault already. And that folder.
    fn with_scans(name: &str) -> (Harness, PathBuf) {
        with_scans_and(name, &[])
    }

    // ------------------------------------------------------------------------
    /// `with_scans`, with the vault files `more` too (an AI account).
    fn with_scans_and(name: &str, more: &[(&str, String)]) -> (Harness, PathBuf) {
        let mut h = Harness::new(name);
        let root = h.vault();
        let folder = root.with_file_name("scans");
        fs::create_dir_all(&folder).unwrap();
        for file in ["a.jpg", "b.jpg", "c.jpg"] {
            fs::write(folder.join(file), file).unwrap();
        }
        let watch = format!(
            "id: {WATCH_ID}\nfolder-watch:\n  type: images\n  folder: '{}'\n  patterns: ['*.jpg']\n  models:\n    read:\n      account: {OLLAMA_ID}\n      model: m\n",
            folder.display()
        );
        let imported = format!(
            "---\nid: {IMPORTED_ID}\ntitle: Old\ndocument: {{}}\nscan:\n  watch: {WATCH_ID}\n  origin: c.jpg\n---\nOld\n"
        );
        let files = [
            (".avelor/watches/watch_images.yaml", watch),
            ("Scans/old.md", imported),
        ];
        for (path, text) in files.into_iter().chain(more.iter().cloned()) {
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
    /// A harness whose vault has a PDF watch on the folder `pdfs` beside it,
    /// reading with the Ollama account `account` if given (else with no
    /// pick), which holds a bill with a text layer, a lease of two scanned
    /// pages, an encrypted letter and a photo. And that folder.
    fn with_pdf_watch(name: &str, account: Option<String>) -> (Harness, PathBuf) {
        let mut h = Harness::new(name);
        let root = h.vault();
        let folder = root.with_file_name("pdfs");
        fs::create_dir_all(&folder).unwrap();
        let bill = ["Stadtwerke Rechnung Nr. 4711", "Betrag: 42,00 EUR"];
        let letter = ["Kontoauszug Nr. 10 mit allen Buchungen des Monats"];
        let files = [
            ("bill.pdf", pdf_of(&[TestPage::Text(&bill)])),
            (
                "lease.pdf",
                pdf_of(&[
                    TestPage::Scan(b"\xff\xd8 page one"),
                    TestPage::Scan(b"\xff\xd8 page two"),
                ]),
            ),
            ("letter.pdf", encrypted(pdf_of(&[TestPage::Text(&letter)]))),
            ("photo.jpg", b"photo".to_vec()),
        ];
        for (file, bytes) in files {
            fs::write(folder.join(file), bytes).unwrap();
        }
        let models = match account {
            Some(_) => {
                format!("\n  models:\n    read:\n      account: {OLLAMA_ID}\n      model: m")
            }
            None => String::new(),
        };
        let watch = format!(
            "id: {WATCH_ID}\nfolder-watch:\n  type: pdf\n  folder: '{}'\n  patterns: ['*.pdf', '*.jpg']{models}\n",
            folder.display()
        );
        let dir = root.join(".avelor/watches");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("watch_pdf.yaml"), watch).unwrap();
        if let Some(account) = account {
            let dir = root.join(".avelor/accounts");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("ollama.yaml"), account).unwrap();
        }
        crate::api::switch_to(&mut h.app, root);
        h.wait_scanned();
        h.take();
        (h, folder)
    }

    // ------------------------------------------------------------------------
    /// Read `files` of the watch to the end: what the page got.
    fn read_all(h: &mut Harness, files: &[&str]) -> Vec<Value> {
        let args = json!({ "watch": WATCH_ID, "files": files });
        let request = json!({ "id": 1, "cmd": "scan_read", "args": args });
        h.app.on_web_message(request.to_string());
        let ended = |t: &String| t.contains(r#""state":"done""#);
        parse(&h.wait_for(|sent| sent.iter().any(ended)))
    }

    // ------------------------------------------------------------------------
    /// The last `scan_page` of `file` in `sent`.
    fn last_page<'a>(sent: &'a [Value], file: &str) -> &'a Value {
        let mut pages = sent.iter().filter(|m| m["ev"] == "scan_page");
        &pages.rfind(|m| m["data"]["file"] == file).unwrap()["data"]
    }

    // ------------------------------------------------------------------------
    /// Keep `text` as what the file `name` of `folder` read to.
    fn read_page(h: &mut Harness, folder: &Path, name: &str, text: &str) {
        let file = FolderFile::read(folder, name).unwrap();
        let ocr = Ocr {
            stamp: Stamp::of(&file),
            words: reader::word_count(text),
            title: title_of(text),
            text: text.to_string(),
            encrypted: false,
        };
        h.app.scans.cache.insert(file.path, ocr);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn pages_become_one_document() {
        let watch = Id::generate();
        let first =
            DateTime::from_system_time(SystemTime::UNIX_EPOCH + Duration::from_secs(1 << 30));
        let later =
            DateTime::from_system_time(SystemTime::UNIX_EPOCH + Duration::from_secs(1 << 31));
        let sub = Path::new("2026")
            .join("scan_0806.jpg")
            .to_string_lossy()
            .into_owned();
        let page = |title| ScanPage {
            file: &sub,
            taken: first,
            text: "# Bill",
            title,
            encrypted: false,
        };
        let second = ScanPage {
            file: "scan_0807.jpg",
            taken: later,
            text: "Page two",
            title: "Page two",
            encrypted: false,
        };
        let pages = [page("Bill"), second];

        let doc = scan_document(watch, "  ", &pages);
        assert_eq!(doc.title(), "Bill", "the first page's title");
        assert_eq!(doc.date(), Some(first));
        assert_eq!(doc.content(), "# Bill\n\nPage two\n");
        assert!(doc.roles().document.is_some());
        let Some(Source::Scan(scan)) = doc.source() else {
            panic!("{:?}", doc.source());
        };
        assert_eq!(scan.watch, Some(watch));
        assert_eq!(
            scan.origin.as_slice(),
            [sub.clone(), "scan_0807.jpg".into()]
        );

        let doc = scan_document(watch, " Taxes ", &pages);
        assert_eq!(doc.title(), "Taxes", "the title given, trimmed");
        let doc = scan_document(watch, "", &[page("")]);
        assert_eq!(doc.title(), "scan_0806", "the first file's name");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_import_keeps_a_single_image_and_hands_its_documents_to_the_analysis() {
        let answer =
            r#"{"title": "Brief der Stadtwerke", "date": "2020-03-01", "genre": "letter"}"#;
        let address = FakeOllama::new(&["m"]).replying(&[answer]).start();
        let account = format!("id: {OLLAMA_ID}\nollama:\n  address: {address}\n  offered: [m]\n");
        let (mut h, folder) = with_scans_and(
            "scan-import-analysis",
            &[(".avelor/accounts/ollama.yaml", account)],
        );
        fs::write(folder.join("d.jpg"), "d.jpg").unwrap();
        for (name, text) in [
            ("a.jpg", "Sehr geehrter Herr"),
            ("b.jpg", "Seite 1"),
            ("d.jpg", "Seite 2"),
        ] {
            read_page(&mut h, &folder, name, text);
        }
        let analysis = json!({ "level": "lite", "model": { "account": OLLAMA_ID, "model": "m" } });
        let settings = json!({ "watch": WATCH_ID, "analysis": analysis, "keep_original": true });
        h.call("set_watch", settings);
        let docs = json!([
            { "title": "", "files": ["a.jpg"] },
            { "title": "Mein Titel", "files": ["b.jpg", "d.jpg"] },
        ]);
        let sent = h.call("scan_import", json!({ "watch": WATCH_ID, "docs": docs }));
        assert_eq!(shape(&sent), ["entity", "entity", "job", "reply"]);
        assert_eq!(sent[2]["data"]["label"], "Analysing scans \u{b7} scans");
        let path = |i: usize| h.vault().join(sent[i]["data"]["path"].as_str().unwrap());
        let (single, double) = (path(0), path(1));
        let done = |sent: &[String]| sent.iter().any(|m| m.contains(r#""state":"done""#));
        h.wait_for(done);

        let text = fs::read_to_string(&single).unwrap();
        assert!(text.contains("title: Brief der Stadtwerke"), "{text}");
        assert!(
            text.contains("date: 2020-03-01T12:00:00Z") && text.contains("genre: letter"),
            "{text}"
        );
        assert!(
            text.contains("name: a.jpg") && text.contains("origin: a.jpg"),
            "{text}"
        );
        let id = Id::parse(sent[0]["data"]["id"].as_str().unwrap()).unwrap();
        let original = h.app.vault.index().original_path(&id).unwrap();
        assert_eq!(
            fs::read(&original).unwrap(),
            b"a.jpg",
            "the image beside it"
        );
        assert!(folder.join("a.jpg").exists(), "the watch's file stays");

        let text = fs::read_to_string(&double).unwrap();
        assert!(
            text.contains("title: Mein Titel"),
            "the title given stays: {text}"
        );
        assert!(text.contains("genre: letter"), "{text}");
        assert!(!text.contains("original:"), "two images keep none: {text}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_pdf_watch_reads_text_layers_scans_by_page_and_imports_encrypted_pdfs() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let address = FakeOllama::new(&["m"])
            .answering_in_turn(&["Seite eins", "Seite zwei"])
            .recording(&requests)
            .start();
        let account = format!("id: {OLLAMA_ID}\nollama:\n  address: {address}\n  offered: [m]\n");
        let (mut h, folder) = with_pdf_watch("scan-pdf-watch", Some(account));
        let sent = read_all(
            &mut h,
            &["bill.pdf", "lease.pdf", "letter.pdf", "photo.jpg"],
        );

        let job = sent.iter().find(|m| m["ev"] == "job").unwrap();
        assert_eq!(job["data"]["label"], "Reading PDFs \u{b7} pdfs");
        let bill = last_page(&sent, "bill.pdf");
        assert_eq!(bill["status"], "read");
        assert_eq!(bill["title"], "Stadtwerke Rechnung Nr. 4711");
        assert_eq!(last_page(&sent, "lease.pdf")["words"], 4);
        let letter = last_page(&sent, "letter.pdf");
        assert_eq!(
            (&letter["status"], &letter["encrypted"]),
            (&json!("read"), &json!(true))
        );
        let photo = last_page(&sent, "photo.jpg");
        assert_eq!(photo["error"], "Only PDF files can be read.");
        assert_eq!(
            requests.lock().unwrap().len(),
            2,
            "a request per scanned page"
        );

        // Without Keep original, only the encrypted PDF keeps its file.
        let docs = json!([{ "files": ["bill.pdf"] }, { "files": ["letter.pdf"] }]);
        let sent = h.call("scan_import", json!({ "watch": WATCH_ID, "docs": docs }));
        assert_eq!(shape(&sent), ["entity", "entity", "reply"]);
        let path = |i: usize| h.vault().join(sent[i]["data"]["path"].as_str().unwrap());
        let text = fs::read_to_string(path(0)).unwrap();
        assert!(!text.contains("original:"), "{text}");
        assert!(text.contains("Stadtwerke Rechnung Nr. 4711"), "{text}");
        let text = fs::read_to_string(path(1)).unwrap();
        assert!(
            text.contains("encrypted: true") && text.contains("name: letter.pdf"),
            "{text}"
        );
        assert!(text.ends_with("---\n\n"), "no text: {text}");
        let id = Id::parse(sent[1]["data"]["id"].as_str().unwrap()).unwrap();
        let original = h.app.vault.index().original_path(&id).unwrap();
        assert_eq!(
            fs::read(original).unwrap(),
            fs::read(folder.join("letter.pdf")).unwrap()
        );

        // With it, a PDF keeps its file.
        h.call(
            "set_watch",
            json!({ "watch": WATCH_ID, "keep_original": true }),
        );
        let docs = json!([{ "files": ["lease.pdf"] }]);
        let sent = h.call("scan_import", json!({ "watch": WATCH_ID, "docs": docs }));
        let id = Id::parse(sent[0]["data"]["id"].as_str().unwrap()).unwrap();
        let original = h.app.vault.index().original_path(&id).unwrap();
        assert!(original.to_string_lossy().ends_with(".pdf"), "{original:?}");
        let text = fs::read_to_string(h.vault().join(sent[0]["data"]["path"].as_str().unwrap()));
        assert!(text.unwrap().ends_with("Seite eins\n\nSeite zwei\n"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn without_a_model_a_pdf_watch_reads_text_layers_only() {
        let (mut h, _) = with_pdf_watch("scan-pdf-no-model", None);
        let sent = read_all(&mut h, &["bill.pdf", "lease.pdf"]);
        let reply = sent.iter().find(|m| m["re"] == 1).unwrap();
        assert!(reply["ok"]["job"].is_string(), "{reply}");
        assert_eq!(last_page(&sent, "bill.pdf")["status"], "read");
        let lease = last_page(&sent, "lease.pdf");
        assert_eq!(
            lease["error"],
            "For the scanned pages, add an AI provider under Accounts."
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn pages_read_again_replace_their_documents_text_only_whole() {
        let (mut h, folder) = with_scans("scan-replace");
        read_page(&mut h, &folder, "c.jpg", "# New text");
        read_page(&mut h, &folder, "a.jpg", "# A");
        let id = Id::parse(IMPORTED_ID).unwrap();

        let docs = json!([
            { "files": ["c.jpg"] },
            { "files": ["a.jpg", "c.jpg"], "replace": id },
        ]);
        let sent = h.call("scan_import", json!({ "watch": WATCH_ID, "docs": docs }));
        let failed: Vec<&str> = sent[0]["ok"]["failed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["error"].as_str().unwrap())
            .collect();
        assert_eq!(
            failed,
            [
                "c.jpg is in the vault already.",
                "\u{201c}Old\u{201d} has other pages; read them all again and import them together.",
            ]
        );

        let docs = json!([{ "files": ["c.jpg"], "replace": id }]);
        let sent = h.call("scan_import", json!({ "watch": WATCH_ID, "docs": docs }));
        let reply = &sent.last().unwrap()["ok"];
        assert_eq!(reply["imported"], json!([{ "files": ["c.jpg"], "id": id }]));
        let text = fs::read_to_string(h.vault().join("Scans/old.md")).unwrap();
        assert!(text.contains("title: Old"), "the title stays: {text}");
        assert!(text.ends_with("---\n# New text\n"), "{text}");

        let docs = json!([{ "title": "Neu", "files": ["c.jpg"], "replace": id }]);
        h.call("scan_import", json!({ "watch": WATCH_ID, "docs": docs }));
        let text = fs::read_to_string(h.vault().join("Scans/old.md")).unwrap();
        assert!(
            text.contains("title: Neu"),
            "a title given replaces it: {text}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_watch_whose_analysis_has_no_working_pick_neither_reads_nor_imports() {
        let (mut h, folder) = with_scans("scan-analysis-blocked");
        read_page(&mut h, &folder, "a.jpg", "# A");
        let analysis = json!({ "level": "lite", "model": null });
        h.call(
            "set_watch",
            json!({ "watch": WATCH_ID, "analysis": analysis }),
        );
        let read = json!({ "watch": WATCH_ID, "files": ["b.jpg"] });
        let import = json!({ "watch": WATCH_ID, "docs": [{ "files": ["a.jpg"] }] });
        for (cmd, args) in [("scan_read", read), ("scan_import", import)] {
            let sent = h.call(cmd, args);
            let message = sent[0]["err"]["message"].as_str().unwrap_or_default();
            assert!(
                message.starts_with("For the scan analysis,"),
                "{cmd}: {sent:?}"
            );
        }
        assert!(h.app.scans.read.is_none());
        let index = h.app.vault.index();
        let origins = index.scan_origins(&Id::parse(WATCH_ID).unwrap());
        assert!(!origins.contains_key("a.jpg"), "nothing imported");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_watch_reads_the_files_of_its_type() {
        assert_eq!(readable(WatchKind::Images, "a.JPG"), Ok(Kind::Image));
        assert_eq!(readable(WatchKind::Pdf, "a.pdf"), Ok(Kind::Pdf));
        assert!(readable(WatchKind::Images, "a.pdf").is_err());
        assert_eq!(
            readable(WatchKind::Pdf, "a.jpg"),
            Err("Only PDF files can be read.")
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn import_writes_read_pages_and_says_why_not_the_others() {
        let (mut h, folder) = with_scans("scan-import");
        read_page(&mut h, &folder, "a.jpg", "# Bill 4711\n\nPay now.");
        read_page(&mut h, &folder, "c.jpg", "# Old");
        let docs = json!([
            { "title": "", "files": ["a.jpg"] },
            { "title": "B", "files": ["b.jpg"] },
            { "files": ["c.jpg"] },
            { "files": [] },
        ]);
        let sent = h.call("scan_import", json!({ "watch": WATCH_ID, "docs": docs }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let entity = &sent[0]["data"];
        assert_eq!(entity["title"], "Bill 4711");
        let reply = &sent[1]["ok"];
        assert_eq!(
            reply["imported"],
            json!([{ "files": ["a.jpg"], "id": entity["id"] }])
        );
        let failed: Vec<&str> = reply["failed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["error"].as_str().unwrap())
            .collect();
        assert_eq!(
            failed,
            [
                "b.jpg isn't read, or changed since; read it again.",
                "c.jpg is in the vault already.",
                "A document needs a page.",
            ]
        );
        let path = h.vault().join(entity["path"].as_str().unwrap());
        let text = fs::read_to_string(path).unwrap();
        assert!(
            text.contains(&format!("watch: {WATCH_ID}")) && text.contains("origin: a.jpg"),
            "{text}"
        );
        assert!(text.ends_with("# Bill 4711\n\nPay now.\n"), "{text}");

        let again = json!({ "watch": WATCH_ID, "docs": [{ "files": ["a.jpg"] }] });
        let sent = h.call("scan_import", again);
        assert_eq!(
            sent[0]["ok"]["failed"][0]["error"],
            "a.jpg is in the vault already."
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_listing_tells_what_is_read_and_what_is_in_the_vault() {
        let (mut h, folder) = with_scans("scan-list");
        read_page(&mut h, &folder, "a.jpg", "# A");
        let sent = h.call_waiting("scan_list", json!({ "watch": WATCH_ID }));
        let listing = &sent.last().unwrap()["ok"];
        let files = listing["files"].as_array().unwrap();
        let names: Vec<&str> = files.iter().map(|f| f["file"].as_str().unwrap()).collect();
        assert_eq!(names, ["a.jpg", "b.jpg", "c.jpg"]);
        assert_eq!(files[0]["read"]["title"], "A");
        assert_eq!(
            (&files[0]["in_vault"], &files[1]["read"]),
            (&Value::Null, &Value::Null)
        );
        assert_eq!(files[2]["in_vault"], IMPORTED_ID);
        assert_eq!(listing["job"], Value::Null, "nothing is being read");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_read_stops_before_its_next_page_at_the_spend_limit() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let address = FakeOllama::new(&["m"])
            .replying(&["# A"])
            .recording(&requests)
            .start();
        let account = format!("id: {OLLAMA_ID}\nollama:\n  address: {address}\n  offered: [m]\n");
        let (mut h, _) = with_scans_and("scan-limit", &[(".avelor/accounts/ollama.yaml", account)]);
        let args = json!({ "watch": WATCH_ID, "files": ["a.jpg", "b.jpg"] });
        let request = json!({ "id": 1, "cmd": "scan_read", "args": args });
        h.app.on_web_message(request.to_string());
        // The first page is on its way; then the limit is reached.
        let id = Id::parse(OLLAMA_ID).unwrap();
        h.app.ai.update(id, |s| s.spend_limit = Some(0.0)).unwrap();
        let ended = |t: &String| t.contains(r#""state":"done""#);
        let sent = parse(&h.wait_for(|sent| sent.iter().any(ended)));

        let last_of = |file: &str| {
            let mut pages = sent.iter().filter(|m| m["ev"] == "scan_page");
            pages.rfind(|m| m["data"]["file"] == file).unwrap()["data"]["status"].clone()
        };
        assert_eq!(last_of("a.jpg"), "read");
        assert_eq!(last_of("b.jpg"), "stopped");
        let requests = requests.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["options"]["num_ctx"], ai::CONTEXT_TOKENS);
        let notice = sent.iter().find(|m| m["ev"] == "notice").unwrap();
        let message = notice["data"]["message"].as_str().unwrap();
        assert!(
            message.starts_with("Ollama has reached its spend limit"),
            "{message}"
        );
        let job = sent.iter().rfind(|m| m["ev"] == "job").unwrap();
        assert_eq!(
            job["data"]["note"],
            "1 read \u{b7} 1 stopped \u{b7} spend limit reached"
        );
        assert!(h.app.scans.read.is_none());

        // The next read is refused before it starts.
        let sent = h.call(
            "scan_read",
            json!({ "watch": WATCH_ID, "files": ["b.jpg"] }),
        );
        assert_eq!(sent[0]["err"]["code"], "spend_limit");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn requests_for_unknown_watches_accounts_and_files_are_refused() {
        let (mut h, folder) = with_scans("scan-refused");
        let unknown = Id::generate();
        let requests = [
            ("scan_list", json!({ "watch": unknown })),
            ("scan_read", json!({ "watch": unknown, "files": [] })),
            ("scan_previews", json!({ "watch": unknown, "files": [] })),
            ("scan_open", json!({ "watch": unknown, "file": "a.jpg" })),
            ("scan_import", json!({ "watch": unknown, "docs": [] })),
        ];
        for (cmd, args) in requests {
            let sent = h.call(cmd, args);
            assert_eq!(sent[0]["err"]["code"], "not_found", "{cmd}");
        }

        // The account of the watch's OCR model is gone.
        let sent = h.call(
            "scan_read",
            json!({ "watch": WATCH_ID, "files": ["a.jpg"] }),
        );
        assert_eq!(shape(&sent), ["reply"], "nothing queued");
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        assert_eq!(
            sent[0]["err"]["message"],
            "For OCR, the picked model\u{2019}s account is gone; pick another model."
        );
        assert!(h.app.scans.read.is_none());

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
    fn titles_come_from_the_first_heading_or_line() {
        let letter = "Stadtwerke Springfield\nMain Street 1\n\n## Rechnung Nr. 4711\n\nText";
        assert_eq!(title_of(letter), "Rechnung Nr. 4711");
        assert_eq!(title_of("\n**Dear Mr. Yahnek,**\nText"), "Dear Mr. Yahnek,");
        assert_eq!(title_of("| Date | Amount |\n|---|---|"), "Date Amount");
        let long = format!("# {}", "word ".repeat(30));
        assert!(title_of(&long).chars().count() <= TITLE_LEN + 1);
        assert_eq!(title_of(""), "");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn statuses_and_summaries_read_well() {
        fn json(status: PageStatus) -> String {
            serde_json::to_string(&status).unwrap()
        }
        assert_eq!(json(PageStatus::Queued), r#"{"status":"queued"}"#);
        let watch = Id::generate();
        let status = PageStatus::Failed { error: "Gone" };
        let event = PageEvent {
            watch,
            file: "a.jpg",
            status: &status,
        };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({ "watch": watch, "file": "a.jpg", "status": "failed", "error": "Gone" })
        );
        assert_eq!(
            json(PageStatus::Read {
                words: 3,
                title: "Bill",
                encrypted: false,
            }),
            r#"{"status":"read","words":3,"title":"Bill"}"#
        );
        assert_eq!(
            json(PageStatus::Read {
                words: 0,
                title: "",
                encrypted: true,
            }),
            r#"{"status":"read","words":0,"title":"","encrypted":true}"#
        );
        let mut counts = ReadCounts {
            read: 2,
            failed: 0,
            stopped: 1,
            limit: false,
        };
        assert_eq!(counts.summary(), "2 read · 1 stopped");
        counts.limit = true;
        assert_eq!(counts.summary(), "2 read · 1 stopped · spend limit reached");
        assert_eq!(ReadCounts::default().summary(), "nothing to read");
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
