//! Import scans (import.md#import-scans): the files of an images folder watch,
//! read by an AI account's OCR model and imported as documents into `Scans/`.
//!
//! The page lists the files, has the ticked ones read (a job reading one file
//! after the other, which `cancel_job` stops), arranges the pages into
//! documents and imports them; the commands of the typed protocol
//! (api.md#scans). What a file read to is kept in memory while the app runs.
//! Every document written carries `scan: {watch, origin: [files]}`, which is
//! how a later listing tells the files already in the vault
//! (`Index::scan_origins`).
//!
//! The page shows the files as previews: `scan_previews` renders them
//! (`sys::preview`: WIC on Windows, gdk-pixbuf on Linux) on a worker and keeps
//! them while the app runs; `scan_open` opens a file in its default app.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use av_llm::{ChatRequest, ChatResponse, Client, FileData, Message, Part, Provider, StopReason};
use av_utils::base64::to_base64;
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::envelope::{Document as DocumentRole, Roles, Scan, Source};
use av_vault::schema::layout;
use av_vault::shared::OneOrMany;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use crate::ai;
use crate::app::App;
use crate::app::Task;
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use crate::protocol::Req;
use crate::sys;
use crate::watches::READ_STEP;
use crate::watches::{self, FolderFile};
use av_vault::markdown::plain_title;

/// The longest proposed title, in characters.
const TITLE_LEN: usize = 80;
/// How many more words a page must have before its progress is sent again.
const PROGRESS_WORDS: usize = 10;
/// What the OCR model is told, and asked.
const OCR_SYSTEM: &str = "You transcribe scanned pages. Write the text of the \
page in the image as Markdown: keep its headings, paragraphs, lists and tables, \
in reading order. Write only the page's text: no comments, no code fence.";
const OCR_REQUEST: &str = "Transcribe this page.";
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
/// A file's text, as the OCR model read it.
struct Ocr {
    /// The file as it was read.
    stamp: Stamp,
    text: String,
    words: usize,
    title: String,
}

// ----------------------------------------------------------------------------
/// A read in progress: files of one watch, read one after the other.
struct ScanRead {
    job: Id,
    watch: Id,
    /// The watch's folder, which the files' names are relative to.
    root: PathBuf,
    account: Id,
    client: Client,
    provider: Provider,
    address: String,
    model: String,
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
    /// Empty: the first page's proposed title.
    #[serde(default)]
    title: String,
    files: Vec<String>,
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
/// Whether `name` is a file the OCR models take: JPEG, PNG, GIF or WebP.
fn is_image(name: &str) -> bool {
    av_utils::mime::type_for(Path::new(name)).starts_with("image/")
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
    let (root, watch_name, picked) = match app.watches.get(request.watch) {
        Ok(watch) => {
            let settings = watch.settings();
            let picked = settings.model(READ_STEP).cloned();
            (settings.root(), settings.display_name(), picked)
        }
        Err(e) => return req.err(e),
    };
    let watch = request.watch;
    if app.scans.read.as_ref().is_some_and(|r| r.watch != watch) {
        let what = "reading a second folder".to_string();
        let message = "Another folder is still being read; stop it or wait for it.";
        return refuse(req, Error::Unsupported { what }, message);
    }
    let resolved = match app.ai.choose(picked.as_ref(), "OCR", DateTime::now()) {
        Ok(resolved) => resolved,
        Err(refusal) => return refuse(req, refusal.error, refusal.message),
    };

    let mut queue = Vec::new();
    for name in request.files {
        if app.scans.status(watch, &name).is_some() || queue.contains(&name) {
            continue;
        }
        if !is_image(&name) {
            let error = "Only JPEG, PNG, GIF and WebP images can be read.";
            post_page(app, watch, &name, &PageStatus::Failed { error });
            continue;
        }
        let file = FolderFile::read(&root, &name).ok();
        let cached = file.as_ref().and_then(|f| app.scans.cached(f));
        match cached {
            Some(ocr) if !request.again => {
                let (words, title) = (ocr.words, ocr.title.as_str());
                post_page(app, watch, &name, &PageStatus::Read { words, title });
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
        let label = format!("Reading scans · {watch_name}");
        let job = app.start_job(JobKind::ScanRead, label, total);
        log::info!("Scan read {job}: reading {total} files of {watch_name}");
        app.scans.read = Some(ScanRead {
            job,
            watch,
            root,
            account: resolved.account,
            client: resolved.client,
            provider: resolved.provider,
            address: resolved.address,
            model: resolved.model,
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
    if let Err(refusal) = app.ai.check_limit(read.account, DateTime::now()) {
        return stop_at_limit(app, &refusal.message);
    }
    let Some(read) = &mut app.scans.read else {
        return;
    };
    let Some(name) = read.queue.pop_front() else {
        return finish(app);
    };
    let (job, watch, root) = (read.job, read.watch, read.root.clone());
    let (client, model) = (read.client.clone(), read.model.clone());
    post_page(app, watch, &name, &PageStatus::Reading { words: 0 });

    let (file, progress_file, done_file) = (name.clone(), name.clone(), name.clone());
    app.run_job_with_progress(
        job,
        move |progress, cancel| {
            ocr(&client, cancel, &model, &root, &file, |words| {
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
/// Why reading a file failed: the file, or the model.
enum ReadError {
    File(Error),
    Model(Error),
}

// ----------------------------------------------------------------------------
/// Read the image `name` in `root` with `model`: the file as it was read,
/// and the model's answer. `on_words` gets the words read so far as the
/// answer streams in.
fn ocr(
    client: &Client,
    cancel: &Cancel,
    model: &str,
    root: &Path,
    name: &str,
    mut on_words: impl FnMut(usize),
) -> Result<(FolderFile, ChatResponse), ReadError> {
    // The file's facts first: a change while it is read makes the text stale.
    let file = FolderFile::read(root, name).map_err(ReadError::File)?;
    let data = FileData::from_path(&file.path).map_err(ReadError::File)?;
    let request = ChatRequest::new(model)
        .system(OCR_SYSTEM)
        .message(Message::user(OCR_REQUEST).with(Part::Inline(data)))
        .context(ai::CONTEXT_TOKENS);
    let mut text = String::new();
    let mut sent = 0;
    let response = client
        .chat_stream(cancel, &request, |delta| {
            text.push_str(delta);
            let words = word_count(&text);
            if words >= sent + PROGRESS_WORDS {
                sent = words;
                on_words(words);
            }
        })
        .map_err(ReadError::Model)?;
    Ok((file, response))
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
    result: Result<(FolderFile, ChatResponse), ReadError>,
) {
    let Some(read) = app.scans.read.as_mut().filter(|r| r.job == job) else {
        return;
    };
    read.current = None;
    let page = match result {
        Ok((file, response)) => page_of(file, response),
        Err(ReadError::File(e)) => {
            log::warn!("Scan read {job}: could not read {name}: {e}");
            Err("Could not read the file.".to_string())
        }
        Err(ReadError::Model(e)) => Err(ai::chat_error(read.provider, &read.address, &e)),
    };
    let (watch, account) = (read.watch, read.account);
    match page {
        Ok((path, ocr)) => {
            read.counts.read += 1;
            let (words, title) = (ocr.words, ocr.title.as_str());
            post_page(app, watch, &name, &PageStatus::Read { words, title });
            app.scans.cache.insert(path, ocr);
        }
        Err(error) => {
            read.counts.failed += 1;
            log::info!("Scan read {job}: {name} failed: {error}");
            post_page(app, watch, &name, &PageStatus::Failed { error: &error });
        }
    }
    // Saved per page, so a long read keeps what it spent.
    ai::account_used(app, account);
    post_progress(app);
    pump(app);
}

// ----------------------------------------------------------------------------
/// The page a file read to, by the file's path; or why it has none.
fn page_of(file: FolderFile, response: ChatResponse) -> Result<(PathBuf, Ocr), String> {
    let text = clean_ocr(&response.text);
    if text.is_empty() {
        return Err(match response.stop {
            StopReason::Refused => "The model declined to read the page.",
            _ => "The model read no text.",
        }
        .to_string());
    }
    let ocr = Ocr {
        stamp: Stamp::of(&file),
        words: word_count(&text),
        title: title_of(&text),
        text,
    };
    Ok((file.path, ocr))
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
    ai::account_used(app, read.account);
}

// ----------------------------------------------------------------------------
/// `scan_import {watch, docs}`: import each document into `Scans/` from its
/// pages' text (`scan_document`). Answers `{imported, failed}` after an
/// `entity` event per document written.
pub fn scan_import(app: &mut App, req: Req, request: ImportRequest) {
    let (watch, root) = match app.watches.get(request.watch) {
        Ok(watch) => (watch.id(), watch.settings().root()),
        Err(e) => return req.err(e),
    };
    let mut reply = ImportReply::default();
    for doc in &request.docs {
        match write_document(app, watch, &root, doc) {
            Ok(id) => reply.imported.push(Imported {
                files: &doc.files,
                id,
            }),
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
    req.ok(reply);
}

// ----------------------------------------------------------------------------
/// Write the document of `doc`'s pages; or say why not.
fn write_document(app: &mut App, watch: Id, root: &Path, doc: &NewDocument) -> Result<Id, String> {
    let document = {
        let pages = gather_pages(app, watch, root, doc)?;
        scan_document(watch, &doc.title, &pages)
    };
    let folder = Path::new(layout::SCANS);
    app.edit(|index, changes| index.create(document, folder, changes))
        .map_err(|e| {
            log::warn!("Scan import: could not write the document: {e}");
            "Could not write the document.".to_string()
        })
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
}

// ----------------------------------------------------------------------------
/// The pages of `doc`: each not in the vault yet, still in the folder, and
/// read as it is now; or why the document can't be imported.
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
        if imported.contains_key(name) {
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
    let texts: Vec<&str> = pages.iter().map(|page| page.text).collect();
    document.set_content(format!("{}\n", texts.join("\n\n")));
    document
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
/// The model's answer as page text: trimmed, without a code fence around
/// it (which some models add despite being told not to).
fn clean_ocr(text: &str) -> String {
    let text = text.trim();
    let unfenced = text.strip_prefix("```").and_then(|rest| {
        let (info, body) = rest.split_once('\n')?;
        // Only an info string (`markdown`), not a fenced block that starts
        // the page and ends before its end.
        if info.contains('`') || info.trim().contains(' ') {
            return None;
        }
        body.trim_end().strip_suffix("```")
    });
    unfenced.unwrap_or(text).trim().to_string()
}

// ----------------------------------------------------------------------------
/// The words of `text`: the runs between whitespace that have a letter or a
/// digit (markdown's `#`, `-` and `|` don't count).
fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|word| word.chars().any(char::is_alphanumeric))
        .count()
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
    use crate::testing::{FakeOllama, parse, shape};
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
    /// Keep `text` as what the file `name` of `folder` read to.
    fn read_page(h: &mut Harness, folder: &Path, name: &str, text: &str) {
        let file = FolderFile::read(folder, name).unwrap();
        let ocr = Ocr {
            stamp: Stamp::of(&file),
            words: word_count(text),
            title: title_of(text),
            text: text.to_string(),
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
        };
        let second = ScanPage {
            file: "scan_0807.jpg",
            taken: later,
            text: "Page two",
            title: "Page two",
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
    fn answers_lose_a_fence_around_them() {
        assert_eq!(clean_ocr("  # Bill\n\nText\n "), "# Bill\n\nText");
        assert_eq!(
            clean_ocr("```markdown\n# Bill\n\nText\n```\n"),
            "# Bill\n\nText"
        );
        assert_eq!(clean_ocr("```\nText\n```"), "Text");
        let code = "```\nfn a()\n```\nAfter the code.";
        assert_eq!(clean_ocr(code), code, "a fenced block at the start stays");
        assert_eq!(clean_ocr(" \n"), "");
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
    fn words_are_runs_with_a_letter_or_digit() {
        assert_eq!(word_count("# Bill\n\n- 3 x 4,50 EUR\n| a | b |"), 7);
        assert_eq!(word_count(""), 0);
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
                title: "Bill"
            }),
            r#"{"status":"read","words":3,"title":"Bill"}"#
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
        assert!(is_image("scan.JPG") && is_image("p1.webp") && !is_image("a.pdf"));
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
