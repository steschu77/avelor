//! The Windows backend: Win32 through the `windows` crate.

mod dialogs;
mod paths;
mod webview;
mod window;

use crate::config::ConfigStore;
use crate::shell::{Shell, TITLE};
use av_utils::error::Result;

pub use dialogs::show_error;
pub use paths::Paths;
pub use paths::paths;
pub use webview::BRIDGE_JS;
pub use window::UiSignal;

// ----------------------------------------------------------------------------
/// Show the main window in the state `config` saved, with the page in its
/// web view, and run the message loop until the window is gone.
pub fn run(paths: &Paths, config: ConfigStore) -> Result<()> {
    let webview_dir = paths.webview_dir();
    let shell = Shell::new(config);
    let window = window::WindowProc::create(TITLE, Some("APP_ICON"), webview_dir, shell)?;
    window::run_message_loop();
    drop(window);
    Ok(())
}
