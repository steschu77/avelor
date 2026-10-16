use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use av_utils::error::{Error, Result};
use gtk::gio;
use gtk::prelude::*;

// ----------------------------------------------------------------------------
/// The owner of the app's dialogs: the main window.
pub struct Owner(Option<gtk::ApplicationWindow>);

impl Owner {
    // ------------------------------------------------------------------------
    pub(super) fn new(window: gtk::ApplicationWindow) -> Self {
        Self(Some(window))
    }

    // ------------------------------------------------------------------------
    /// An owner without a window, for tests: they open no dialogs.
    #[cfg(test)]
    pub fn none() -> Self {
        Self(None)
    }
}

// ----------------------------------------------------------------------------
/// Run the GTK call `start` to its end on the UI thread: a nested main loop
/// dispatches meanwhile, as a Windows modal loop does (`App::on_ui`). What
/// it called back with.
fn wait<T: 'static>(start: impl FnOnce(Box<dyn FnOnce(T)>)) -> Option<T> {
    let result = Rc::new(RefCell::new(None));
    let nested = glib::MainLoop::new(None, false);
    {
        let (result, nested) = (Rc::clone(&result), nested.clone());
        start(Box::new(move |value| {
            *result.borrow_mut() = Some(value);
            nested.quit();
        }));
    }
    if result.borrow().is_none() {
        nested.run();
    }
    result.borrow_mut().take()
}

// ----------------------------------------------------------------------------
/// Show an error in a dialog without an owner: for failures before or after
/// the main window, when there is no page. Without a display it goes to stderr.
pub fn show_error(title: &str, text: &str) {
    if gtk::init().is_err() {
        eprintln!("{title}: {text}");
        return;
    }
    let dialog = gtk::AlertDialog::builder()
        .message(title)
        .detail(text)
        .modal(true)
        .build();
    wait(|done| {
        dialog.choose(None::<&gtk::Window>, None::<&gio::Cancellable>, move |_| {
            done(())
        })
    });
}

// ----------------------------------------------------------------------------
/// The folder the user picks in a dialog owned by `owner`; `None` if the
/// dialog was cancelled.
pub fn pick_folder(owner: &Owner, title: &str) -> Option<PathBuf> {
    let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
    let picked =
        wait(|done| dialog.select_folder(owner.0.as_ref(), None::<&gio::Cancellable>, done))?;
    picked.ok()?.path()
}

// ----------------------------------------------------------------------------
/// Open `url` in the default browser.
pub fn open_url(url: &str) -> Result<()> {
    let launcher = gtk::UriLauncher::new(url);
    launched(wait(|done| {
        launcher.launch(None::<&gtk::Window>, None::<&gio::Cancellable>, done)
    }))
}

// ----------------------------------------------------------------------------
/// Open `path` in its default app: a folder in the file manager.
pub fn open_path(path: &Path) -> Result<()> {
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    launched(wait(|done| {
        launcher.launch(None::<&gtk::Window>, None::<&gio::Cancellable>, done)
    }))
}

// ----------------------------------------------------------------------------
/// What a launcher called back with, as the app's result.
fn launched(launched: Option<std::result::Result<(), glib::Error>>) -> Result<()> {
    match launched {
        Some(Ok(())) => Ok(()),
        Some(Err(e)) => Err(Error::Gtk { msg: e.to_string() }),
        None => Err(Error::Cancelled),
    }
}
