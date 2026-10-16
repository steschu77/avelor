//! File helpers.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

// ----------------------------------------------------------------------------
/// Replace the file at `path` with `bytes` as one step: write a temporary
/// file beside it (`.{name}.tmp`), flush it to disk, then rename it over
/// `path`. A crash or a full disk leaves the old file whole instead of a
/// truncated one. Creates the file if it doesn't exist; the folder must.
/// A failure is `Error::FileWrite` with `path`. On Linux the new file keeps
/// the old one's mode.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let temp = temp_path(path)?;
    let written = write_synced(&temp, bytes, false)
        .and_then(|()| keep_mode(path, &temp))
        .and_then(|()| fs::rename(&temp, path));
    if let Err(err) = written {
        let _ = fs::remove_file(&temp); // best effort: it may not exist
        let path = path.to_path_buf();
        return Err(Error::FileWrite { path, err });
    }
    Ok(())
}

// ----------------------------------------------------------------------------
/// `.{name}.tmp` beside `path`: a dot-file, so vault scans skip it.
fn temp_path(path: &Path) -> Result<PathBuf> {
    let Some(name) = path.file_name() else {
        return Err(Error::InvalidPath {
            path: path.to_path_buf(),
        });
    };
    Ok(path.with_file_name(format!(".{}.tmp", name.to_string_lossy())))
}

// ----------------------------------------------------------------------------
fn write_synced(path: &Path, bytes: &[u8], _private: bool) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

// ----------------------------------------------------------------------------
/// On Linux, `temp` takes the permissions of the file at `path` it
/// replaces, if there is one; a new file has the user's defaults. On
/// Windows the folder decides.
fn keep_mode(path: &Path, temp: &Path) -> std::io::Result<()> {
    if !cfg!(unix) {
        return Ok(());
    }
    match fs::metadata(path) {
        Ok(metadata) => fs::set_permissions(temp, metadata.permissions()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use std::fs;
    use std::process;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("av-utils-{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_atomic_creates_and_replaces_without_leftovers() {
        let dir = temp_dir("write-atomic");
        let path = dir.join("note.md");

        write_atomic(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");
        write_atomic(&path, b"second, longer").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second, longer");

        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, ["note.md"], "no temporary file is left behind");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_keeps_the_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("write-atomic-mode");
        let path = dir.join("note.md");
        fs::write(&path, b"first").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

        write_atomic(&path, b"second").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o640);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_atomic_fails_without_folder() {
        let dir = temp_dir("write-atomic-missing");
        let path = dir.join("missing").join("note.md");
        assert!(matches!(
            write_atomic(&path, b"x"),
            Err(Error::FileWrite { path: p, .. }) if p == path
        ));
        fs::remove_dir_all(&dir).unwrap();
    }
}
