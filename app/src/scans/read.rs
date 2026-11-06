//! Reading scans: each page's text and title, read by a model.

use std::path::{Path, PathBuf};

use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::id::Id;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use super::{Ocr, Stamp, analysis_works, refuse, refuse_statements};
use crate::ai;
use crate::ai::queue::AiQueue;
use crate::app::App;
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use crate::jobs::job_summary;
use crate::protocol::Req;
use crate::reader::{self, Engine, Kind, ReadError};
use crate::texts::Text;
use crate::watches::{FolderFile, READ_STEP, WatchKind};
use av_utils::is_false;
use av_vault::markdown::plain_title;

/// The longest proposed title, in characters.
pub(super) const TITLE_LEN: usize = 80;

// ----------------------------------------------------------------------------
/// A read in progress: files of one watch, read one after the other.
pub(super) struct ScanRead {
    pub(super) job: Id,
    pub(super) watch: Id,
    /// The watch's folder, which the files' names are relative to.
    pub(super) root: PathBuf,
    pub(super) kind: WatchKind,
    engine: Engine,
    /// The files by name, on the model's account if one reads.
    pub(super) queue: AiQueue<String>,
    /// The files read.
    pub(super) read: usize,
}

impl ScanRead {
    // ------------------------------------------------------------------------
    /// Its counts, as its summary says them.
    fn counts(&self) -> ReadCounts {
        ReadCounts {
            read: self.read,
            failed: self.queue.failed(),
            stopped: self.queue.left(),
            limit: self.queue.limit(),
        }
    }
}

// ----------------------------------------------------------------------------
#[derive(Default)]
pub(super) struct ReadCounts {
    pub(super) read: usize,
    pub(super) failed: usize,
    pub(super) stopped: usize,
    /// Stopped at the account's spend limit.
    pub(super) limit: bool,
}

impl ReadCounts {
    // ------------------------------------------------------------------------
    /// "5 read · 1 failed": the job's summary; "· spend limit reached"
    /// if that stopped it. The files a stop left say it, not "stopped ·".
    pub(super) fn summary(&self) -> Vec<Text> {
        let counts = vec![
            (self.read, Text::new("count.read")),
            (self.failed, Text::new("count.failed")),
            (self.stopped, Text::new("count.stopped")),
        ];
        let counts = count_summary(counts, Text::new("job.nothing_to_read"));
        job_summary(counts, false, self.limit)
    }
}

// ----------------------------------------------------------------------------
/// Where a file stands in a read (the `scan_page` event).
#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub(super) enum PageStatus<'a> {
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
        error: &'a Text,
    },
    Stopped,
}

// ----------------------------------------------------------------------------
/// The `scan_page` event: `{watch, file, status, …}`.
#[derive(Serialize)]
pub(super) struct PageEvent<'a> {
    pub(super) watch: Id,
    pub(super) file: &'a str,
    #[serde(flatten)]
    pub(super) status: &'a PageStatus<'a>,
}

// ----------------------------------------------------------------------------
/// `scan_read`'s args.
#[derive(Deserialize)]
pub(crate) struct ReadRequest {
    pub(super) watch: Id,
    pub(super) files: Vec<String>,
    /// Read files that were read before too.
    #[serde(default)]
    pub(super) again: bool,
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
        let text = Text::new("scans.read_running");
        return refuse(req, Error::Unsupported { what }, text);
    }
    let (what, reading) = match kind {
        WatchKind::Images => (Text::new("feature.ocr"), Text::new("scans.reading")),
        WatchKind::Pdf => (
            Text::new("feature.scanned_pages"),
            Text::new("scans.reading_pdfs"),
        ),
        WatchKind::Statements => return refuse_statements(req),
    };
    // A PDF watch reads text layers without a model.
    let (engine, account) = match app.ai.choose(picked.as_ref(), what, DateTime::now()) {
        Ok(resolved) => {
            let account = resolved.account;
            (Engine::Ai(resolved), Some(account))
        }
        Err(refusal) if kind == WatchKind::Pdf => {
            let why = refusal.text;
            (Engine::None { why }, None)
        }
        Err(refusal) => return refuse(req, refusal.error, refusal.text),
    };

    let mut queue = Vec::new();
    for name in request.files {
        if app.scans.status(watch, &name).is_some() || queue.contains(&name) {
            continue;
        }
        if let Err(error) = readable(kind, &name) {
            post_page(app, watch, &name, &PageStatus::Failed { error: &error });
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
        read.queue.extend(queue);
        post_progress(app);
    } else {
        let label = reading.arg("watch", watch_name.as_str());
        let job = app.start_job(JobKind::ScanRead, label, total);
        log::info!("Scan read {job}: reading {total} files of {watch_name}");
        app.scans.read = Some(ScanRead {
            job,
            watch,
            root,
            kind,
            engine,
            queue: AiQueue::new(account, queue.into()),
            read: 0,
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
    if read.queue.is_busy() {
        return;
    }
    if read.queue.is_empty() {
        return finish(app);
    }
    if let Err(text) = read.queue.check_limit(&app.ai) {
        return stop_at_limit(app, text);
    }
    let Some(name) = read.queue.start().cloned() else {
        return finish(app);
    };
    let (job, watch, root) = (read.job, read.watch, read.root.clone());
    let engine = read.engine.clone();
    let kind = readable(read.kind, &name).unwrap_or(Kind::Image);
    post_page(app, watch, &name, &PageStatus::Reading { words: 0 });

    let (file, progress_file, done_file) = (name.clone(), name.clone(), name);
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
}

// ----------------------------------------------------------------------------
/// The kind of file the watch of `kind` reads `name` as; or why it doesn't
/// read it.
pub(super) fn readable(kind: WatchKind, name: &str) -> Result<Kind, Text> {
    match (kind, Kind::of(name)) {
        (WatchKind::Images, Some(Kind::Image)) => Ok(Kind::Image),
        (WatchKind::Images, _) => Err(Text::new("scans.images_only")),
        (WatchKind::Pdf, Some(Kind::Pdf)) => Ok(Kind::Pdf),
        (WatchKind::Pdf, _) => Err(Text::new("scans.pdfs_only")),
        (WatchKind::Statements, _) => Err(Text::new("scans.statements_as_they_are")),
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
    if read.job == job && read.queue.current().is_some_and(|current| current == name) {
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
    read.queue.end();
    let page = match result {
        Ok((file, Ok(text))) => Ok(page_of(file, text, false)),
        Ok((file, Err(ReadError::Encrypted))) => Ok(page_of(file, String::new(), true)),
        Ok((_, Err(e))) => Err(e.words(&read.engine)),
        Err(e) => {
            log::warn!("Scan read {job}: could not read {name}: {e}");
            Err(Text::new("reader.unread"))
        }
    };
    let (watch, account) = (read.watch, read.queue.account());
    match page {
        Ok((path, ocr)) => {
            read.read += 1;
            let status = PageStatus::Read {
                words: ocr.words,
                title: &ocr.title,
                encrypted: ocr.encrypted,
            };
            post_page(app, watch, &name, &status);
            app.scans.cache.insert(path, ocr);
        }
        Err(error) => {
            read.queue.fail(&name, error.clone());
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
    let (job, done, total) = (read.job, read.queue.done(), read.queue.total());
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
    let stopped = read.queue.stop();
    let watch = read.watch;
    log::info!("Scan read {}: stopped", read.job);
    for name in &stopped {
        post_page(app, watch, name, &PageStatus::Stopped);
    }
    finish(app);
}

// ----------------------------------------------------------------------------
/// The read's account has reached its spend limit (`text` says so): say it
/// once, and stop the read as Stop does, the files left `stopped`.
fn stop_at_limit(app: &mut App, text: Text) {
    let Some(read) = &app.scans.read else {
        return;
    };
    log::info!("Scan read {}: {text}", read.job);
    app.notice(text);
    stop_read(app);
}

// ----------------------------------------------------------------------------
/// The read is done: report it, and the account's spending (what the
/// file being read when it stopped cost, too).
fn finish(app: &mut App) {
    let Some(read) = app.scans.read.take() else {
        return;
    };
    app.end_job(read.job, read.counts().summary());
    if let Some(account) = read.queue.account() {
        ai::account_used(app, account);
    }
}

// ----------------------------------------------------------------------------
/// A title proposed for a page: its first heading, else its first line,
/// as plain text (`plain_title`).
pub(super) fn title_of(text: &str) -> String {
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
