//! Reading scans: each page's text and title, read by a model.

use std::path::{Path, PathBuf};

use av_llm::{ChatResponse, Client, FileData, Provider, StopReason};
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::id::Id;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;

use super::{Ocr, Stamp, refuse};
use crate::ai;
use crate::ai::queue::AiQueue;
use crate::app::App;
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use crate::jobs::job_summary;
use crate::protocol::Req;
use crate::reader;
use crate::texts::Text;
use crate::watches::FolderFile;
use crate::watches::READ_STEP;
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
    client: Client,
    provider: Provider,
    pub(super) address: String,
    model: String,
    /// The files by name, on the model's account.
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
/// Whether `name` is a file the OCR models take: JPEG, PNG, GIF or WebP.
pub(super) fn is_image(name: &str) -> bool {
    av_utils::mime::type_for(Path::new(name)).starts_with("image/")
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
        let text = Text::new("scans.read_running");
        return refuse(req, Error::Unsupported { what }, text);
    }
    let resolved = match app
        .ai
        .choose(picked.as_ref(), Text::new("feature.ocr"), DateTime::now())
    {
        Ok(resolved) => resolved,
        Err(refusal) => return refuse(req, refusal.error, refusal.text),
    };

    let mut queue = Vec::new();
    for name in request.files {
        if app.scans.status(watch, &name).is_some() || queue.contains(&name) {
            continue;
        }
        if !is_image(&name) {
            let error = Text::new("scans.images_only");
            post_page(app, watch, &name, &PageStatus::Failed { error: &error });
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
        read.queue.extend(queue);
        post_progress(app);
    } else {
        let label = Text::new("scans.reading").arg("watch", watch_name.to_string());
        let job = app.start_job(JobKind::ScanRead, label, total);
        log::info!("Scan read {job}: reading {total} files of {watch_name}");
        app.scans.read = Some(ScanRead {
            job,
            watch,
            root,
            client: resolved.client,
            provider: resolved.provider,
            address: resolved.address,
            model: resolved.model,
            queue: AiQueue::new(Some(resolved.account), queue.into()),
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
    let (client, model) = (read.client.clone(), read.model.clone());
    post_page(app, watch, &name, &PageStatus::Reading { words: 0 });

    let (file, progress_file, done_file) = (name.clone(), name.clone(), name);
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
}

// ----------------------------------------------------------------------------
/// Why reading a file failed: the file, or the model.
enum ReadError {
    File(Error),
    Model(Error),
}

// ----------------------------------------------------------------------------
/// Read the image `name` in `root` with `model` (`reader::ask`): the file
/// as it was read, and the model's answer. `on_words` gets the words read
/// so far as the answer streams in.
fn ocr(
    client: &Client,
    cancel: &Cancel,
    model: &str,
    root: &Path,
    name: &str,
    on_words: impl FnMut(usize),
) -> Result<(FolderFile, ChatResponse), ReadError> {
    // The file's facts first: a change while it is read makes the text stale.
    let file = FolderFile::read(root, name).map_err(ReadError::File)?;
    let data = FileData::from_path(&file.path).map_err(ReadError::File)?;
    let response = reader::ask(client, cancel, model, data, on_words).map_err(ReadError::Model)?;
    Ok((file, response))
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
    result: Result<(FolderFile, ChatResponse), ReadError>,
) {
    let Some(read) = app.scans.read.as_mut().filter(|r| r.job == job) else {
        return;
    };
    read.queue.end();
    let page = match result {
        Ok((file, response)) => page_of(file, response),
        Err(ReadError::File(e)) => {
            log::warn!("Scan read {job}: could not read {name}: {e}");
            Err(Text::new("reader.unread"))
        }
        Err(ReadError::Model(e)) => Err(ai::chat_error(read.provider, &read.address, &e)),
    };
    let (watch, account) = (read.watch, read.queue.account());
    match page {
        Ok((path, ocr)) => {
            read.read += 1;
            let (words, title) = (ocr.words, ocr.title.as_str());
            post_page(app, watch, &name, &PageStatus::Read { words, title });
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
/// The page a file read to, by the file's path; or why it has none.
fn page_of(file: FolderFile, response: ChatResponse) -> Result<(PathBuf, Ocr), Text> {
    let text = reader::clean(&response.text);
    if text.is_empty() {
        return Err(match response.stop {
            StopReason::Refused => Text::new("reader.refused"),
            _ => Text::new("reader.blank"),
        });
    }
    let ocr = Ocr {
        stamp: Stamp::of(&file),
        words: reader::word_count(&text),
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
