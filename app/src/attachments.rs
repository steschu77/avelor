//! Reading a mail's attachments (mail.md#attachments): after a fetch, the
//! files its filter saved are read into their documents' bodies through
//! `reader` -- an image by the filter's read model, a PDF from its text
//! layer or by the model, a text file as it is -- one after the other as a
//! job, "Reading attachments · <email>".
//! The analysis, if the filter asks for one, starts when it ends, with the
//! attachments it read.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_vault::document::Document;

use crate::ai;
use crate::ai_accounts::ModelChoice;
use crate::app::App;
use crate::enrich::Analysis;
use crate::enrich::{self, Batch};
use crate::jobs::JobKind;
use crate::jobs::count_summary;
use crate::reader::{self, Engine, Kind, ReadError};

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
    engine: Engine,
    queue: VecDeque<Id>,
    current: Option<Id>,
    total: usize,
    counts: Counts,
    /// The documents whose text it wrote.
    written: Vec<Id>,
    /// The mails to analyse afterwards, and how.
    then: Option<(Vec<Id>, Analysis)>,
}

// ----------------------------------------------------------------------------
#[derive(Default)]
struct Counts {
    /// Documents whose text was written.
    read: usize,
    failed: usize,
    /// Files of a type nothing reads, or gone.
    unread: usize,
    /// Encrypted PDFs: marked, not read.
    encrypted: usize,
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
    /// "3 read · 1 failed · 1 unread · 1 encrypted".
    fn summary(&self) -> String {
        let counts = [
            (self.read, "read", "read"),
            (self.failed, "failed", "failed"),
            (self.unread, "unread", "unread"),
            (self.encrypted, "encrypted", "encrypted"),
            (self.left, "left", "left"),
        ];
        count_summary(&counts, "nothing to read")
    }

    // ------------------------------------------------------------------------
    fn done(&self) -> usize {
        self.read + self.failed + self.unread + self.encrypted + self.kept
    }
}

// ----------------------------------------------------------------------------
/// What reading a document takes, as `step_for` sees it.
#[derive(Debug, PartialEq)]
enum Step {
    /// It has text already, or is gone: left as it is.
    Keep,
    /// No file, or one nothing reads here.
    Unread,
    /// A PDF marked encrypted.
    Encrypted,
    /// The file `path`, which came as `name`, of the kind `kind`.
    Read {
        kind: Kind,
        path: PathBuf,
        name: String,
    },
}

// ----------------------------------------------------------------------------
/// What reading `doc`, whose original is at `path` (if it has one), takes:
/// only an empty body is read, a file of a kind the reader knows -- by the
/// name it came with.
fn step_for(doc: Option<&Document>, path: Option<PathBuf>) -> Step {
    let Some(doc) = doc.filter(|doc| doc.content().trim().is_empty()) else {
        return Step::Keep;
    };
    let (Some(original), Some(path)) = (doc.original(), path) else {
        return Step::Unread;
    };
    if original.encrypted {
        return Step::Encrypted;
    }
    let name = original.name.clone().or_else(|| {
        let file = path.file_name()?;
        Some(file.to_string_lossy().into_owned())
    });
    let Some(name) = name else {
        return Step::Unread;
    };
    match Kind::of(&name) {
        Some(kind) => Step::Read { kind, path, name },
        None => Step::Unread,
    }
}

// ----------------------------------------------------------------------------
/// Read the attachment documents `ids` of `email`'s account with the model
/// `pick` as a job ("Reading attachments · <email>"), then analyse the mails
/// `then` asks for, and the attachments read. A refused pick is a notice,
/// and the analysis starts at once; so it does if there is nothing to read.
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
            return analyse(app, email, then, Vec::new());
        }
    };
    let queue: VecDeque<Id> = ids
        .into_iter()
        .filter(|id| app.vault.index().entry(id).is_some())
        .collect();
    if queue.is_empty() {
        return analyse(app, email, then, Vec::new());
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
        engine: Engine::Ai {
            client: resolved.client,
            provider: resolved.provider,
            address: resolved.address,
            model: resolved.model,
        },
        queue,
        current: None,
        total,
        counts: Counts::default(),
        written: Vec::new(),
        then,
    };
    app.attachments.reads.insert(job, read);
    pump(app, job);
}

// ----------------------------------------------------------------------------
/// The mails `then` asks for analysed, with the attachments `read`
/// (`enrich::analyse`).
fn analyse(app: &mut App, email: &str, then: Option<(Vec<Id>, Analysis)>, read: Vec<Id>) {
    if let Some((mails, analysis)) = then {
        enrich::analyse(app, Batch::mail(email, mails, read), &analysis);
    }
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
            Step::Encrypted => {
                read.queue.pop_front();
                read.counts.encrypted += 1;
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
        let engine = read.engine.clone();
        return app.run_job(
            job,
            move |cancel| {
                let data = std::fs::read(&path).map_err(|e| ReadError::File(e.into()))?;
                reader::read(&engine, cancel, &name, kind, data, |_| {})
            },
            move |app, result| on_read(app, job, id, kind, result),
        );
    }
}

// ----------------------------------------------------------------------------
/// The document `id` was read: write its text, or count the failure; then
/// the next. Ignored if the read is gone (stopped).
fn on_read(app: &mut App, job: Id, id: Id, kind: Kind, result: Result<String, ReadError>) {
    let Some(read) = app.attachments.reads.get_mut(&job) else {
        return;
    };
    read.current = None;
    if let Err(ReadError::File(e)) = &result {
        log::warn!("Attachment read {job}: could not read the file of {id}: {e}");
    }
    let encrypted = matches!(result, Err(ReadError::Encrypted));
    let text = result.map_err(|e| e.words(&read.engine));
    let written = match encrypted {
        true => mark_encrypted(app, id).map(|()| false),
        false => text.and_then(|text| write(app, id, text)),
    };

    let Some(read) = app.attachments.reads.get_mut(&job) else {
        return;
    };
    if written == Ok(true) {
        read.written.push(id);
    }
    let counts = &mut read.counts;
    match written {
        Ok(true) => counts.read += 1,
        Ok(false) if encrypted => {
            log::info!("Attachment read {job}: {id} is an encrypted PDF");
            counts.encrypted += 1;
        }
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
/// Mark the original of `id` as an encrypted PDF (`Original::encrypted`):
/// no model is sent it, and its body stays empty. A failed write in words.
fn mark_encrypted(app: &mut App, id: Id) -> Result<(), String> {
    let outcome = app.edit(|index, changes| {
        index.edit(&id, changes, |doc| {
            if let Some(mut original) = doc.original().cloned() {
                original.encrypted = true;
                doc.set_original(original);
            }
        })
    });
    match outcome.and_then(|outcome| app.vault.require_written(outcome)) {
        Ok(()) => Ok(()),
        Err(e) => {
            log::warn!("Attachment read: {id} could not be marked encrypted: {e}");
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
        analyse(app, &read.email, read.then, read.written);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enrich::AnalysisLevel;
    use crate::testing::{FakeOllama, Harness, TestPage, encrypted, parse, pdf_of};
    use serde_json::Value;
    use std::fs;
    use std::sync::{Arc, Mutex};

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
    /// and the attachments: a scan (an image), notes (text), a bill (a PDF
    /// with a text layer), an archive, and a document with text typed into
    /// it.
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
        let h = Harness::with_vault(name, &files);
        let lines = ["Stadtwerke Rechnung Nr. 4711", "Betrag: 42,00 EUR"];
        let bill = pdf_of(&[TestPage::Text(&lines)]);
        fs::write(h.vault().join("Attachments/bill.pdf"), bill).unwrap();
        h
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
    fn what_a_document_takes_goes_by_its_body_and_its_file() {
        let mut doc = Document::create(
            Default::default(),
            av_vault::envelope::Source::Created(Default::default()),
            "Bill".into(),
            Vec::new(),
            av_vault::document::FileFormat::Markdown,
        );
        let path = PathBuf::from("bill.pdf");
        assert_eq!(step_for(None, None), Step::Keep);
        assert_eq!(
            step_for(Some(&doc), Some(path.clone())),
            Step::Unread,
            "no original"
        );
        let original = |name: &str| av_vault::envelope::Original {
            file: Some("bill.pdf".into()),
            name: Some(name.into()),
            size: Some(4),
            encrypted: false,
        };
        doc.set_original(original("Rechnung.pdf"));
        assert_eq!(step_for(Some(&doc), None), Step::Unread, "no file");
        let read = Step::Read {
            kind: Kind::Pdf,
            path: path.clone(),
            name: "Rechnung.pdf".into(),
        };
        assert_eq!(step_for(Some(&doc), Some(path.clone())), read);
        let mut marked = original("Rechnung.pdf");
        marked.encrypted = true;
        doc.set_original(marked);
        assert_eq!(step_for(Some(&doc), Some(path.clone())), Step::Encrypted);
        doc.set_original(original("Rechnung.zip"));
        assert_eq!(step_for(Some(&doc), Some(path.clone())), Step::Unread);
        doc.set_original(original("Rechnung.txt"));
        let Step::Read { kind, .. } = step_for(Some(&doc), Some(path.clone())) else {
            panic!("a text file is read");
        };
        assert_eq!(kind, Kind::Plain);
        doc.set_content("Typed.\n".into());
        assert_eq!(step_for(Some(&doc), Some(path)), Step::Keep);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_read_writes_the_text_into_empty_bodies_then_analyses_the_mails_and_them() {
        let analysis = r#"{"kind": "notification", "title": "Abschlag Oktober"}"#;
        let document = r#"{"title": "Stadtwerke Rechnung", "date": "2020-03-01", "genre": "Bill"}"#;
        let address = FakeOllama::new(&["m"])
            .answering_in_turn(&["# Rechnung\n\n42,00 EUR", analysis, document])
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
            notes.contains(&&Value::from("3 read · 1 unread")),
            "{notes:?}"
        );
        assert!(
            notes.contains(&&Value::from("4 analysed")),
            "the mail, then the three attachments read: {notes:?}"
        );
        assert!(of(&sent, "notice").is_empty(), "{sent:?}");

        assert_eq!(body(&h, "scan.jpg.md"), "# Rechnung\n\n42,00 EUR\n");
        let scan = fs::read_to_string(h.vault().join("Attachments/scan.jpg.md")).unwrap();
        assert!(scan.contains("title: Stadtwerke Rechnung"), "{scan}");
        assert!(scan.contains("date: 2020-03-01T12:00:00Z"), "{scan}");
        assert!(scan.contains("genre: bill"), "{scan}");
        let typed = fs::read_to_string(h.vault().join("Attachments/typed.jpg.md")).unwrap();
        assert!(
            typed.contains("title: Typed.jpg"),
            "not read, not analysed: {typed}"
        );
        assert_eq!(body(&h, "notes.txt.md"), "Gr\u{fc}\u{df}e\n");
        let bill = body(&h, "bill.pdf.md");
        assert!(
            bill.starts_with("Stadtwerke Rechnung Nr. 4711")
                && bill.contains("\nBetrag: 42,00 EUR"),
            "from its text layer, without a request: {bill}"
        );
        assert_eq!(body(&h, "photos.zip.md"), "");
        assert_eq!(body(&h, "typed.jpg.md"), "Typed by hand.\n");
        let mail = fs::read_to_string(h.vault().join("Mail/a.md")).unwrap();
        assert!(mail.contains("title: Abschlag Oktober"), "{mail}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_pdfs_scans_are_read_page_by_page_and_its_text_pages_as_they_are() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let address = FakeOllama::new(&["m"])
            .answering_in_turn(&["Seite eins", "Seite zwei"])
            .recording(&requests)
            .start();
        let h = with_attachments("attachments-pdf-pages", &address);
        let lines = ["Zwischenblatt: die Anlagen zum Mietvertrag folgen."];
        let pages = [
            TestPage::Scan(b"\xff\xd8 page one"),
            TestPage::Text(&lines),
            TestPage::Scan(b"\xff\xd8 page two"),
        ];
        fs::write(h.vault().join("Attachments/bill.pdf"), pdf_of(&pages)).unwrap();
        let mut h = h;
        read(
            &mut h.app,
            "jan@example.com",
            ids(&[BILL]),
            Some(&pick()),
            None,
        );
        let sent = until_done(&mut h, 1);

        assert!(of(&sent, "notice").is_empty(), "{sent:?}");
        assert_eq!(
            body(&h, "bill.pdf.md"),
            "Seite eins\n\nZwischenblatt: die Anlagen zum Mietvertrag folgen.\n\nSeite zwei\n"
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for request in requests.iter() {
            let asked = &request["messages"][1];
            assert_eq!(asked["content"], "Transcribe this page.");
            assert_eq!(asked["images"].as_array().map(Vec::len), Some(1));
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_encrypted_pdf_is_marked_and_sent_to_no_model() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let address = FakeOllama::new(&["m"]).recording(&requests).start();
        let h = with_attachments("attachments-pdf-encrypted", &address);
        let lines = ["Kontoauszug Nr. 10 vom 30.10.2026 mit allen Buchungen"];
        let pdf = encrypted(pdf_of(&[TestPage::Text(&lines)]));
        fs::write(h.vault().join("Attachments/bill.pdf"), pdf).unwrap();
        let mut h = h;
        read(
            &mut h.app,
            "jan@example.com",
            ids(&[BILL]),
            Some(&pick()),
            None,
        );
        let sent = until_done(&mut h, 1);

        let notes: Vec<&Value> = of(&sent, "job").into_iter().map(|j| &j["note"]).collect();
        assert!(notes.contains(&&Value::from("1 encrypted")), "{notes:?}");
        assert!(of(&sent, "notice").is_empty(), "{sent:?}");
        assert!(requests.lock().unwrap().is_empty());
        let bill = fs::read_to_string(h.vault().join("Attachments/bill.pdf.md")).unwrap();
        assert!(bill.contains("    encrypted: true\n"), "{bill}");
        assert_eq!(body(&h, "bill.pdf.md"), "");
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
            encrypted: 1,
            kept: 2,
            ..Counts::default()
        };
        assert_eq!(
            counts.summary(),
            "3 read · 1 failed · 1 unread · 1 encrypted"
        );
        assert_eq!(counts.done(), 8);
        assert_eq!(Counts::default().summary(), "nothing to read");
    }
}
