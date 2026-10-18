//! `App`, the host's one coordinator (`architecture.md#principles`,
//! principle 7).
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use crate::ai_accounts::AiAccounts;
use crate::calendar;
use crate::calendar::CalendarState;
use crate::chat::ChatState;
use crate::close::CloseState;
use crate::config::{ConfigStore, WindowState};
use crate::documents::DocumentState;
use crate::google_accounts::GoogleAccounts;
use crate::jobs::Jobs;
use crate::mail;
use crate::mail::MailState;
use crate::protocol::{self, Req};
use crate::sys::Owner;
use crate::sys::UiSignal;
use crate::vault_scan::VaultScan;
use crate::vault_settings::VaultSettings;
use av_utils::cancel::Cancel;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::id::Id;
use av_vault::envelope::{DocumentType, Roles, Source};
use av_vault::markdown;
use av_vault::shared::Genre;
use av_vault::vault::Refresh;
use av_vault::vault::SaveOutcome;
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
/// An account connect between opening the browser and saving the account.
struct PendingConnect {
    /// Names the connect in its continuations (`google::connect_google`).
    id: Id,
    /// Ends the wait for the browser redirect early, and the requests that
    /// redeem the code.
    cancel: Cancel,
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
    /// A document's content by its ID, as `(markdown source, editor HTML)`.
    /// Both are empty for an unknown ID; the HTML is empty for YAML records,
    /// which have no markdown body.
    pub fn content(&self, id: &Id) -> (String, String) {
        match self.index.get(id) {
            Some(doc) => (doc.content().to_string(), markdown::to_html(doc.content())),
            None => (String::new(), String::new()),
        }
    }

    // ------------------------------------------------------------------------
    /// Save `markdown` (serialized from the editor's DOM by the webview,
    /// `app/static/editor/markdown.js`) as `id`'s new content: write it to
    /// disk if that changed anything (`saving.md#messages`,
    /// `markdown.md#round-trip`). A file changed outside the app is a
    /// conflict and isn't written, unless `overwrite`.
    pub fn save_content(
        &mut self,
        id: &Id,
        markdown: &str,
        overwrite: bool,
    ) -> Result<SaveOutcome> {
        self.index.save_content(id, markdown.to_string(), overwrite)
    }

    // ------------------------------------------------------------------------
    /// Read `id`'s file again (`Index::reload`): the id it has now, `None`
    /// if it is gone.
    pub fn reload(&mut self, id: &Id) -> Result<Option<Id>> {
        self.index.reload(id)
    }

    // ------------------------------------------------------------------------
    /// Take in a change made outside the app to `id`'s file, unless it
    /// replaced the app's own edits (`Index::refresh`).
    pub fn refresh(&mut self, id: &Id) -> Result<Refresh> {
        self.index.refresh(id)
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

    // ------------------------------------------------------------------------
    /// For writers that can't let the user choose (a sync, the assistant):
    /// a conflict as a failure, `Error::ChangedOutside`.
    pub fn require_written(&self, outcome: SaveOutcome) -> Result<()> {
        match outcome {
            SaveOutcome::Conflict(id) => Err(Error::ChangedOutside {
                path: self
                    .index
                    .entry(&id)
                    .map(|e| e.local.clone())
                    .unwrap_or_default(),
            }),
            SaveOutcome::Saved | SaveOutcome::Unchanged => Ok(()),
        }
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

    /// The open vault's Google accounts (`accounts::open`).
    pub google: GoogleAccounts,

    /// Whether this build hides Google (the feature `hide-google`): no
    /// Google account is loaded, and the page offers none to connect.
    pub hides_google: bool,

    /// The open vault's AI accounts (`accounts::open`).
    pub ai: AiAccounts,

    /// The open vault's own settings (`vault_settings::open`).
    pub settings: VaultSettings,

    /// The account connect waiting for the browser sign-in. Starting
    /// another replaces (and so cancels) it, like `scan`; so does opening a
    /// vault.
    connect: Option<PendingConnect>,

    /// The vault scan running (`vault_scan`); a new one replaces it.
    pub scan: VaultScan,

    /// Per channel, the latest request of `run_superseding` and its task, to
    /// abort when a newer one arrives.
    superseding: HashMap<&'static str, (String, Task)>,

    /// The long operations running (`jobs`).
    pub jobs: Jobs,

    /// The documents in the page's editors (`documents`): those in edit
    /// mode, and the app's version of each one in conflict.
    pub documents: DocumentState,

    /// Fetch mail (`mail`): the fetch in progress; one at a time.
    pub mail: MailState,

    /// The calendar sync (`calendar`): the sync in progress, one at a
    /// time, and the auto-syncs' schedule.
    pub calendar: CalendarState,

    /// The close asked of the page (`close`).
    pub close: CloseState,

    /// The assistant's replies being written, and the failed ones
    /// (`chat`); neither is in the vault.
    pub chat: ChatState,
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
            google: GoogleAccounts::default(),
            hides_google: !av_google::auth::has_client(),
            ai: AiAccounts::default(),
            settings: VaultSettings::default(),
            connect: None,
            scan: VaultScan::default(),
            superseding: HashMap::new(),
            jobs: Jobs::default(),
            documents: DocumentState::default(),
            mail: MailState::default(),
            calendar: CalendarState::default(),
            close: CloseState::default(),
            chat: ChatState::default(),
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
    /// Tell the page `message` in its status bar (the event `notice`, level
    /// info), and log it: what a job brought, e.g. new mail; `entity` is the
    /// one a click on it shows.
    pub fn inform(&self, message: &str, entity: Option<Id>) {
        log::info!("{message}");
        let notice = serde_json::json!({ "level": "info", "message": message, "entity": entity });
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
        self.spawn(cancel.clone(), false, work, apply);
        Task(cancel)
    }

    // ------------------------------------------------------------------------
    /// Run `work` on a new worker thread under `cancel`, then `apply` its
    /// output on the UI thread -- unless `cancel` was cancelled meanwhile
    /// and not `apply_if_cancelled` (`run`; a job's continuation always runs,
    /// `App::run_job`).
    pub(crate) fn spawn<T: Send + 'static>(
        &self,
        cancel: Cancel,
        apply_if_cancelled: bool,
        work: impl FnOnce(&Cancel) -> T + Send + 'static,
        apply: impl FnOnce(&mut App, T) + Send + 'static,
    ) {
        let ui = Arc::clone(&self.ui);
        let spawned = std::thread::Builder::new()
            .name("avelor-worker".to_string())
            .spawn(move || {
                let out = work(&cancel);
                if apply_if_cancelled || !cancel.is_cancelled() {
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
    /// order (`send_latest` replaces what hasn't arrived yet), and `apply`
    /// runs after the last of them. Aborting the returned task cancels
    /// `work` and skips `apply`; progress queued before still arrives, so
    /// `on_progress` must check it is still wanted.
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
            latest: Arc::new(Mutex::new(None)),
        }
    }

    // ------------------------------------------------------------------------
    /// The UI queue, for what must run after the current task.
    pub(crate) fn ui(&self) -> &UiQueue {
        &self.ui
    }

    // ------------------------------------------------------------------------
    /// Run `work` like `run`, as the request `req_id` on `channel`, where
    /// only the latest request counts (a preview while the user types): it
    /// aborts the channel's previous work, and `apply` only runs while
    /// `req_id` is still the channel's latest -- so a superseded reply is
    /// never applied, whatever order the network answers in.
    pub fn run_superseding<T: Send + 'static>(
        &mut self,
        channel: &'static str,
        req_id: &str,
        work: impl FnOnce(&Cancel) -> T + Send + 'static,
        apply: impl FnOnce(&mut App, T) + Send + 'static,
    ) {
        let latest = req_id.to_string();
        let task = self.run(work, move |app, out| {
            let current = app.superseding.get(channel);
            if current.is_some_and(|(req_id, _)| *req_id == latest) {
                app.superseding.remove(channel);
                apply(app, out);
            }
        });
        // `apply` runs on this (the UI) thread, so it can't look before
        // this insert.
        let previous = self.superseding.insert(channel, (req_id.to_string(), task));
        if let Some((old, task)) = previous {
            log::debug!("{channel}: {req_id} supersedes {old}");
            task.abort();
        }
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
    /// The clock ticked (the window's timer, every minute): start what the
    /// app does on its own -- the mail and calendar auto-syncs that are due.
    pub fn on_clock(&mut self) {
        let now = DateTime::now();
        mail::auto_sync(self, now);
        calendar::auto_sync(self, now);
    }

    // ------------------------------------------------------------------------
    /// Make `id` the pending account connect. The one it replaces is
    /// cancelled; its own continuation then answers `cancelled`.
    pub fn begin_connect(&mut self, id: Id, cancel: Cancel) {
        let pending = PendingConnect { id, cancel };
        if let Some(old) = self.connect.replace(pending) {
            log::info!("Connect {}: replaced by {id}", old.id);
            old.cancel.cancel();
        }
    }

    // ------------------------------------------------------------------------
    /// Whether `id` is still the pending connect (not cancelled or
    /// replaced). Every continuation of a connect checks this first.
    pub fn is_connect_pending(&self, id: Id) -> bool {
        self.connect.as_ref().is_some_and(|c| c.id == id)
    }

    // ------------------------------------------------------------------------
    pub fn cancel_connect(&mut self) {
        if let Some(old) = self.connect.take() {
            log::info!("Connect {}: cancelling", old.id);
            old.cancel.cancel();
        }
    }

    // ------------------------------------------------------------------------
    /// The connect `id` is over (connected or failed).
    pub fn end_connect(&mut self, id: Id) {
        if self.is_connect_pending(id) {
            self.connect = None;
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
    /// Tell the page that `id` left the vault (trashed, or its file gone):
    /// the `entity_deleted` event.
    pub fn post_entity_deleted(&self, id: &Id) {
        self.post_event("entity_deleted", serde_json::json!({ "id": id }));
    }

    // ------------------------------------------------------------------------
    /// The window is active again: the page looks at the files it shows
    /// (the `activated` event, `saving.md#file-checks`), and the calendars that
    /// haven't looked at Google for a while sync (`calendar::activated`).
    pub fn on_activated(&mut self) {
        self.post_event("activated", ());
        calendar::activated(self, DateTime::now());
    }

    // ------------------------------------------------------------------------
    /// Send `id`'s body as it is now, unasked -- a view showing an older one
    /// loads it: the `content` event, the markdown source and its editor
    /// HTML.
    pub fn post_content(&self, id: &Id) {
        let (source, html) = self.vault.content(id);
        let content = serde_json::json!({ "id": id, "source": source, "html": html });
        self.post_event("content", content);
    }

    // ------------------------------------------------------------------------
    /// Run the vault edit `f` and send the page everything it wrote -- also
    /// when it failed part-way, since a multi-file edit stays part-way
    /// (`panel.md#saving-structure`). The one way the app edits the vault:
    /// the editor's own save, which is answered with what it wrote
    /// (`Vault::save_content`), and taking in a change made outside the app
    /// (`Vault::reload`, `Vault::refresh`) are not edits by the app. An
    /// event it wrote may wait to be pushed to Google (`calendar::changed`).
    pub fn edit<R>(&mut self, f: impl FnOnce(&mut Index, &mut Changes) -> Result<R>) -> Result<R> {
        let mut changes = Changes::default();
        let result = f(&mut self.vault.index, &mut changes);
        self.post_changes(&changes);
        calendar::changed(self, changes.created.iter().chain(&changes.saved));
        result
    }

    // ------------------------------------------------------------------------
    /// Send what an edit wrote: every created or rewritten entity
    /// (`entity`), a rewritten body (`content`), every trashed entity
    /// (`entity_deleted`).
    fn post_changes(&self, changes: &Changes) {
        for id in changes.created.iter().chain(&changes.saved) {
            if let Some(entry) = self.vault.index().entry(id) {
                self.post_entity(entry);
            }
        }
        for id in &changes.content {
            self.post_content(id);
        }
        for id in &changes.trashed {
            self.post_entity_deleted(id);
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
/// What `App::run_with_progress` hands its work: `send` queues the
/// run's `on_progress` on the UI thread, `send_latest` coalesces. It never
/// reaches the webview itself; only the queued continuation runs, with the
/// `App`.
pub struct Progress<P> {
    ui: Arc<UiQueue>,
    on_progress: Arc<Mutex<OnProgress<P>>>,
    /// The item of `send_latest` waiting for its queued task.
    latest: Arc<Mutex<Option<P>>>,
}

impl<P> Clone for Progress<P> {
    fn clone(&self) -> Self {
        Self {
            ui: Arc::clone(&self.ui),
            on_progress: Arc::clone(&self.on_progress),
            latest: Arc::clone(&self.latest),
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

    // ------------------------------------------------------------------------
    /// Queue `on_progress(app, item)` on the UI thread, replacing an
    /// item sent this way that hasn't arrived yet: for state so far (a
    /// reply's text, the words read), at most one per run of the UI queue
    /// however fast the work sends. A run uses either this or `send`.
    pub fn send_latest(&self, item: P) {
        if self.latest.lock().unwrap().replace(item).is_some() {
            return; // its task is queued already
        }
        let (latest, on_progress) = (Arc::clone(&self.latest), Arc::clone(&self.on_progress));
        self.ui.send_to_ui(move |app| {
            let Some(item) = latest.lock().unwrap().take() else {
                return;
            };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Harness;

    // ------------------------------------------------------------------------
    #[test]
    fn latest_progress_replaces_what_has_not_arrived() {
        let mut h = Harness::new("latest-progress");
        let got = Arc::new(Mutex::new(Vec::new()));
        let progress = {
            let got = Arc::clone(&got);
            h.app
                .progress(move |_app, n: u32| got.lock().unwrap().push(n))
        };
        for n in 1..=3 {
            progress.send_latest(n);
        }
        assert_eq!(h.ui.queued(), 1, "one task for all three");
        h.run_queue();
        progress.send_latest(4);
        h.run_queue();
        assert_eq!(*got.lock().unwrap(), [3, 4]);
    }
}
