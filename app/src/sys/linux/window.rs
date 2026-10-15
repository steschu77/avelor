use std::cell::RefCell;
use std::rc::Rc;
use std::rc::Weak;

use gtk::glib;
use gtk::prelude::*;

use crate::config::WindowState;
use crate::shell::{Shell, TITLE};

thread_local! {
    /// The shell whose queue `UiSignal::wake` runs, on the UI thread.
    static SHELL: RefCell<Weak<Shell>> = const { RefCell::new(Weak::new()) };
}

// ----------------------------------------------------------------------------
/// Wakes the app's queue from any thread: an idle callback of GTK's main
/// loop runs it (`Shell::run_tasks`). Not `MainContext::invoke`, which on
/// the UI thread calls at once, inside the task that woke the queue; the
/// queue runs once the main loop gets to it, as on Windows
/// (`architecture.md#one-way-in`).
pub struct UiSignal {
    live: bool,
}

impl UiSignal {
    // ------------------------------------------------------------------------
    /// The signal of `shell`'s queue; on the UI thread.
    pub fn new(shell: &Rc<Shell>, _window: &gtk::ApplicationWindow) -> Self {
        SHELL.with(|s| *s.borrow_mut() = Rc::downgrade(shell));
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

    // GTK has no end of a resize drag: the size is saved when the window
    // closes, a change of maximized when it happens.
    let saving = Rc::clone(shell);
    window.connect_close_request(move |window| {
        save_window_state(window, &saving);
        glib::Propagation::Proceed
    });
    let saving = Rc::clone(shell);
    window.connect_maximized_notify(move |window| save_window_state(window, &saving));

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
