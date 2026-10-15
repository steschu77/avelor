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
    /// `$XDG_CONFIG_HOME/avelor`: `config.yaml`.
    pub fn config_dir(&self) -> PathBuf {
        glib::user_config_dir().join(APP_DIR)
    }

    // ------------------------------------------------------------------------
    /// `$XDG_DATA_HOME/avelor/webkit`: the web view's data.
    pub fn webview_dir(&self) -> PathBuf {
        glib::user_data_dir().join(APP_DIR).join("webkit")
    }

    // ------------------------------------------------------------------------
    /// `$XDG_CACHE_HOME/avelor/webkit`: the web view's cache.
    pub fn webview_cache_dir(&self) -> PathBuf {
        glib::user_cache_dir().join(APP_DIR).join("webkit")
    }

    // ------------------------------------------------------------------------
    /// `$XDG_STATE_HOME/avelor/log`: one log file per run.
    pub fn log_dir(&self) -> PathBuf {
        glib::user_state_dir().join(APP_DIR).join("log")
    }
}
