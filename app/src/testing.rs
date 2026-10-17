//! For tests: an `App` without a window, on a temporary config folder and,
//! if asked, a temporary vault. What it sends to the page is collected, and
//! the test runs its UI queue.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
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
        let app = App::new(Arc::clone(&ui), outbox, config);
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
/// A fake Ollama on this computer, for the AI commands: `GET /api/tags`
/// lists its models.
pub struct FakeOllama {
    models: Vec<String>,
}

impl FakeOllama {
    // ------------------------------------------------------------------------
    pub fn new(models: &[&str]) -> Self {
        Self {
            models: models.iter().map(|m| m.to_string()).collect(),
        }
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
    /// Read the request, then answer it.
    fn answer(&self, mut stream: TcpStream) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            if !reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                break;
            }
        }
        let models: Vec<Value> = self
            .models
            .iter()
            .map(|name| json!({ "name": name, "details": {} }))
            .collect();
        write_json(&mut stream, &json!({ "models": models }))
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
