use std::path::Path;
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

    // ------------------------------------------------------------------------
    /// `%LOCALAPPDATA%\Avelor\catalog`: the vaults' catalogs.
    pub fn catalog_dir(&self) -> PathBuf {
        self.app_dir.join("catalog")
    }
}

// ----------------------------------------------------------------------------
/// `text` with each `%NAME%` replaced by the environment variable `NAME`;
/// a name that isn't set is left as it is (like Windows does).
pub fn expand_env(text: &str) -> PathBuf {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(len) = after.find('%') else {
            break;
        };
        let name = &after[..len];
        out.push_str(&rest[..start]);
        match std::env::var(name) {
            Ok(value) if !name.is_empty() => out.push_str(&value),
            _ => {
                out.push('%');
                out.push_str(name);
                out.push('%');
            }
        }
        rest = &after[len + 1..];
    }
    out.push_str(rest);
    PathBuf::from(out)
}

// ----------------------------------------------------------------------------
/// `path` as a watch stores it: under the user's profile as
/// `%USERPROFILE%\…`, so the file still fits after the profile moves.
pub fn collapse_env(path: &Path) -> String {
    if let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        if let Ok(rest) = path.strip_prefix(&home) {
            let rest = rest.to_string_lossy();
            return if rest.is_empty() {
                "%USERPROFILE%".to_string()
            } else {
                format!("%USERPROFILE%\\{rest}")
            };
        }
    }
    path.to_string_lossy().into_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn the_profile_is_expanded_and_collapsed() {
        let home = PathBuf::from(std::env::var("USERPROFILE").unwrap());
        assert_eq!(expand_env(r"%USERPROFILE%\Scans"), home.join("Scans"));
        assert_eq!(
            expand_env(r"%AVELOR_SURELY_NOT_SET%\x %%"),
            PathBuf::from(r"%AVELOR_SURELY_NOT_SET%\x %%"),
            "unknown names stay"
        );
        assert_eq!(expand_env("C:\\100%"), PathBuf::from("C:\\100%"));
        assert_eq!(collapse_env(&home.join("Scans")), r"%USERPROFILE%\Scans");
        assert_eq!(collapse_env(&home), "%USERPROFILE%");
        assert_eq!(collapse_env(Path::new(r"D:\Scans")), r"D:\Scans");
    }
}
