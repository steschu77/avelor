//! For tests: an `App` without a window, on a temporary config folder.
//! What it sends to the page is collected, and the test runs its UI queue.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::app::{App, UiQueue, UiTask};
use crate::config::load_config;
use crate::sys::UiSignal;

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
    /// A vault's folder in the test's folder.
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
    /// Send `cmd` with `args` as the typed request 1; what the page got
    /// back, parsed.
    pub fn call(&mut self, cmd: &str, args: Value) -> Vec<Value> {
        let request = json!({ "id": 1, "cmd": cmd, "args": args });
        parse(&self.send(&request.to_string()))
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
