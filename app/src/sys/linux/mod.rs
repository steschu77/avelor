//! The Linux backend: GTK4 through the `gtk4` crate.

mod dialogs;
mod paths;
mod webview;
mod window;

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::config::ConfigStore;
use crate::shell::Shell;
use av_utils::error::Result;

pub use dialogs::show_error;
pub use paths::Paths;
pub use paths::paths;
pub use webview::BRIDGE_JS;
pub use window::UiSignal;

// ----------------------------------------------------------------------------
/// The app's id with the session manager and D-Bus.
const APP_ID: &str = "io.github.steschu77.Avelor";

// ----------------------------------------------------------------------------
/// Show the main window in the state `config` saved, with the page in its
/// web view, and run the main loop until the window is gone. Each start is
/// a process of its own (`NON_UNIQUE`), as on Windows.
pub fn run(paths: &Paths, config: ConfigStore) -> Result<()> {
    let (data_dir, cache_dir) = (paths.webview_dir(), paths.webview_cache_dir());
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .register_session(true)
        .build();
    let shell = Rc::new(Shell::new(config));
    app.connect_activate(move |app| {
        let webview = webview::create(&data_dir, &cache_dir, &shell);
        let window = window::create(app, &webview, &shell);
        end_session_on_request(app, &window, &shell);
        shell.start(UiSignal::new(&shell, &window), webview::outbox(&webview));
        webview::load(&webview);
    });
    // GTK isn't handed the command line: it has no options of its own.
    app.run_with_args::<&str>(&[]);
    Ok(())
}

// ----------------------------------------------------------------------------
/// The session ends: the session manager's `query-end` (the app registers
/// with it), or SIGTERM or SIGHUP. The window's state is kept, without
/// asking anything (`architecture.md#principles`, principle 8); a signal
/// also ends the app. SIGINT (Ctrl+C in a terminal) closes the window as its
/// Close does. Each is logged, to see in the log how a session ended.
fn end_session_on_request(
    app: &gtk::Application,
    window: &gtk::ApplicationWindow,
    shell: &Rc<Shell>,
) {
    let (w, s) = (window.clone(), Rc::clone(shell));
    app.connect_query_end(move |_| {
        log::info!("The session manager is ending the session");
        window::save_window_state(&w, &s);
    });

    for (signal, name) in [(libc::SIGTERM, "SIGTERM"), (libc::SIGHUP, "SIGHUP")] {
        let (a, w, s) = (app.clone(), window.clone(), Rc::clone(shell));
        glib_unix::unix_signal_add_local_once(signal, move || {
            log::info!("{name}: the session ends");
            window::save_window_state(&w, &s);
            a.quit();
        });
    }

    let w = window.clone();
    glib_unix::unix_signal_add_local(libc::SIGINT, move || {
        log::info!("SIGINT: closing the window");
        w.close();
        glib::ControlFlow::Continue
    });
}
