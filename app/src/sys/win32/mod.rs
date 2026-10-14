//! The Windows backend: Win32 through the `windows` crate.

mod dialogs;
mod paths;
mod window;

use crate::config::ConfigStore;
use crate::shell::{Shell, TITLE};
use av_utils::error::Result;

pub use dialogs::show_error;
pub use paths::Paths;
pub use paths::paths;

// ----------------------------------------------------------------------------
/// Show the main window in the state `config` saved, and run the message
/// loop until the window is gone.
pub fn run(_paths: &Paths, config: ConfigStore) -> Result<()> {
    let window = window::WindowProc::create(TITLE, Some("APP_ICON"), Shell::new(config))?;
    window::run_message_loop();
    drop(window);
    Ok(())
}
