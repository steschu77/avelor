//! `App`, the host's one coordinator (`architecture.md#principles`,
//! principle 7).
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::config::{ConfigStore, WindowState};
use crate::protocol::{self, Req};
use crate::sys::UiSignal;
use serde::Serialize;

// ----------------------------------------------------------------------------
pub type Prefs = BTreeMap<String, String>;

/// Where messages to the page go: the web view in the app, a list in
/// tests.
pub type Outbox = Box<dyn Fn(&str)>;

// ----------------------------------------------------------------------------
/// Everything that only makes sense once the web view is live: the channel
/// back to the page, and the host-side app state it drives.
pub struct App {
    outbox: Outbox,
    pub config: ConfigStore,
    ui: Arc<UiQueue>,
}

impl App {
    // ------------------------------------------------------------------------
    /// `ui` is the window's task queue: the shell runs what is queued on it
    /// (`Shell::run_tasks`). Every message to the page goes to `outbox`.
    pub fn new(ui: Arc<UiQueue>, outbox: Outbox, config: ConfigStore) -> Self {
        Self { outbox, config, ui }
    }

    // ------------------------------------------------------------------------
    /// A message arrived from the page's JS via `window.host.post`: a
    /// request of the protocol (`protocol`).
    pub fn on_web_message(&mut self, message: String) {
        protocol::on_message(self, &message);
    }

    // ------------------------------------------------------------------------
    /// Send `text` to the page as it is: a reply or an event.
    pub fn post_text(&self, text: &str) {
        (self.outbox)(text);
    }

    // ------------------------------------------------------------------------
    /// Push the event `ev` with `data` to the page: `{"ev": ev, "data": data}`
    /// (`api.md#events`).
    pub fn post_event(&self, ev: &str, data: impl Serialize) {
        #[derive(Serialize)]
        struct Event<'a, T> {
            ev: &'a str,
            data: T,
        }
        match serde_json::to_string(&Event { ev, data }) {
            Ok(text) => self.post_text(&text),
            Err(e) => log::warn!("Could not send the event {ev}: {e}"),
        }
    }

    // ------------------------------------------------------------------------
    /// Show `message` in the page's status bar as an error (the event
    /// `notice`, level error). `message` is the host's own words; the error
    /// behind it goes to the log.
    pub fn notice(&self, message: &str) {
        let notice = serde_json::json!({ "level": "error", "message": message });
        self.post_event("notice", notice);
    }

    // ------------------------------------------------------------------------
    /// Remember the main window's state in the app config, writing it only
    /// if it changed (a plain move ends a drag too). A failure is shown in
    /// the status bar (`notice`); the window works on.
    pub fn save_window_state(&mut self, state: WindowState) {
        if let Err(e) = self.config.set_window(state) {
            log::warn!("Could not save the window state: {e}");
            self.notice("Could not save the settings.");
        }
    }

    // ------------------------------------------------------------------------
    /// The answer token for the page's request `id`: it must be answered
    /// (`Req::ok`, `Req::err`), or it answers `cancelled` when dropped.
    pub fn req(&self, id: u64) -> Req {
        Req::new(id, Arc::clone(&self.ui))
    }
}

// ----------------------------------------------------------------------------
/// Work queued for the UI thread, run in order by `Shell::run_tasks`.
pub enum UiTask {
    /// Runs with `App` (`UiQueue::send_to_ui`, the window's own
    /// messages).
    App(Box<dyn FnOnce(&mut App) + Send>),
}

// ----------------------------------------------------------------------------
pub struct UiQueue {
    signal: UiSignal,
    tasks: Mutex<VecDeque<UiTask>>,
}

// ----------------------------------------------------------------------------
impl UiQueue {
    // ------------------------------------------------------------------------
    pub fn new(signal: UiSignal) -> Self {
        Self {
            signal,
            tasks: Mutex::new(VecDeque::new()),
        }
    }

    // ------------------------------------------------------------------------
    /// Queue `apply` to run on the UI thread and wake the queue.
    pub fn send_to_ui(&self, apply: impl FnOnce(&mut App) + Send + 'static) {
        self.send(UiTask::App(Box::new(apply)));
    }

    // ------------------------------------------------------------------------
    /// Queue `task` and wake the queue.
    fn send(&self, task: UiTask) {
        self.push(task);
        self.wake();
    }

    // ------------------------------------------------------------------------
    /// Have the UI thread run the queue (`UiSignal::wake`). A queue without a
    /// window (in tests) only queues; the test runs the queue itself.
    pub fn wake(&self) {
        self.signal.wake();
    }

    // ------------------------------------------------------------------------
    /// Queue `task` without waking the queue: for the window's own
    /// messages, which run the queue right after.
    pub fn push(&self, task: UiTask) {
        self.tasks.lock().unwrap().push_back(task);
    }

    // ------------------------------------------------------------------------
    /// Take the oldest queued task.
    pub fn pop(&self) -> Option<UiTask> {
        self.tasks.lock().unwrap().pop_front()
    }

    // ------------------------------------------------------------------------
    /// How many tasks are queued.
    pub fn queued(&self) -> usize {
        self.tasks.lock().unwrap().len()
    }
}
