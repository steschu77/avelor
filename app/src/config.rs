//! App configuration, persisted in the config folder's config.yaml.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use av_utils::error::{Error, Result};
use av_utils::fs::write_atomic;
use serde::{Deserialize, Serialize};

const CONFIG_FILE: &str = "config.yaml";
const BROKEN_FILE: &str = "config.yaml.broken";

// ----------------------------------------------------------------------------
pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowState>,
    /// UI preferences, owned by the frontend; the host only stores and
    /// replays them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prefs: BTreeMap<String, String>,
    /// The keys this version doesn't know, written back as they were read,
    /// so the settings of another version survive a save.
    #[serde(flatten)]
    pub other: BTreeMap<String, serde_yaml_ng::Value>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub maximized: bool,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Default)]
pub struct ConfigStore {
    config: AppConfig,
    path: PathBuf,
    error: Option<Error>,
    /// The vault the command line names: opened instead of the last
    /// session's, and not remembered.
    session_vault: Option<PathBuf>,
}

// ----------------------------------------------------------------------------
impl ConfigStore {
    // ------------------------------------------------------------------------
    /// The config at `path`; the defaults if there is no file yet.
    fn load_from(path: &Path) -> Result<Self> {
        let config = match fs::read(path) {
            Ok(t) => serde_yaml_ng::from_slice(&t)?,
            Err(e) if e.kind() == ErrorKind::NotFound => AppConfig::default(),
            Err(e) => return Err(Error::from(e)),
        };

        Ok(Self {
            config,
            path: path.to_path_buf(),
            error: None,
            session_vault: None,
        })
    }

    // ------------------------------------------------------------------------
    /// The folder holding config.yaml.
    pub fn dir(&self) -> &Path {
        self.path.parent().unwrap_or(Path::new(""))
    }

    // ------------------------------------------------------------------------
    /// The vault to open: the command line's, else the last session's.
    pub fn vault(&self) -> Option<&Path> {
        self.session_vault
            .as_deref()
            .or(self.config.vault.as_deref())
    }

    // ------------------------------------------------------------------------
    /// Open `root` in this session instead of the last session's vault,
    /// without remembering it.
    pub fn set_session_vault(&mut self, root: PathBuf) {
        self.session_vault = Some(root);
    }

    // ------------------------------------------------------------------------
    /// Remember `root` as the vault to open at the next start, and write the
    /// file.
    pub fn set_vault(&mut self, root: PathBuf) -> Result<()> {
        self.session_vault = None;
        self.config.vault = Some(root);
        self.save()
    }

    // ------------------------------------------------------------------------
    pub fn window(&self) -> Option<WindowState> {
        self.config.window
    }

    // ------------------------------------------------------------------------
    pub fn prefs(&self) -> &BTreeMap<String, String> {
        &self.config.prefs
    }

    // ------------------------------------------------------------------------
    /// Store the page's preferences as given, and write the file.
    pub fn set_prefs(&mut self, prefs: BTreeMap<String, String>) -> Result<()> {
        self.config.prefs = prefs;
        self.save()
    }

    // ------------------------------------------------------------------------
    pub fn set_window(&mut self, state: WindowState) -> Result<()> {
        if self.config.window == Some(state) {
            return Ok(());
        }
        self.config.window = Some(state);
        self.save()
    }

    // ------------------------------------------------------------------------
    pub fn take_error(&mut self) -> Option<Error> {
        self.error.take()
    }

    // ------------------------------------------------------------------------
    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_yaml_ng::to_string(&self.config)?;
        write_atomic(&self.path, text.as_bytes())
    }
}

// ----------------------------------------------------------------------------
/// The config of the last session in `dir`. A config file that can't be
/// parsed is set aside, so the defaults can be used and saved; `Err` means
/// the file is still there and must not be overwritten.
pub fn load_config(dir: &Path) -> Result<ConfigStore> {
    let path = dir.join(CONFIG_FILE);
    match ConfigStore::load_from(&path) {
        Ok(store) => Ok(store),
        Err(err @ Error::Serde { .. }) => {
            log::error!("Can't read {}: {err}", path.display());
            set_aside(dir)?;
            Ok(ConfigStore {
                config: AppConfig::default(),
                path,
                error: Some(err),
                session_vault: None,
            })
        }
        Err(err) => Err(err),
    }
}

// ----------------------------------------------------------------------------
/// Moves the broken config file in `dir` out of the way. An existing
/// `.broken` file is kept, as it may hold the user's config with a syntax
/// error; the config file is discarded then.
fn set_aside(dir: &Path) -> Result<()> {
    let config_file = dir.join(CONFIG_FILE);
    let broken_file = dir.join(BROKEN_FILE);

    // `fs::rename` replaces an existing target, so check first.
    if broken_file.exists() {
        fs::remove_file(&config_file)?;
        log::warn!(
            "Broken config file {} discarded, keeping {}",
            config_file.display(),
            broken_file.display()
        );
    } else {
        fs::rename(&config_file, &broken_file)?;
        log::warn!("Broken config file moved to {}", broken_file.display());
    }

    Ok(())
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("avelor-config-{name}-{}", process::id()));
        println!("Temporary directory: {}", dir.display());
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_config_loads_as_the_defaults() {
        let dir = temp_dir("missing");
        let config = load_config(&dir).unwrap();
        assert!(config.error.is_none());
        assert!(config.window().is_none());
        assert!(config.prefs().is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_command_lines_vault_is_opened_not_remembered() {
        let dir = temp_dir("session");
        let mut config = load_config(&dir).unwrap();
        config.set_vault(PathBuf::from("last")).unwrap();
        config.set_session_vault(PathBuf::from("named"));
        assert_eq!(config.vault(), Some(Path::new("named")));
        let saved = load_config(&dir).unwrap();
        assert_eq!(saved.vault(), Some(Path::new("last")));

        config.set_vault(PathBuf::from("switched")).unwrap();
        assert_eq!(config.vault(), Some(Path::new("switched")));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unreadable_config_is_left_in_place() {
        let dir = temp_dir("unreadable");

        // A directory can't be read as a file.
        let config_file = dir.join("config.yaml");
        fs::create_dir(&config_file).unwrap();
        assert!(matches!(load_config(&dir), Err(Error::FileIo { .. })));

        assert!(config_file.exists());
        assert!(!dir.join("config.yaml.broken").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_broken_config_is_set_aside() {
        let broken_unopened = "window: unopened]\n";
        let broken_unclosed = "window: [unclosed\n";
        let dir = temp_dir("broken");

        let config_file = dir.join("config.yaml");
        let broken_file = dir.join("config.yaml.broken");

        fs::write(&config_file, broken_unopened).unwrap();
        let config = load_config(&dir).unwrap();
        assert!(matches!(config.error, Some(Error::Serde { .. })));
        assert!(config.window().is_none());

        assert!(broken_file.exists());
        assert_eq!(fs::read_to_string(&broken_file).unwrap(), broken_unopened);

        assert!(!config_file.exists());
        fs::write(&config_file, broken_unclosed).unwrap();
        let config = load_config(&dir).unwrap();
        assert!(matches!(config.error, Some(Error::Serde { .. })));
        assert!(config.window().is_none());

        // still the initial broken file
        assert!(broken_file.exists());
        assert_eq!(fs::read_to_string(&broken_file).unwrap(), broken_unopened);

        assert!(!config_file.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_save_keeps_the_keys_it_doesnt_know() {
        let dir = temp_dir("unknown");
        let config_file = dir.join("config.yaml");
        fs::write(&config_file, "zoom: 2\nlook:\n  theme: dark\n").unwrap();

        let mut config = load_config(&dir).unwrap();
        let state = WindowState {
            width: 800,
            height: 600,
            maximized: true,
        };
        config.set_window(state).unwrap();

        let text = fs::read_to_string(&config_file).unwrap();
        assert!(text.contains("zoom: 2"), "{text}");
        assert!(text.contains("theme: dark"), "{text}");
        assert_eq!(load_config(&dir).unwrap().window(), Some(state));
        fs::remove_dir_all(&dir).unwrap();
    }
}
