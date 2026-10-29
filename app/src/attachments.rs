//! Reading a mail's attachments (mail.md#attachments): after a fetch, the
//! files its filter saved are read into their documents' bodies through
//! `reader` -- an image or a PDF by the filter's read model, a text file as
//! it is -- one after the other as a job, "Reading attachments · <email>".
//! The mails' analysis, if the filter asks for one, starts when it ends.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use av_llm::{ChatResponse, Client, FileData, Provider, StopReason};
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::id::Id;
use av_utils::mime::type_for;
use av_vault::document::Document;

use crate::ai;
use crate::ai_accounts::ModelChoice;
use crate::app::App;
use crate::enrich;
use crate::google_accounts::Analysis;
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use crate::reader::{self, Kind};

/// What the reading is called where its pick is refused
/// (`AiAccounts::choose`).
pub const READING: &str = "reading attachments";

// ----------------------------------------------------------------------------
/// The reads running (`App::attachments`), by job.
#[derive(Default)]
pub struct AttachmentState {
    reads: HashMap<Id, Read>,
}

// ----------------------------------------------------------------------------
/// One read: the model, the documents left and the one being read, and the
/// analysis to start when it ends.
struct Read {
    /// Whose mail it is, for the notices.
    email: String,
    account: Id,
    client: Client,
    provider: Provider,
    address: String,
    model: String,
    queue: VecDeque<Id>,
    current: Option<Id>,
    total: usize,
    counts: Counts,
    /// The mails to analyse afterwards, and how.
    then: Option<(Vec<Id>, Analysis)>,
}

// ----------------------------------------------------------------------------
#[derive(Default)]
struct Counts {
    /// Documents whose text was written.
    read: usize,
    failed: usize,
    /// Files of a type nothing reads, or that the model doesn't take, or
    /// gone.
    unread: usize,
    /// Documents that got text meanwhile, or were removed: left as they are.
    kept: usize,
    /// Not read: stopped, or the spend limit reached.
    left: usize,
    /// The first failure, in words, for the notice.
    first_error: Option<String>,
    limit: bool,
}

impl Counts {
    // ------------------------------------------------------------------------
    /// "3 read · 1 failed · 1 unread".
    fn summary(&self) -> String {
        let counts = [
            (self.read, "read", "read"),
            (self.failed, "failed", "failed"),
            (self.unread, "unread", "unread"),
            (self.left, "left", "left"),
        ];
        count_summary(&counts, "nothing to read")
    }

    // ------------------------------------------------------------------------
    fn done(&self) -> usize {
        self.read + self.failed + self.unread + self.kept
    }
}

// ----------------------------------------------------------------------------
/// What reading a document takes, as `step_for` sees it.
#[derive(Debug, PartialEq)]
enum Step {
    /// It has text already, or is gone: left as it is.
    Keep,
    /// No file, or one nothing reads here or the model doesn't take.
    Unread,
    /// The file `path`, which came as `name`, of the kind `kind`.
    Read {
        kind: Kind,
        path: PathBuf,
        name: String,
    },
}

// ----------------------------------------------------------------------------
/// What reading `doc`, whose original is at `path` (if it has one), takes
/// with a model of `provider`: only an empty body is read, a file of a kind
/// the reader knows -- by the name it came with -- and, for an image or a
/// PDF, that the provider takes.
fn step_for(doc: Option<&Document>, path: Option<PathBuf>, provider: Provider) -> Step {
    let Some(doc) = doc.filter(|doc| doc.content().trim().is_empty()) else {
        return Step::Keep;
    };
    let (Some(original), Some(path)) = (doc.original(), path) else {
        return Step::Unread;
    };
    let name = original.name.clone().or_else(|| {
        let file = path.file_name()?;
        Some(file.to_string_lossy().into_owned())
    });
    let Some(name) = name else {
        return Step::Unread;
    };
    match Kind::of(&name) {
        Some(kind) if !kind.by_model() || kind.sent_to(provider) => Step::Read { kind, path, name },
        _ => Step::Unread,
    }
}

// ----------------------------------------------------------------------------
/// Read the attachment documents `ids` of `email`'s account with the model
/// `pick` as a job ("Reading attachments · <email>"), then analyse the mails
/// `then` asks for. A refused pick is a notice, and the analysis starts at
/// once; so it does if there is nothing to read.
pub fn read(
    app: &mut App,
    email: &str,
    ids: Vec<Id>,
    pick: Option<&ModelChoice>,
    then: Option<(Vec<Id>, Analysis)>,
) {
    let resolved = match app.ai.choose(pick, READING, DateTime::now()) {
        Ok(resolved) => resolved,
        Err(refusal) => {
            app.notice(&format!(
                "Reading attachments · {email}: {}",
                refusal.message
            ));
            return analyse(app, email, then);
        }
    };
    let queue: VecDeque<Id> = ids
        .into_iter()
        .filter(|id| app.vault.index().entry(id).is_some())
        .collect();
    if queue.is_empty() {
        return analyse(app, email, then);
    }
    let total = queue.len();
    let label = format!("Reading attachments · {email}");
    let job = app.start_job(JobKind::AttachmentRead, label, total);
    log::info!(
        "Attachment read {job}: {total} files of {email} with {}",
        resolved.model
    );
    let read = Read {
        email: email.to_string(),
        account: resolved.account,
        client: resolved.client,
        provider: resolved.provider,
        address: resolved.address,
        model: resolved.model,
        queue,
        current: None,
        total,
        counts: Counts::default(),
        then,
    };
    app.attachments.reads.insert(job, read);
    pump(app, job);
}

// ----------------------------------------------------------------------------
/// The mails `then` asks for, analysed (`enrich::analyse`).
fn analyse(app: &mut App, email: &str, then: Option<(Vec<Id>, Analysis)>) {
    if let Some((mails, analysis)) = then {
        enrich::analyse(app, email, mails, &analysis);
    }
}

// ----------------------------------------------------------------------------
/// What the worker got: a text file's text, or the model's answer.
enum Got {
    Text(String),
    Answer(ChatResponse),
}

// ----------------------------------------------------------------------------
/// Why reading a file failed: the file, or the model.
enum ReadError {
    File(Error),
    Model(Error),
}

// ----------------------------------------------------------------------------
/// Read the next document of the read `job`, or finish it if none is left.
/// Documents that need nothing are counted on the way.
fn pump(app: &mut App, job: Id) {
    loop {
        let Some(read) = app.attachments.reads.get_mut(&job) else {
            return;
        };
        if read.current.is_some() {
            return;
        }
        let Some(&id) = read.queue.front() else {
            return finish(app, job);
        };
        let index = app.vault.index();
        let step = step_for(
            index.entry(&id).map(|e| &e.doc),
            index.original_path(&id).filter(|p| p.is_file()),
            read.provider,
        );
        let (kind, path, name) = match step {
            Step::Keep => {
                read.queue.pop_front();
                read.counts.kept += 1;
                continue;
            }
            Step::Unread => {
                read.queue.pop_front();
                read.counts.unread += 1;
                log::info!("Attachment read {job}: {id} left unread");
                continue;
            }
            Step::Read { kind, path, name } => (kind, path, name),
        };
        // The files read so far are counted (`account_used`): at its limit
        // the account takes no more.
        if kind.by_model() {
            if let Err(refusal) = app.ai.check_limit(read.account, DateTime::now()) {
                return stop_at_limit(app, job, &refusal.message);
            }
        }
        let Some(read) = app.attachments.reads.get_mut(&job) else {
            return;
        };
        read.queue.pop_front();
        read.current = Some(id);
        let (client, model) = (read.client.clone(), read.model.clone());
        return app.run_job(
            job,
            move |cancel| {
                let data = std::fs::read(&path).map_err(|e| ReadError::File(e.into()))?;
                if !kind.by_model() {
                    return Ok(Got::Text(reader::text_of(kind, &data)));
                }
                let mime_type = type_for(Path::new(&name)).to_string();
                let file = FileData {
                    name,
                    mime_type,
                    data,
                };
                reader::ask(&client, cancel, &model, kind, file, |_| {})
                    .map(Got::Answer)
                    .map_err(ReadError::Model)
            },
            move |app, result| on_read(app, job, id, kind, result),
        );
    }
}

// ----------------------------------------------------------------------------
/// The document `id` was read: write its text, or count the failure; then
/// the next. Ignored if the read is gone (stopped).
fn on_read(app: &mut App, job: Id, id: Id, kind: Kind, result: Result<Got, ReadError>) {
    let Some(read) = app.attachments.reads.get_mut(&job) else {
        return;
    };
    read.current = None;
    let text = match result {
        Ok(Got::Text(text)) => Ok(text.trim().to_string()),
        Ok(Got::Answer(response)) => {
            let text = reader::clean(&response.text);
            match response.stop {
                _ if !text.is_empty() => Ok(text),
                StopReason::Refused => Err("The model declined to read the file.".to_string()),
                _ => Err("The model read no text.".to_string()),
            }
        }
        Err(ReadError::File(e)) => {
            log::warn!("Attachment read {job}: could not read the file of {id}: {e}");
            Err("Could not read the file.".to_string())
        }
        Err(ReadError::Model(e)) => Err(ai::chat_error(read.provider, &read.address, &e)),
    };
    let text = text.and_then(|text| match text.is_empty() {
        true => Err("The file has no text.".to_string()),
        false => Ok(text),
    });
    let written = text.and_then(|text| write(app, id, text));

    let Some(read) = app.attachments.reads.get_mut(&job) else {
        return;
    };
    let counts = &mut read.counts;
    match written {
        Ok(true) => counts.read += 1,
        Ok(false) => counts.kept += 1,
        Err(error) => {
            log::info!("Attachment read {job}: {id} failed: {error}");
            counts.failed += 1;
            let title = app
                .vault
                .index()
                .entry(&id)
                .map(|e| e.doc.title().to_string());
            let title = title.unwrap_or_default();
            counts
                .first_error
                .get_or_insert_with(|| format!("\u{201c}{title}\u{201d}: {error}"));
        }
    }
    let (account, done, total) = (read.account, read.counts.done(), read.total);
    if kind.by_model() {
        // Saved per file, so a long read keeps what it spent.
        ai::account_used(app, account);
    }
    app.update_job(job, done, total);
    pump(app, job);
}

// ----------------------------------------------------------------------------
/// Write `text` as the body of `id`, while it is still empty: text typed
/// meanwhile stays. Whether it was written; a failed write in words.
fn write(app: &mut App, id: Id, text: String) -> Result<bool, String> {
    let empty = app
        .vault
        .index()
        .get(&id)
        .map(|doc| doc.content().trim().is_empty());
    if empty != Some(true) {
        return Ok(false);
    }
    let outcome = app.edit(|index, changes| index.set_content(&id, format!("{text}\n"), changes));
    match outcome.and_then(|outcome| app.vault.require_written(outcome)) {
        Ok(()) => Ok(true),
        Err(e) => {
            log::warn!("Attachment read: {id} could not be written: {e}");
            Err("could not be written.".to_string())
        }
    }
}

// ----------------------------------------------------------------------------
/// Stop the read `job` (`jobs::stop` has cancelled its request): the file
/// being read and those waiting are left, and the job ends.
pub fn stop(app: &mut App, job: Id) {
    let Some(read) = app.attachments.reads.get_mut(&job) else {
        return;
    };
    let left = read.queue.len() + usize::from(read.current.is_some());
    read.queue.clear();
    read.current = None;
    read.counts.left += left;
    log::info!("Attachment read {job}: stopped");
    finish(app, job);
}

// ----------------------------------------------------------------------------
/// The read's account has reached its spend limit (`message` says so): say
/// it once, and stop the read as Stop does.
fn stop_at_limit(app: &mut App, job: Id, message: &str) {
    let Some(read) = app.attachments.reads.get_mut(&job) else {
        return;
    };
    read.counts.limit = true;
    let notice = format!("Reading attachments · {}: {message}", read.email);
    app.notice(&notice);
    stop(app, job);
}

// ----------------------------------------------------------------------------
/// The read `job` is done or stopped: end the job with its summary, record
/// what it spent, and tell the failures in one notice. Unless it was
/// stopped, the analysis it was handed starts.
fn finish(app: &mut App, job: Id) {
    let Some(read) = app.attachments.reads.remove(&job) else {
        return;
    };
    let counts = &read.counts;
    let stopped = app.job_stopping(job);
    let mut note = counts.summary();
    if counts.limit {
        note.push_str(" · spend limit reached");
    } else if stopped {
        note = format!("stopped · {note}");
    }
    log::info!("Attachment read {job}: {note}");
    app.end_job(job, &note);
    ai::account_used(app, read.account);
    if let Some(error) = &counts.first_error {
        let (failed, total) = (counts.failed, read.total);
        let files = if total == 1 { "file" } else { "files" };
        app.notice(&format!(
            "Reading attachments · {}: {failed} of {total} {files} couldn't be read; {error}",
            read.email
        ));
    }
    if !stopped {
        analyse(app, &read.email, read.then);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google_accounts::AnalysisLevel;
    use crate::testing::{FakeOllama, Harness, parse};
    use serde_json::Value;
    use std::fs;

    const OLLAMA_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b70";
    const SCAN: &str = "01a0e1cc-a24c-7490-82fe-e75669985b71";
    const NOTES: &str = "01a0e1cc-a24c-7490-82fe-e75669985b72";
    const BILL: &str = "01a0e1cc-a24c-7490-82fe-e75669985b73";
    const ZIP: &str = "01a0e1cc-a24c-7490-82fe-e75669985b74";
    const TYPED: &str = "01a0e1cc-a24c-7490-82fe-e75669985b75";
    const MAIL: &str = "01a0e1cc-a24c-7490-82fe-e75669985b76";

    // ------------------------------------------------------------------------
    /// An attachment document `id` in `Attachments/` with the original
    /// `file`, which came as `name`, and the body `body`.
    fn attachment(id: &str, file: &str, name: &str, body: &str) -> (String, String) {
        let text = format!(
            "---\nid: {id}\ntitle: {name}\ndocument:\n  original:\n    file: {file}\n    name: {name}\n    size: 4\n---\n{body}"
        );
        (format!("Attachments/{file}.md"), text)
    }

    // ------------------------------------------------------------------------
    /// A vault with an Ollama account at `address` offering `m`, a mail,
    /// and the attachments: a scan (an image), notes (text), a bill (a
    /// PDF), an archive, and a document with text typed into it.
    fn with_attachments(name: &str, address: &str) -> Harness {
        let account = format!("id: {OLLAMA_ID}\nollama:\n  address: {address}\n  offered: [m]\n");
        let mail = format!(
            "---\nid: {MAIL}\ntitle: Abschlag\nemail:\n  from: a@x\n---\nYour bill is attached.\n"
        );
        let mut files = vec![
            (".avelor/accounts/ollama.yaml".to_string(), account),
            ("Mail/a.md".to_string(), mail),
        ];
        let originals = [
            (SCAN, "scan.jpg", "Scan.JPG", ""),
            (NOTES, "notes.txt", "Notes.txt", ""),
            (BILL, "bill.pdf", "Bill.pdf", ""),
            (ZIP, "photos.zip", "Photos.zip", ""),
            (TYPED, "typed.jpg", "Typed.jpg", "Typed by hand.\n"),
        ];
        for (id, file, name, body) in originals {
            files.push(attachment(id, file, name, body));
            files.push((format!("Attachments/{file}"), "Gr\u{fc}\u{df}e".to_string()));
        }
        Harness::with_vault(name, &files)
    }

    // ------------------------------------------------------------------------
    fn pick() -> ModelChoice {
        ModelChoice {
            account: Id::parse(OLLAMA_ID).unwrap(),
            model: "m".into(),
        }
    }

    // ------------------------------------------------------------------------
    fn ids(list: &[&str]) -> Vec<Id> {
        list.iter().map(|id| Id::parse(id).unwrap()).collect()
    }

    // ------------------------------------------------------------------------
    fn of<'a>(sent: &'a [Value], ev: &str) -> Vec<&'a Value> {
        sent.iter()
            .filter(|m| m["ev"] == ev)
            .map(|m| &m["data"])
            .collect()
    }

    // ------------------------------------------------------------------------
    /// What the page got until `jobs` jobs ended, parsed.
    fn until_done(h: &mut Harness, jobs: usize) -> Vec<Value> {
        let ended = |sent: &[String]| {
            let done = sent.iter().filter(|m| m.contains(r#""state":"done""#));
            done.count() >= jobs
        };
        parse(&h.wait_for(ended))
    }

    // ------------------------------------------------------------------------
    fn body(h: &Harness, file: &str) -> String {
        let text = fs::read_to_string(h.vault().join("Attachments").join(file)).unwrap();
        text.split_once("\n---\n").unwrap().1.to_string()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn what_a_document_takes_goes_by_its_body_its_file_and_the_provider() {
        let mut doc = Document::create(
            Default::default(),
            av_vault::envelope::Source::Created(Default::default()),
            "Bill".into(),
            Vec::new(),
            av_vault::document::FileFormat::Markdown,
        );
        let path = PathBuf::from("bill.pdf");
        assert_eq!(step_for(None, None, Provider::Ollama), Step::Keep);
        assert_eq!(
            step_for(Some(&doc), Some(path.clone()), Provider::Anthropic),
            Step::Unread,
            "no original"
        );
        let original = |name: &str| av_vault::envelope::Original {
            file: Some("bill.pdf".into()),
            name: Some(name.into()),
            size: Some(4),
        };
        doc.set_original(original("Rechnung.pdf"));
        assert_eq!(
            step_for(Some(&doc), None, Provider::Anthropic),
            Step::Unread,
            "no file"
        );
        let read = Step::Read {
            kind: Kind::Pdf,
            path: path.clone(),
            name: "Rechnung.pdf".into(),
        };
        assert_eq!(
            step_for(Some(&doc), Some(path.clone()), Provider::Anthropic),
            read
        );
        assert_eq!(
            step_for(Some(&doc), Some(path.clone()), Provider::Ollama),
            Step::Unread
        );
        doc.set_original(original("Rechnung.zip"));
        assert_eq!(
            step_for(Some(&doc), Some(path.clone()), Provider::Anthropic),
            Step::Unread
        );
        doc.set_original(original("Rechnung.txt"));
        let Step::Read { kind, .. } = step_for(Some(&doc), Some(path.clone()), Provider::Ollama)
        else {
            panic!("a text file is read on Ollama too");
        };
        assert_eq!(kind, Kind::Plain);
        doc.set_content("Typed.\n".into());
        assert_eq!(
            step_for(Some(&doc), Some(path), Provider::Anthropic),
            Step::Keep
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_read_writes_the_text_into_empty_bodies_then_analyses_the_mails() {
        let analysis = r#"{"kind": "notification", "title": "Abschlag Oktober"}"#;
        let address = FakeOllama::new(&["m"])
            .answering_in_turn(&["# Rechnung\n\n42,00 EUR", analysis])
            .start();
        let mut h = with_attachments("attachments-read", &address);
        let then = Analysis {
            level: AnalysisLevel::Lite,
            model: Some(pick()),
        };
        let all = ids(&[SCAN, NOTES, BILL, ZIP, TYPED]);
        read(
            &mut h.app,
            "jan@example.com",
            all,
            Some(&pick()),
            Some((ids(&[MAIL]), then)),
        );
        let sent = until_done(&mut h, 2);

        let jobs = of(&sent, "job");
        assert_eq!(jobs[0]["label"], "Reading attachments · jan@example.com");
        let notes: Vec<&Value> = jobs
            .iter()
            .map(|j| &j["note"])
            .filter(|n| !n.is_null())
            .collect();
        assert!(
            notes.contains(&&Value::from("2 read · 2 unread")),
            "{notes:?}"
        );
        assert!(notes.contains(&&Value::from("1 analysed")), "{notes:?}");
        assert!(of(&sent, "notice").is_empty(), "{sent:?}");

        assert_eq!(body(&h, "scan.jpg.md"), "# Rechnung\n\n42,00 EUR\n");
        assert_eq!(body(&h, "notes.txt.md"), "Gr\u{fc}\u{df}e\n");
        assert_eq!(body(&h, "bill.pdf.md"), "", "Ollama takes no PDF");
        assert_eq!(body(&h, "photos.zip.md"), "");
        assert_eq!(body(&h, "typed.jpg.md"), "Typed by hand.\n");
        let mail = fs::read_to_string(h.vault().join("Mail/a.md")).unwrap();
        assert!(mail.contains("title: Abschlag Oktober"), "{mail}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_refused_pick_reads_nothing_and_still_analyses() {
        let address = FakeOllama::new(&["m"])
            .replying(&[r#"{"kind": "notification", "title": "Abschlag Oktober"}"#])
            .start();
        let mut h = with_attachments("attachments-refused", &address);
        let then = Analysis {
            level: AnalysisLevel::Lite,
            model: Some(pick()),
        };
        read(
            &mut h.app,
            "jan@example.com",
            ids(&[SCAN]),
            None,
            Some((ids(&[MAIL]), then)),
        );
        let sent = until_done(&mut h, 1);

        let jobs = of(&sent, "job");
        assert_eq!(jobs[0]["label"], "Analysing mail · jan@example.com");
        let notices = of(&sent, "notice");
        assert_eq!(
            notices[0]["message"],
            "Reading attachments · jan@example.com: For reading attachments, pick a model."
        );
        assert_eq!(body(&h, "scan.jpg.md"), "");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn counts_read_well() {
        let counts = Counts {
            read: 3,
            failed: 1,
            unread: 1,
            kept: 2,
            ..Counts::default()
        };
        assert_eq!(counts.summary(), "3 read · 1 failed · 1 unread");
        assert_eq!(counts.done(), 7);
        assert_eq!(Counts::default().summary(), "nothing to read");
    }
}
