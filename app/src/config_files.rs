//! What the YAML files of the configuration share (`accounts.md#design`):
//! accounts of both kinds and folder watches are each one YAML file, an
//! account's secret a DPAPI-encrypted file with the app (on Linux, a file
//! only the user can read).

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use av_utils::error::{Error, Result};
use av_utils::fs::{write_atomic, write_private};
use av_utils::id::Id;
use av_utils::secret;
use serde::Serialize;

// ----------------------------------------------------------------------------
/// An account's secret (a refresh token, an API key), DPAPI-encrypted in
/// the app's config folder (on Linux, readable by the user alone) and named
/// by the account's id (`<id>.token`, `<id>.key`): the account file doesn't
/// name it, and it never enters a vault.
#[derive(Debug, Clone, PartialEq)]
pub struct SecretFile(PathBuf);

impl SecretFile {
    // ------------------------------------------------------------------------
    /// The secret of the account `id` in `secrets`; `ext` says which kind.
    pub fn of(secrets: &Path, id: Id, ext: &str) -> Self {
        Self(secrets.join(format!("{id}.{ext}")))
    }

    // ------------------------------------------------------------------------
    /// The secret, decrypted: `FileNotFound` if there is none, an error if
    /// it can't be decrypted (e.g. it was copied from another Windows user).
    pub fn read(&self) -> Result<String> {
        let cipher = fs::read(&self.0).map_err(|e| match e.kind() {
            ErrorKind::NotFound => Error::FileNotFound {
                path: self.0.clone(),
            },
            _ => Error::from(e),
        })?;
        Ok(String::from_utf8(secret::unprotect(&cipher)?)?)
    }

    // ------------------------------------------------------------------------
    /// Store `secret`, encrypted, replacing the one there is.
    pub fn write(&self, secret: &str) -> Result<()> {
        if let Some(dir) = self.0.parent() {
            fs::create_dir_all(dir)?;
        }
        write_private(&self.0, &secret::protect(secret.as_bytes())?)
    }

    // ------------------------------------------------------------------------
    /// Delete the secret, if there is one.
    pub fn remove(&self) -> Result<()> {
        remove_if_exists(&self.0)
    }
}

// ----------------------------------------------------------------------------
/// Whether an account with a secret can be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionStatus {
    /// Its secret was read.
    Connected,
    /// Deliberately without a secret: its file is marked `disconnected`.
    Disconnected,
    /// The secret is missing or can't be decrypted on this computer (a
    /// vault opened somewhere else, or as another user).
    Reconnect,
}

impl ConnectionStatus {
    // ------------------------------------------------------------------------
    /// The status of an account whose secret was read (`secret_read`), and
    /// whose file is marked `disconnected` or not.
    pub fn of(secret_read: bool, disconnected: bool) -> Self {
        match (secret_read, disconnected) {
            (true, _) => ConnectionStatus::Connected,
            (false, true) => ConnectionStatus::Disconnected,
            (false, false) => ConnectionStatus::Reconnect,
        }
    }

    // ------------------------------------------------------------------------
    /// The name used in messages to the page.
    pub fn name(self) -> &'static str {
        match self {
            ConnectionStatus::Connected => "connected",
            ConnectionStatus::Disconnected => "disconnected",
            ConnectionStatus::Reconnect => "reconnect",
        }
    }
}

// ----------------------------------------------------------------------------
/// Write `value` as YAML to `path`, through a temporary file and a rename.
pub fn write_yaml(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    write_atomic(path, serde_yaml_ng::to_string(value)?.as_bytes())
}

// ----------------------------------------------------------------------------
pub(crate) fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

// ----------------------------------------------------------------------------
/// What `load` makes of every `*.yaml` in `dir` but config.yaml, in file
/// name order; `None` skips a file of another kind. A broken file is logged
/// (as a `what`) and skipped, so it can't keep the app from starting.
pub(crate) fn load_config_files<T>(
    dir: &Path,
    what: &str,
    load: impl Fn(&Path) -> Result<Option<T>>,
) -> Vec<T> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml"))
        })
        .filter(|p| p.file_name().is_some_and(|name| name != "config.yaml"))
        .collect();
    paths.sort();

    paths
        .iter()
        .filter_map(|path| match load(path) {
            Ok(item) => item,
            Err(e) => {
                log::warn!("Skipping {what} {}: {e}", path.display());
                None
            }
        })
        .collect()
}

// ----------------------------------------------------------------------------
/// The first file stem in `dir` that no file with one of `extensions` has
/// taken: `base`, then `base_2`, `base_3`, …
pub(crate) fn free_stem(dir: &Path, base: &str, extensions: &[&str]) -> String {
    let taken = |stem: &str| {
        extensions
            .iter()
            .any(|ext| dir.join(format!("{stem}.{ext}")).exists())
    };
    (1..)
        .map(|n| match n {
            1 => base.to_string(),
            n => format!("{base}_{n}"),
        })
        .find(|stem| !taken(stem))
        .unwrap_or_default()
}

// ----------------------------------------------------------------------------
/// `names` without empty entries and repeats, in their order.
pub(crate) fn unique_names(names: Vec<String>) -> Vec<String> {
    let mut unique = Vec::with_capacity(names.len());
    for name in names {
        if !name.is_empty() && !unique.contains(&name) {
            unique.push(name);
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("avelor-config-files-{}", Id::generate()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_status_follows_the_secret_then_the_mark() {
        use ConnectionStatus::*;
        assert_eq!(ConnectionStatus::of(true, false), Connected);
        assert_eq!(ConnectionStatus::of(true, true), Connected);
        assert_eq!(ConnectionStatus::of(false, true), Disconnected);
        assert_eq!(ConnectionStatus::of(false, false), Reconnect);
        assert_eq!(Reconnect.name(), "reconnect");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_secret_is_named_by_its_account_and_kept_protected() {
        let dir = temp_dir();
        let id = Id::generate();
        let secret = SecretFile::of(&dir, id, "token");
        assert!(matches!(secret.read(), Err(Error::FileNotFound { .. })));

        secret.write("1//refresh").unwrap();
        let path = dir.join(format!("{id}.token"));
        #[cfg(target_os = "windows")]
        let stored = fs::read(&path).unwrap();
        #[cfg(target_os = "linux")]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(&path).unwrap().permissions().mode() & 0o777
        };
        let read = secret.read().unwrap();
        secret.remove().unwrap();
        secret.remove().unwrap();
        let gone = !path.exists();
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(read, "1//refresh");
        #[cfg(target_os = "windows")]
        assert!(!stored.windows(9).any(|w| w == b"1//refres"), "encrypted");
        #[cfg(target_os = "linux")]
        assert_eq!(mode, 0o600, "only the user can read it");
        assert!(gone, "removed, and removing again is fine");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn unique_names_drop_empties_and_repeats_in_order() {
        let names = ["b", "", "a", "b", "c", "a"].map(String::from).to_vec();
        assert_eq!(unique_names(names), ["b", "a", "c"]);
    }
}
