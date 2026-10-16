//! `App`, the host's one coordinator (`architecture.md#principles`,
//! principle 7).
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::close::CloseState;
use crate::config::{ConfigStore, WindowState};
use crate::jobs::Jobs;
use crate::protocol::{self, Req};
use crate::sys::Owner;
use crate::sys::UiSignal;
use crate::vault_scan::VaultScan;
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::Result;
use av_utils::id::Id;
use av_vault::envelope::{DocumentType, Roles, Source};
use av_vault::shared::Genre;
use av_vault::vault::{Changes, FileStamp};
use av_vault::vault::{Entry, Index, ScanEvent};
use serde::Serialize;

// ----------------------------------------------------------------------------
pub type Prefs = BTreeMap<String, String>;

/// Where messages to the page go: the web view in the app, a list in
/// tests.
pub type Outbox = Box<dyn Fn(&str)>;

// ----------------------------------------------------------------------------
/// An entity as the page sees it (the `entity` event): one shape for every
/// kind -- a project, a thread, every other entity as a table row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntityMeta {
    pub id: Id,
    pub title: String,
    pub path: String,
    pub kind: DocumentType,
    pub source: Option<Source>,
    pub genre: Option<Genre>,
    pub date: Option<DateTime>,
    pub projects: Vec<Id>,
    pub tags: Vec<String>,
    /// The entity this one is linked to (`linked-to`).
    pub linked_to: Option<Id>,
    pub starred: bool,
    pub archived: bool,
    /// The threads this entity is a member of (`message.threads`).
    pub threads: Vec<Id>,
    pub editable: bool,
    /// Whether the host may rewrite its frontmatter (`Entry::is_writable`):
    /// labels, title, membership.
    pub writable: bool,
    /// Whether the delete button may move the file to `.trash`.
    pub deletable: bool,
    #[serde(flatten)]
    pub roles: Roles,
}

// ----------------------------------------------------------------------------
/// The open vault: its folder and av-vault's `Index`, private so nothing
/// else can change the index unseen. The scan has its method here.
#[derive(Default)]
pub struct Vault {
    root: Option<PathBuf>,
    index: Index,
}

// ----------------------------------------------------------------------------
impl Vault {
    // ------------------------------------------------------------------------
    /// Make `root` the open vault and reset the index ready to receive scan
    /// events (`App::run_scan`).
    pub fn begin_open(&mut self, root: &Path) {
        self.index.begin(root);
        self.root = Some(root.to_path_buf());
    }

    // ------------------------------------------------------------------------
    /// Put one event of the vault's scan into the index (`Index::apply`):
    /// the id of the entry it added.
    pub fn apply_scanned(&mut self, event: ScanEvent) -> Option<Id> {
        self.index.apply(event)
    }

    // ------------------------------------------------------------------------
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    // ------------------------------------------------------------------------
    /// The index, for reading; edits go through `App::edit`.
    pub fn index(&self) -> &Index {
        &self.index
    }

    // ------------------------------------------------------------------------
    /// Read `id`'s file again (`Index::reload`): the id it has now, `None`
    /// if it is gone.
    pub fn reload(&mut self, id: &Id) -> Result<Option<Id>> {
        self.index.reload(id)
    }

    // ------------------------------------------------------------------------
    /// When `id`'s file was last modified, as it is now: for a conflict, the
    /// time of the change made outside the app. `None` if the file is gone.
    pub fn changed_at(&self, id: &Id) -> Option<DateTime> {
        let stamp = FileStamp::read(&self.index.entry(id)?.path).ok()??;
        let since = stamp
            .modified()
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?;
        Some(DateTime::from_unix_millis(since.as_millis() as i64))
    }
}

// ----------------------------------------------------------------------------
/// Everything that only makes sense once the web view is live: the channel
/// back to the page, and the host-side app state it drives.
pub struct App {
    outbox: Outbox,
    pub config: ConfigStore,
    /// The open vault: read through `Vault::index`, edited through
    /// `App::edit`.
    pub vault: Vault,
    ui: Arc<UiQueue>,

    /// The vault scan running (`vault_scan`); a new one replaces it.
    pub scan: VaultScan,

    /// The long operations running (`jobs`).
    pub jobs: Jobs,

    /// The close asked of the page (`close`).
    pub close: CloseState,
}

impl App {
    // ------------------------------------------------------------------------
    /// `ui` is the window's task queue: the shell runs what is queued on it
    /// (`Shell::run_tasks`). Every message to the page goes to `outbox`.
    pub fn new(ui: Arc<UiQueue>, outbox: Outbox, config: ConfigStore) -> Self {
        Self {
            outbox,
            config,
            vault: Vault::default(),
            ui,
            scan: VaultScan::default(),
            jobs: Jobs::default(),
            close: CloseState::default(),
        }
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
    /// The answer token for the page's request `id`: it must be answered
    /// (`Req::ok`, `Req::err`), or it answers `cancelled` when dropped.
    pub fn req(&self, id: u64) -> Req {
        Req::new(id, Arc::clone(&self.ui))
    }

    // ------------------------------------------------------------------------
    /// Run blocking `work` (requests, filesystem, hashing, …) on a worker
    /// thread, then apply its output on the UI thread. Aborting the returned
    /// task cancels `work` through its `Cancel` and skips `apply`.
    pub fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Cancel) -> T + Send + 'static,
        apply: impl FnOnce(&mut App, T) + Send + 'static,
    ) -> Task {
        let cancel = Cancel::new();
        self.spawn(cancel.clone(), work, apply);
        Task(cancel)
    }

    // ------------------------------------------------------------------------
    /// Run `work` on a new worker thread under `cancel`, then `apply` its
    /// output on the UI thread -- unless `cancel` was cancelled meanwhile.
    fn spawn<T: Send + 'static>(
        &self,
        cancel: Cancel,
        work: impl FnOnce(&Cancel) -> T + Send + 'static,
        apply: impl FnOnce(&mut App, T) + Send + 'static,
    ) {
        let ui = Arc::clone(&self.ui);
        let spawned = std::thread::Builder::new()
            .name("avelor-worker".to_string())
            .spawn(move || {
                let out = work(&cancel);
                if !cancel.is_cancelled() {
                    ui.send_to_ui(move |app| apply(app, out));
                }
            });
        if let Err(e) = spawned {
            log::error!("Could not start a worker thread: {e}");
        }
    }

    // ------------------------------------------------------------------------
    /// Run `work` like `run`, handing it a `Progress` to report along the
    /// way: each `Progress::send` queues `on_progress` on the UI thread, in
    /// order, and `apply` runs after the last of them. Aborting the returned
    /// task cancels `work` and skips `apply`; progress queued before still
    /// arrives, so `on_progress` must check it is still wanted.
    pub fn run_with_progress<P, T>(
        &self,
        work: impl FnOnce(Progress<P>, &Cancel) -> T + Send + 'static,
        on_progress: impl FnMut(&mut App, P) + Send + 'static,
        apply: impl FnOnce(&mut App, T) + Send + 'static,
    ) -> Task
    where
        P: Send + 'static,
        T: Send + 'static,
    {
        let progress = self.progress(on_progress);
        self.run(move |cancel| work(progress, cancel), apply)
    }

    // ------------------------------------------------------------------------
    /// The `Progress` that queues `on_progress` on the UI thread.
    fn progress<P: Send + 'static>(
        &self,
        on_progress: impl FnMut(&mut App, P) + Send + 'static,
    ) -> Progress<P> {
        Progress {
            ui: Arc::clone(&self.ui),
            on_progress: Arc::new(Mutex::new(Box::new(on_progress))),
        }
    }

    // ------------------------------------------------------------------------
    /// The UI queue, for what must run after the current task.
    pub(crate) fn ui(&self) -> &UiQueue {
        &self.ui
    }

    // ------------------------------------------------------------------------
    /// Run `work` on the UI thread once the current handler has returned and
    /// released `App`, with the main window as the dialog's owner;
    /// then `apply` its result with `App`. A modal dialog runs its own
    /// message loop: shown from inside a handler, it would dispatch messages
    /// into an `App` still in use. Messages that arrive while the dialog is
    /// open run normally; the window can't be closed until it ends.
    pub fn on_ui<T: 'static>(
        &self,
        work: impl FnOnce(&Owner) -> T + Send + 'static,
        apply: impl FnOnce(&mut App, T) + Send + 'static,
    ) {
        self.ui.send(UiTask::Modal(Box::new(move |owner| {
            let out = work(owner);
            Box::new(move |app: &mut App| apply(app, out))
        })));
    }

    // ------------------------------------------------------------------------
    /// Remember the main window's state in the app config, writing it only
    /// if it changed (a plain move ends a drag too).
    pub fn save_window_state(&mut self, state: WindowState) {
        let saved = self.config.set_window(state);
        self.config_saved(saved);
    }

    // ------------------------------------------------------------------------
    /// Remember `root` in the app config as the vault to open at the next
    /// start (`api::switch_to`).
    pub fn remember_vault(&mut self, root: PathBuf) {
        let saved = self.config.set_vault(root);
        self.config_saved(saved);
    }

    // ------------------------------------------------------------------------
    /// How a write of the config for a change no request asked for went (the
    /// window's state, the vault remembered on a switch): a failure is shown
    /// in the status bar (`notice`), the error logged.
    fn config_saved(&self, saved: Result<()>) {
        if let Err(e) = saved {
            log::warn!("Could not save the settings: {e}");
            self.notice("Could not save the settings.");
        }
    }

    // ------------------------------------------------------------------------
    /// Send one entity to the page: the `entity` event, for scanned entries
    /// and for entities written in the app alike.
    pub fn post_entity(&self, entry: &Entry) {
        let doc = &entry.doc;
        let meta = EntityMeta {
            id: doc.id(),
            title: doc.title().to_string(),
            path: entry.local.display().to_string(),
            kind: doc.kind(),
            source: doc.source().cloned(),
            genre: doc.roles().document.as_ref().and_then(|d| d.genre.clone()),
            date: doc.date(),
            projects: doc.projects().to_vec(),
            tags: doc.tags().to_vec(),
            linked_to: doc.linked_to(),
            starred: doc.starred(),
            archived: doc.archived(),
            threads: doc
                .roles()
                .message
                .as_ref()
                .map(|m| m.threads.clone())
                .unwrap_or_default(),
            editable: entry.is_editable(),
            writable: entry.is_writable(),
            deletable: entry.is_deletable(),
            roles: doc.roles().clone(),
        };
        self.post_event("entity", meta);
    }

    // ------------------------------------------------------------------------
    /// Tell the page that `id` left the vault (its file gone): the
    /// `entity_deleted` event.
    pub fn post_entity_deleted(&self, id: &Id) {
        self.post_event("entity_deleted", serde_json::json!({ "id": id }));
    }

    // ------------------------------------------------------------------------
    /// Run the vault edit `f` and send the page everything it wrote -- also
    /// when it failed part-way, since a multi-file edit stays part-way
    /// (`panel.md#saving-structure`). The one way the app edits the vault:
    /// taking in a change made outside the app (`Vault::reload`) is not an
    /// edit by the app.
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut Index, &mut Changes) -> Result<R>) -> Result<R> {
        let mut changes = Changes::default();
        let result = f(&mut self.vault.index, &mut changes);
        self.post_changes(&changes);
        result
    }

    // ------------------------------------------------------------------------
    /// Send what an edit wrote: every rewritten entity (`entity`).
    fn post_changes(&self, changes: &Changes) {
        for id in &changes.saved {
            if let Some(entry) = self.vault.index().entry(id) {
                self.post_entity(entry);
            }
        }
    }
}

// ----------------------------------------------------------------------------
/// Work started by `App::run` and its variants. Dropping it leaves the
/// work running.
pub struct Task(Cancel);

impl Task {
    // ------------------------------------------------------------------------
    /// Cancel the work and skip its `apply`. Too late once `apply` is
    /// queued, so continuations check they are still wanted.
    pub fn abort(&self) {
        self.0.cancel();
    }
}

// ----------------------------------------------------------------------------
/// Work queued for the UI thread, run in order by `Shell::run_tasks`.
pub enum UiTask {
    /// Runs with `App` (`UiQueue::send_to_ui`, the window's own
    /// messages).
    App(Box<dyn FnOnce(&mut App) + Send>),
    /// Runs without `App` -- a modal dialog, owned by the main window
    /// -- then returns what to apply with it (`App::on_ui`).
    Modal(Box<dyn FnOnce(&Owner) -> Apply + Send>),
}

/// What a `UiTask::Modal` applies once its dialog has closed.
pub type Apply = Box<dyn FnOnce(&mut App)>;
type OnProgress<P> = Box<dyn FnMut(&mut App, P) + Send>;

// ----------------------------------------------------------------------------
/// What `App::run_with_progress` hands its work: `send` queues the run's
/// `on_progress` on the UI thread. It never reaches the webview itself; only
/// the queued continuation runs, with the `App`.
pub struct Progress<P> {
    ui: Arc<UiQueue>,
    on_progress: Arc<Mutex<OnProgress<P>>>,
}

impl<P> Clone for Progress<P> {
    fn clone(&self) -> Self {
        Self {
            ui: Arc::clone(&self.ui),
            on_progress: Arc::clone(&self.on_progress),
        }
    }
}

impl<P: Send + 'static> Progress<P> {
    // ------------------------------------------------------------------------
    /// Queue `on_progress(app, item)` on the UI thread: every item
    /// arrives.
    pub fn send(&self, item: P) {
        let on_progress = Arc::clone(&self.on_progress);
        self.ui.send_to_ui(move |app| {
            let mut on_progress = on_progress.lock().unwrap();
            on_progress(app, item);
        });
    }
}

// ----------------------------------------------------------------------------
pub struct UiQueue {
    signal: UiSignal,
    tasks: Mutex<VecDeque<UiTask>>,
    /// Whether `close_window` was called; what a test can look at.
    closed: AtomicBool,
}

// ----------------------------------------------------------------------------
impl UiQueue {
    // ------------------------------------------------------------------------
    pub fn new(signal: UiSignal) -> Self {
        Self {
            signal,
            tasks: Mutex::new(VecDeque::new()),
            closed: AtomicBool::new(false),
        }
    }

    // ------------------------------------------------------------------------
    /// Have the window close (`UiSignal::close`): how `App` ends the program
    /// once the page is ready (`close`). A queue without a window (in
    /// tests) only remembers it.
    pub fn close_window(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.signal.close();
    }

    // ------------------------------------------------------------------------
    /// Whether the window was asked to close (`close_window`).
    #[cfg(test)]
    pub fn window_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
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
