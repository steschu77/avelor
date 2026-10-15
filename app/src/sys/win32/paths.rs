use std::path::PathBuf;

use av_utils::error::{Error, Result};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

// ----------------------------------------------------------------------------
/// The app's folders, all in `%LOCALAPPDATA%\Avelor`.
pub struct Paths {
    app_dir: PathBuf,
}

// ----------------------------------------------------------------------------
pub fn paths() -> Result<Paths> {
    let app_dir = known_folder(&FOLDERID_LocalAppData)?.join("Avelor");
    Ok(Paths { app_dir })
}

impl Paths {
    // ------------------------------------------------------------------------
    /// `%LOCALAPPDATA%\Avelor`: `config.yaml`.
    pub fn config_dir(&self) -> PathBuf {
        self.app_dir.clone()
    }

    // ------------------------------------------------------------------------
    /// `%LOCALAPPDATA%\Avelor\WebView2`: the web view's profile.
    pub fn webview_dir(&self) -> PathBuf {
        self.app_dir.join("WebView2")
    }

    // ------------------------------------------------------------------------
    /// `%LOCALAPPDATA%\Avelor\log`: one log file per run.
    pub fn log_dir(&self) -> PathBuf {
        self.app_dir.join("log")
    }
}

// ----------------------------------------------------------------------------
fn known_folder(id: &windows::core::GUID) -> Result<PathBuf> {
    unsafe {
        let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).map_err(Error::from)?;
        let result = path.to_string().map(PathBuf::from);
        CoTaskMemFree(Some(path.as_ptr() as *const _));
        result.map_err(Error::from)
    }
}
