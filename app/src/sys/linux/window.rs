use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::config::WindowState;
use crate::shell::{Shell, TITLE};

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
