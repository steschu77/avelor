use std::cell::RefCell;
use std::rc::Rc;
use std::rc::Weak;

use gtk::glib;
use gtk::prelude::*;
use webkit6::prelude::WebViewExt;

use crate::config::WindowState;
use crate::shell::{Shell, TITLE};

/// The clock's period: `App::on_clock` once a minute.
const CLOCK_SECONDS: u32 = 60;

thread_local! {
    /// The shell whose queue `UiSignal::wake` runs, on the UI thread.
    static SHELL: RefCell<Weak<Shell>> = const { RefCell::new(Weak::new()) };
    /// The window `UiSignal::close` closes.
    static WINDOW: RefCell<glib::WeakRef<gtk::ApplicationWindow>> =
        RefCell::new(glib::WeakRef::new());
}

// ----------------------------------------------------------------------------
/// Wakes the app's queue from any thread: an idle callback of GTK's main
/// loop runs it (`Shell::run_tasks`); and closes the window for the app. Not `MainContext::invoke`, which on
/// the UI thread calls at once, inside the task that woke the queue; the
/// queue runs once the main loop gets to it, as on Windows
/// (`architecture.md#one-way-in`).
pub struct UiSignal {
    live: bool,
}

impl UiSignal {
    // ------------------------------------------------------------------------
    /// The signal of `shell`'s queue and `window`; on the UI thread.
    pub fn new(shell: &Rc<Shell>, window: &gtk::ApplicationWindow) -> Self {
        SHELL.with(|s| *s.borrow_mut() = Rc::downgrade(shell));
        WINDOW.with(|w| w.borrow().set(Some(window)));
        Self { live: true }
    }

    // ------------------------------------------------------------------------
    /// A signal without a main loop, for tests: they run the queue themselves.
    #[cfg(test)]
    pub fn none() -> Self {
        Self { live: false }
    }

    // ------------------------------------------------------------------------
    /// Have the main loop run the queue.
    pub fn wake(&self) {
        if !self.live {
            return;
        }
        glib::idle_add_once(|| {
            if let Some(shell) = SHELL.with(|s| s.borrow().upgrade()) {
                shell.run_tasks();
            }
        });
    }

    // ------------------------------------------------------------------------
    /// Have the main loop close the window, its state kept: the app's close,
    /// once the page is ready. Closing the window ends the app.
    pub fn close(&self) {
        if !self.live {
            return;
        }
        glib::idle_add_once(|| {
            let shell = SHELL.with(|s| s.borrow().upgrade());
            let window = WINDOW.with(|w| w.borrow().upgrade());
            if let (Some(shell), Some(window)) = (shell, window) {
                save_window_state(&window, &shell);
                window.destroy();
            }
        });
    }
}

// ----------------------------------------------------------------------------
/// Create the main window of `app` in the state the shell saved, or at the
/// default size if there is none, with `webview` filling it, and show it.
pub fn create(
    app: &gtk::Application,
    webview: &webkit6::WebView,
    shell: &Rc<Shell>,
) -> gtk::ApplicationWindow {
    let state = shell.window_state();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title(TITLE)
        .default_width(state.width)
        .default_height(state.height)
        .maximized(state.maximized)
        .child(webview)
        .build();

    // The window's Close asks the app first, which closes the window once
    // the page is ready (`UiSignal::close`); before the app is up there is no
    // page to ask. Refused while a dialog or a handler is still running: a
    // nested loop dispatched it. GTK has no end of a resize drag: the size is saved when
    // the window closes, a change of maximized when it happens.
    let closing = Rc::clone(shell);
    window.connect_close_request(move |window| {
        if closing.is_busy() {
            log::info!("Close refused: a dialog or a handler is still running");
            return glib::Propagation::Stop;
        }
        if closing.on_close() {
            return glib::Propagation::Stop;
        }
        save_window_state(window, &closing);
        glib::Propagation::Proceed
    });
    let saving = Rc::clone(shell);
    window.connect_maximized_notify(move |window| save_window_state(window, &saving));
    // Back from another program, the page looks at the files it shows: one
    // may have been changed there (`saving.md#held-saves`,
    // `App::on_activated`). Not while the page loads, as the window first
    // becomes active: it doesn't listen yet.
    let activating = Rc::clone(shell);
    let loading = webview.clone();
    window.connect_is_active_notify(move |window| {
        if window.is_active() && !loading.is_loading() {
            activating.on_activated();
        }
    });

    // The clock: what the app does on its own (mail auto-sync).
    let ticking = Rc::clone(shell);
    glib::timeout_add_seconds_local(CLOCK_SECONDS, move || {
        ticking.on_clock();
        glib::ControlFlow::Continue
    });

    window.present();
    window
}

// ----------------------------------------------------------------------------
/// Hand the window state to the shell, which persists it. The size is the
/// restored (non-maximized) one: GTK keeps it as the default size.
pub fn save_window_state(window: &gtk::ApplicationWindow, shell: &Shell) {
    let (width, height) = window.default_size();
    let state = WindowState {
        width,
        height,
        maximized: window.is_maximized(),
    };
    shell.on_window_state(state);
}
