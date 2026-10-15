//! For tests: an `App` without a window. What it sends to the page is
//! collected, and the test runs its UI queue.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::app::{App, UiQueue, UiTask};
use crate::sys::UiSignal;

// ----------------------------------------------------------------------------
pub struct Harness {
    pub app: App,
    pub ui: Arc<UiQueue>,
    sent: Rc<RefCell<Vec<String>>>,
}

impl Harness {
    // ------------------------------------------------------------------------
    /// An `App` whose queue nothing wakes: the test runs it.
    pub fn new(_name: &str) -> Self {
        let ui = Arc::new(UiQueue::new(UiSignal::none()));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let outbox = {
            let sent = Rc::clone(&sent);
            Box::new(move |text: &str| sent.borrow_mut().push(text.to_string()))
        };
        let app = App::new(Arc::clone(&ui), outbox);
        Self { app, ui, sent }
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
            }
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

// ----------------------------------------------------------------------------
/// The messages the page got, parsed.
pub fn parse(sent: &[String]) -> Vec<Value> {
    sent.iter()
        .map(|text| serde_json::from_str(text).unwrap())
        .collect()
}
