use std::path::Path;
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

// ----------------------------------------------------------------------------
/// `text` with a leading `~` replaced by the home folder, and each `$NAME`
/// or `${NAME}` by the environment variable `NAME`; a name that isn't set
/// is left as it is, as a shell does.
pub fn expand_env(text: &str) -> PathBuf {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    if rest == "~" || rest.starts_with("~/") {
        out.push_str(&glib::home_dir().to_string_lossy());
        rest = &rest[1..];
    }
    while let Some(start) = rest.find('$') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let (name, len) = match after.strip_prefix('{') {
            Some(braced) => match braced.find('}') {
                Some(end) => (&braced[..end], end + 2),
                None => ("", 0),
            },
            None => {
                let end = after
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(after.len());
                (&after[..end], end)
            }
        };
        match std::env::var(name) {
            Ok(value) if !name.is_empty() => out.push_str(&value),
            _ => out.push_str(&rest[start..start + 1 + len]),
        }
        rest = &after[len..];
    }
    out.push_str(rest);
    PathBuf::from(out)
}

// ----------------------------------------------------------------------------
/// `path` as a watch stores it: under the home folder as `~/…`, so the
/// file still fits after the home folder moves.
pub fn collapse_env(path: &Path) -> String {
    if let Ok(rest) = path.strip_prefix(glib::home_dir()) {
        let rest = rest.to_string_lossy();
        return if rest.is_empty() {
            "~".to_string()
        } else {
            format!("~/{rest}")
        };
    }
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn the_home_is_expanded_and_collapsed() {
        let home = glib::home_dir();
        assert_eq!(expand_env("~/Scans"), home.join("Scans"));
        assert_eq!(expand_env("~"), home);
        let var = std::env::var("HOME").unwrap();
        assert_eq!(expand_env("$HOME/Scans"), Path::new(&var).join("Scans"));
        assert_eq!(expand_env("${HOME}x"), PathBuf::from(format!("{var}x")));
        assert_eq!(
            expand_env("$AVELOR_SURELY_NOT_SET/x ${AVELOR_SURELY_NOT_SET} $"),
            PathBuf::from("$AVELOR_SURELY_NOT_SET/x ${AVELOR_SURELY_NOT_SET} $"),
            "unknown names stay"
        );
        assert_eq!(expand_env("/srv/100$"), PathBuf::from("/srv/100$"));
        assert_eq!(expand_env("a~/b"), PathBuf::from("a~/b"));
        assert_eq!(collapse_env(&home.join("Scans")), "~/Scans");
        assert_eq!(collapse_env(&home), "~");
        assert_eq!(collapse_env(Path::new("/srv/scans")), "/srv/scans");
    }
}
