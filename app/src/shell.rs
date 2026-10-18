//! What the main window decides, the same on every platform
//! (`architecture.md#platforms`). Each platform's window only turns its
//! native events into calls on it.

use std::cell::RefCell;
use std::cell::{Cell, OnceCell};
use std::sync::Arc;

use crate::app::Prefs;
use crate::app::{App, Outbox, UiQueue, UiTask};
use crate::close;
use crate::config::{ConfigStore, WindowState};
use crate::sys::Owner;
use crate::sys::UiSignal;
use av_utils::error::Error;
use av_utils::error::Result;

// ----------------------------------------------------------------------------
/// The main window's title.
pub const TITLE: &str = "Avelor";

/// The main window's size until one is saved.
const DEFAULT_STATE: WindowState = WindowState {
    width: 1280,
    height: 720,
    maximized: false,
};

// ----------------------------------------------------------------------------
pub struct Shell {
    /// The app config, held until `App` exists and takes it over.
    config: RefCell<Option<ConfigStore>>,
    /// Taken only by `run_tasks`, for one task at a time. A message that
    /// finds it taken came from a modal loop inside that task, and its work
    /// waits in the queue until the task returns.
    app: RefCell<Option<App>>,
    /// The app's task queue.
    ui: OnceCell<Arc<UiQueue>>,
    /// A task waits in the queue because it found `App` taken; the
    /// task holding it wakes the queue when it returns.
    deferred: Cell<bool>,
    /// The owner of the dialogs of `UiTask::Modal` tasks: the main window.
    owner: OnceCell<Owner>,
    /// The dialogs of `UiTask::Modal` tasks that are open; the window can't
    /// be closed meanwhile.
    modal: Cell<u32>,
}

impl Shell {
    // ------------------------------------------------------------------------
    pub fn new(config: ConfigStore) -> Self {
        Self {
            config: RefCell::new(Some(config)),
            app: RefCell::new(None),
            ui: OnceCell::new(),
            deferred: Cell::new(false),
            owner: OnceCell::new(),
            modal: Cell::new(0),
        }
    }

    // ------------------------------------------------------------------------
    /// The web view is up: `App` starts with the config, posting to the
    /// page through `outbox`, its queue woken by `signal`, its dialogs owned
    /// by `owner`. Returns the page's preferences, which the page is loaded
    /// with.
    pub fn start(&self, signal: UiSignal, owner: Owner, outbox: Outbox) -> Prefs {
        let Some(config) = self.config.take() else {
            return Prefs::new();
        };
        let prefs = config.prefs().clone();
        let ui = Arc::new(UiQueue::new(signal));
        let app = App::new(Arc::clone(&ui), outbox, config);
        if let Ok(mut slot) = self.app.try_borrow_mut() {
            *slot = Some(app);
        }
        let _ = self.ui.set(ui);
        let _ = self.owner.set(owner);
        prefs
    }

    // ------------------------------------------------------------------------
    /// A message arrived from the page's JS via `window.host.post`.
    pub fn on_web_message(&self, message: String) {
        let _ = self.queue(move |app| app.on_web_message(message));
    }

    // ------------------------------------------------------------------------
    /// Run `f` with `App` after everything queued before it: right
    /// away, or -- if a handler further up the stack holds it, and
    /// this message came from a modal loop inside it -- once that handler
    /// has returned.
    fn queue(&self, f: impl FnOnce(&mut App) + Send + 'static) -> Result<()> {
        let Some(ui) = self.ui.get() else {
            return Err(Error::NotInitialized);
        };
        ui.push(UiTask::App(Box::new(f)));
        self.run_tasks();
        Ok(())
    }

    // ------------------------------------------------------------------------
    /// The window's Close: `App` asks the page first (`close::on_close`) and
    /// closes the window once it is ready. False before `App` is up: there
    /// is no page to ask, and the window closes at once.
    pub fn on_close(&self) -> bool {
        self.queue(close::on_close).is_ok()
    }

    // ------------------------------------------------------------------------
    /// The session ends: `App` keeps what only it has
    /// (`close::on_session_end`); before `App` is up nothing is held.
    pub fn on_session_end(&self) {
        let _ = self.queue(close::on_session_end);
    }

    // ------------------------------------------------------------------------
    /// The window became active again (`App::on_activated`); before `App`
    /// is up there is no page to tell.
    pub fn on_activated(&self) {
        let _ = self.queue(App::on_activated);
    }

    // ------------------------------------------------------------------------
    /// The clock ticked: the app starts what is due, from the UI queue
    /// like every other way in (`App::on_clock`); before `App` is up
    /// nothing is due.
    pub fn on_clock(&self) {
        let _ = self.queue(App::on_clock);
    }

    // ------------------------------------------------------------------------
    /// Whether a dialog or a handler is running further up the stack: a
    /// modal loop dispatched this message. The window's Close is refused
    /// then.
    pub fn is_busy(&self) -> bool {
        self.modal.get() > 0 || self.app.try_borrow_mut().is_err()
    }

    // ------------------------------------------------------------------------
    /// Run the tasks queued so far, one at a time and in order, each with
    /// `App` taken while it runs. A task queued meanwhile (the next step of
    /// a scan) waits for the next wake (`UiSignal::wake`), so the main loop runs
    /// in between. A dialog (`UiTask::Modal`) runs without `App`, so what its
    /// modal loop dispatches runs normally meanwhile. If the `App` is taken
    /// already, this call comes from a modal loop inside a running task: the
    /// queue waits, and that task wakes it on return.
    pub fn run_tasks(&self) {
        let Some(ui) = self.ui.get() else {
            return;
        };
        for _ in 0..ui.queued() {
            let Ok(mut slot) = self.app.try_borrow_mut() else {
                log::debug!("UI tasks wait for a handler in a modal loop");
                self.deferred.set(true);
                return;
            };
            let Some(task) = ui.pop() else {
                return;
            };
            match task {
                UiTask::App(task) => {
                    if let Some(app) = slot.as_mut() {
                        task(app);
                    }
                    drop(slot);
                }
                UiTask::Modal(work) => {
                    drop(slot);
                    if let Some(owner) = self.owner.get() {
                        self.modal.set(self.modal.get() + 1);
                        let apply = work(owner);
                        self.modal.set(self.modal.get() - 1);
                        // Free again: whatever the dialog's loop ran has
                        // returned.
                        if let Ok(mut slot) = self.app.try_borrow_mut()
                            && let Some(app) = slot.as_mut()
                        {
                            apply(app);
                        }
                    }
                }
            }
            // `App` is free again: wake the queue for what waited.
            if self.deferred.take() {
                ui.wake();
            }
        }
    }

    // ------------------------------------------------------------------------
    /// The state the main window opens in: as it was last saved, or the
    /// default size.
    pub fn window_state(&self) -> WindowState {
        let config = self.config.borrow();
        config
            .as_ref()
            .and_then(ConfigStore::window)
            .unwrap_or(DEFAULT_STATE)
    }

    // ------------------------------------------------------------------------
    /// The window's state changed: a drag ended, it was maximized or
    /// restored, or it closes. `App` persists it (`App::save_window_state`).
    pub fn on_window_state(&self, state: WindowState) {
        let _ = self.queue(move |app| app.save_window_state(state));
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;

    #[test]
    fn the_window_opens_as_it_was_saved() {
        let dir = std::env::temp_dir().join(format!("avelor-shell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let shell = Shell::new(load_config(&dir).unwrap());
        assert_eq!(shell.window_state(), DEFAULT_STATE);
        shell.start(UiSignal::none(), Owner::none(), Box::new(|_| {}));
        let state = WindowState {
            width: 900,
            height: 700,
            maximized: true,
        };
        shell.on_window_state(state);

        let shell = Shell::new(load_config(&dir).unwrap());
        assert_eq!(shell.window_state(), state);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
