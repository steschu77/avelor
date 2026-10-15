use std::path::PathBuf;

use av_utils::error::Result;

// ----------------------------------------------------------------------------
/// The app's folder name in each XDG base directory.
const APP_DIR: &str = "avelor";

// ----------------------------------------------------------------------------
/// The app's folders, each in its XDG base directory.
pub struct Paths {}

// ----------------------------------------------------------------------------
pub fn paths() -> Result<Paths> {
    Ok(Paths {})
}

impl Paths {
    // ------------------------------------------------------------------------
    /// `$XDG_STATE_HOME/avelor/log`: one log file per run.
    pub fn log_dir(&self) -> PathBuf {
        glib::user_state_dir().join(APP_DIR).join("log")
    }
}
