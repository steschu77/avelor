//! For tests: an `App` without a window, on a temporary config folder and,
//! if asked, a temporary vault. What it sends to the page is collected, and
//! the test runs its UI queue.

use std::cell::RefCell;
use std::io::Read;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::api;
use crate::app::{App, UiQueue, UiTask};
use crate::config::load_config;
use crate::sys::UiSignal;
use av_utils::id::Id;

// ----------------------------------------------------------------------------
pub struct Harness {
    pub app: App,
    pub ui: Arc<UiQueue>,
    sent: Rc<RefCell<Vec<String>>>,
    dir: PathBuf,
}

impl Harness {
    // ------------------------------------------------------------------------
    /// An `App` on an empty config folder, named after the test, whose
    /// queue nothing wakes: the test runs it.
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("avelor-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = load_config(&dir).unwrap();
        let ui = Arc::new(UiQueue::new(UiSignal::none()));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let outbox = {
            let sent = Rc::clone(&sent);
            Box::new(move |text: &str| sent.borrow_mut().push(text.to_string()))
        };
        let mut app = App::new(Arc::clone(&ui), outbox, config);
        // The tests run as a build with Google, whatever its features.
        app.hides_google = false;
        Self { app, ui, sent, dir }
    }

    // ------------------------------------------------------------------------
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    // ------------------------------------------------------------------------
    /// An `App` with a vault of `files` (vault-relative path, text), opened
    /// and scanned as the app does it (`api::switch_to`); what the scan sent is
    /// dropped.
    pub fn with_vault(name: &str, files: &[(impl AsRef<Path>, impl AsRef<str>)]) -> Self {
        let mut harness = Self::new(name);
        let root = harness.vault();
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text.as_ref()).unwrap();
        }
        api::switch_to(&mut harness.app, root.clone());
        harness.wait_scanned();
        harness.take();
        harness
    }

    // ------------------------------------------------------------------------
    /// The vault's folder (`with_vault`).
    pub fn vault(&self) -> PathBuf {
        self.dir.join("vault")
    }

    // ------------------------------------------------------------------------
    /// Send `text` from the page, run the queue, and return what the page
    /// got back.
    pub fn send(&mut self, text: &str) -> Vec<String> {
        self.app.on_web_message(text.to_string());
        self.run_queue();
        self.take()
    }

    // ------------------------------------------------------------------------
    /// Run the queued tasks, and those they queue, until none is left.
    pub fn run_queue(&mut self) {
        while let Some(task) = self.ui.pop() {
            match task {
                UiTask::App(task) => task(&mut self.app),
                UiTask::Modal(_) => panic!("a dialog in a test"),
            }
        }
    }

    // ------------------------------------------------------------------------
    /// Run the queue until the vault scan has ended -- it reads on a worker
    /// (`vault_scan`); at most 5 seconds.
    pub fn wait_scanned(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.run_queue();
            if !self.app.scanning() {
                return;
            }
            assert!(Instant::now() < deadline, "the vault scan didn't end");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    // ------------------------------------------------------------------------
    /// Run the queue until what the page got satisfies `done` -- workers post
    /// from their own threads -- and return it; at most 5 seconds.
    pub fn wait_for(&mut self, done: impl Fn(&[String]) -> bool) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut sent = Vec::new();
        loop {
            self.run_queue();
            sent.extend(self.take());
            if done(&sent) {
                return sent;
            }
            assert!(Instant::now() < deadline, "timed out, got {sent:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    // ------------------------------------------------------------------------
    /// Send `cmd` with `args` as the typed request 1; what the page got
    /// back, parsed.
    pub fn call(&mut self, cmd: &str, args: Value) -> Vec<Value> {
        let request = json!({ "id": 1, "cmd": cmd, "args": args });
        parse(&self.send(&request.to_string()))
    }

    // ------------------------------------------------------------------------
    /// `call`, for a command answered from a worker: run the queue until the
    /// reply came (`wait_for`).
    pub fn call_waiting(&mut self, cmd: &str, args: Value) -> Vec<Value> {
        let request = json!({ "id": 1, "cmd": cmd, "args": args });
        self.app.on_web_message(request.to_string());
        let is_reply =
            |m: &String| serde_json::from_str::<Value>(m).is_ok_and(|m| m.get("re").is_some());
        parse(&self.wait_for(|sent| sent.iter().any(is_reply)))
    }

    // ------------------------------------------------------------------------
    /// What the page got since the last call.
    pub fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.sent.borrow_mut())
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ----------------------------------------------------------------------------
/// The messages the page got, parsed.
pub fn parse(sent: &[String]) -> Vec<Value> {
    sent.iter()
        .map(|text| serde_json::from_str(text).unwrap())
        .collect()
}

// ----------------------------------------------------------------------------
/// Each message as its event's name, `reply` for an answer.
pub fn shape(sent: &[Value]) -> Vec<&str> {
    sent.iter()
        .map(|m| m["ev"].as_str().unwrap_or("reply"))
        .collect()
}

// ----------------------------------------------------------------------------
/// Whether `path` is in the vault's `.trash`.
pub fn trashed(root: &Path, path: &str) -> bool {
    root.join(av_vault::schema::layout::TRASH)
        .join(path)
        .exists()
}

// ----------------------------------------------------------------------------
/// A vault of `n` documents, `Notes/n<i>.md`, titled `Note <i>`, with the
/// body `Body <i>`.
pub fn notes(name: &str, n: usize) -> (Harness, Vec<Id>) {
    let ids: Vec<Id> = (0..n).map(|_| Id::generate()).collect();
    let files: Vec<(String, String)> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let text = format!("---\nid: {id}\ntitle: Note {i}\ndocument: {{}}\n---\nBody {i}\n");
            (format!("Notes/n{i}.md"), text)
        })
        .collect();
    (Harness::with_vault(name, &files), ids)
}

// ----------------------------------------------------------------------------
/// The Google accounts of `with_google`: one connected (with a token), one
/// disconnected.
pub const CONNECTED: &str = "01a0e1cc-a24c-7490-82fe-e75669985b24";
pub const DISCONNECTED: &str = "01a0e1cc-a24c-7490-82fe-e75669985b25";

// ----------------------------------------------------------------------------
/// A harness whose vault has the Google accounts `CONNECTED` (with a token)
/// and `DISCONNECTED`, both with the lines `google` in their `google:` block
/// (scopes, calendars, …), plus `files`; opened, and what that sent dropped.
pub fn with_google(name: &str, google: &str, files: &[(&str, String)]) -> Harness {
    let mut h = Harness::new(name);
    let root = h.vault();
    let account = |id: &str, extra: &str| {
        format!("id: {id}\ngoogle:\n  email: {id}@example.com\n{google}{extra}")
    };
    let mut all = vec![
        (".avelor/accounts/google_a.yaml", account(CONNECTED, "")),
        (
            ".avelor/accounts/google_b.yaml",
            account(DISCONNECTED, "  disconnected: 2026-10-01T08:00:00Z\n"),
        ),
    ];
    all.extend(files.iter().cloned());
    for (path, text) in all {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let token = h.app.config.dir().join(format!("{CONNECTED}.token"));
    std::fs::write(token, av_utils::secret::protect(b"1//token").unwrap()).unwrap();
    api::switch_to(&mut h.app, root.clone());
    h.wait_scanned();
    h.take();
    h
}

// ----------------------------------------------------------------------------
/// A fake Ollama on this computer, for the AI commands: `GET /api/tags`
/// lists its models; `POST /api/chat` streams its reply as Ollama does (one
/// JSON object per line, chunked), or, holding, sends the first piece and
/// keeps the answer open until the client goes away (a reply to stop).
#[derive(Default)]
pub struct FakeOllama {
    models: Vec<String>,
    reply: Vec<String>,
    hold: bool,
    /// Where the chat requests go, as JSON, if they are kept.
    requests: Option<Arc<Mutex<Vec<Value>>>>,
}

impl FakeOllama {
    // ------------------------------------------------------------------------
    pub fn new(models: &[&str]) -> Self {
        Self {
            models: models.iter().map(|m| m.to_string()).collect(),
            ..Self::default()
        }
    }

    // ------------------------------------------------------------------------
    /// Answer chats with `pieces`, streamed one after the other.
    pub fn replying(mut self, pieces: &[&str]) -> Self {
        self.reply = pieces.iter().map(|p| p.to_string()).collect();
        self
    }

    // ------------------------------------------------------------------------
    /// Keep the chat requests in `requests`.
    pub fn recording(mut self, requests: &Arc<Mutex<Vec<Value>>>) -> Self {
        self.requests = Some(Arc::clone(requests));
        self
    }

    // ------------------------------------------------------------------------
    /// Stop after the first piece of a reply, until the client goes away.
    pub fn holding(mut self) -> Self {
        self.hold = true;
        self
    }

    // ------------------------------------------------------------------------
    /// Listen on a free port, answering each connection on a thread of its
    /// own; its address.
    pub fn start(self) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let fake = Arc::new(self);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let fake = Arc::clone(&fake);
                std::thread::spawn(move || fake.answer(stream));
            }
        });
        address
    }

    // ------------------------------------------------------------------------
    /// Read the request, then answer it by its path.
    fn answer(&self, mut stream: TcpStream) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        let _ = reader.read_line(&mut line);
        let path = line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string();
        let mut length = 0;
        loop {
            line.clear();
            if !reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                break;
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
        }
        let mut body = vec![0; length];
        let _ = reader.read_exact(&mut body);
        if let (Some(requests), "/api/chat") = (&self.requests, path.as_str()) {
            let request = serde_json::from_slice(&body).unwrap_or_default();
            requests.lock().unwrap().push(request);
        }

        if path != "/api/chat" {
            let models: Vec<Value> = self
                .models
                .iter()
                .map(|name| json!({ "name": name, "details": {} }))
                .collect();
            return write_json(&mut stream, &json!({ "models": models }));
        }

        let head = "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
        let _ = stream.write_all(head.as_bytes());
        let send = |stream: &mut TcpStream, chunk: Value| {
            let line = format!("{chunk}\n");
            let _ = write!(stream, "{:x}\r\n{line}\r\n", line.len());
            let _ = stream.flush();
        };
        for (i, piece) in self.reply.iter().enumerate() {
            let message = json!({ "role": "assistant", "content": piece });
            send(
                &mut stream,
                json!({ "model": "m", "message": message, "done": false }),
            );
            if self.hold && i == 0 {
                // Until the client gives up on the answer.
                let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                let _ = reader.read(&mut [0; 1]);
                return;
            }
        }
        let done = json!({
            "model": "m",
            "message": { "role": "assistant", "content": "" },
            "done": true,
            "done_reason": "stop",
            "prompt_eval_count": 5,
            "eval_count": 3,
        });
        send(&mut stream, done);
        let _ = stream.write_all(b"0\r\n\r\n");
    }
}

// ----------------------------------------------------------------------------
/// Answer with `body` as one JSON document, and close.
fn write_json(stream: &mut TcpStream, body: &Value) {
    let body = body.to_string();
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}
