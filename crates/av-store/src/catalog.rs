//! The catalog: a row per file of a vault's index, its text as the app
//! holds it and the words a search reads (`vault.md#search`), in an SQLite
//! file per vault outside it. A text read from its file once it had settled
//! keeps the file's stamp: it is cached, and stands for the file while the
//! two stamps agree; the caller compares them.

use std::collections::HashSet;
use std::ffi::{CStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use av_utils::error::Result;
use av_utils::fnv::fnv1a_64;

use crate::sqlite::{self, Db};

/// The version of what a row holds, part of the file's name: another
/// version is another file, read afresh, never migrated. The app makes the
/// words (`search::words_of`), so a change to how is another version too.
pub const FORMAT: u32 = 2;

/// A file read sooner than this after its last change keeps no stamp: a
/// second change within the same clock tick and of the same size keeps the
/// stamp, and the row's text would come back on every start.
pub const SETTLED: Duration = Duration::from_secs(2);

const SCHEMA: &CStr = c"
    PRAGMA busy_timeout = 2000;
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS files(
        local TEXT PRIMARY KEY,
        modified INTEGER,
        size INTEGER,
        text TEXT NOT NULL
    );";

/// A row's words, under its rowid in `files`. In trigrams, so a word is
/// found inside a longer one; without case, but with its diacritics.
const WORDS: &CStr = c"CREATE VIRTUAL TABLE IF NOT EXISTS words
    USING fts5(title, labels, body, tokenize = 'trigram')";

const FORGET_WORDS: &CStr =
    c"DELETE FROM words WHERE rowid = (SELECT rowid FROM files WHERE local = ?1)";

// ----------------------------------------------------------------------------
/// A file's text for its row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    /// The path below the vault's root.
    pub local: String,
    /// As the app holds it: as read, its BOM and line endings kept, or as
    /// the app wrote it.
    pub text: String,
    /// For a text read from the file: the file's stamp then, and when.
    pub read: Option<Read>,
    pub words: Words,
}

// ----------------------------------------------------------------------------
/// What a search reads of a file: its title, its labels and its text, a
/// line each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Words {
    pub title: String,
    pub labels: String,
    pub body: String,
}

// ----------------------------------------------------------------------------
/// A file's stamp when its text was read, and the time of the read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Read {
    pub modified: SystemTime,
    pub size: u64,
    pub at: SystemTime,
}

// ----------------------------------------------------------------------------
/// A row with its file's stamp: a text a scan may take instead of reading
/// the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedFile {
    pub local: String,
    pub modified: SystemTime,
    pub size: u64,
    pub text: String,
}

// ----------------------------------------------------------------------------
/// One vault's catalog, open. Not `Send`: it stays on the thread that opened
/// it.
pub struct Catalog {
    db: Db,
}

impl Catalog {
    // ------------------------------------------------------------------------
    /// Open the catalog of the vault at `root` in `dir`, made if missing.
    /// `rebuild`, or another vault's file under the same name, empties it; a
    /// file that isn't a database, or a damaged one, is made anew. The
    /// vault's files of other formats are removed.
    pub fn open(dir: &Path, root: &Path, rebuild: bool) -> Result<Catalog> {
        fs::create_dir_all(dir)?;
        remove_other_formats(dir, &name_hash(root));
        let path = path_of(dir, root);
        match open(&path, root, rebuild) {
            Err(err) if sqlite::is_broken(&err) => {
                log::warn!("{}: {err}; made anew", path.display());
                remove(&path)?;
                open(&path, root, rebuild)
            }
            opened => opened,
        }
    }

    // ------------------------------------------------------------------------
    /// The rows with a stamp; one that doesn't read back (a path or a text
    /// that isn't UTF-8, a time out of range) is left out, so its file is
    /// read again.
    pub fn cached(&self) -> Result<Vec<CachedFile>> {
        let mut select = self.db.prepare(
            c"SELECT local, modified, size, text FROM files
              WHERE modified IS NOT NULL AND size IS NOT NULL",
        )?;
        let mut files = Vec::new();
        while select.step()? {
            let (Some(local), Some(modified), Ok(size), Some(text)) = (
                select.column_text(0),
                from_nanos(select.column_i64(1)),
                u64::try_from(select.column_i64(2)),
                select.column_text(3),
            ) else {
                continue;
            };
            files.push(CachedFile {
                local,
                modified,
                size,
                text,
            });
        }
        Ok(files)
    }

    // ------------------------------------------------------------------------
    /// The paths of every row.
    pub fn locals(&self) -> Result<Vec<String>> {
        let mut select = self.db.prepare(c"SELECT local FROM files")?;
        let mut locals = Vec::new();
        while select.step()? {
            locals.extend(select.column_text(0));
        }
        Ok(locals)
    }

    // ------------------------------------------------------------------------
    /// Replace the rows of `files`' paths with their words, in one
    /// transaction. A read keeps its stamp if the file had settled by then
    /// (`SETTLED`) and the time fits nanoseconds in an `i64`.
    pub fn put(&mut self, files: &[FileRow]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        self.transaction(|db| {
            // An update keeps the row's rowid, which its words go by; a
            // replace would give it a new one.
            let mut upsert = db.prepare(
                c"INSERT INTO files(local, modified, size, text) VALUES(?1, ?2, ?3, ?4)
                  ON CONFLICT(local) DO UPDATE SET
                  modified = excluded.modified, size = excluded.size, text = excluded.text",
            )?;
            let mut forget_words = db.prepare(FORGET_WORDS)?;
            let mut insert_words = db.prepare(
                c"INSERT INTO words(rowid, title, labels, body)
                  SELECT rowid, ?2, ?3, ?4 FROM files WHERE local = ?1",
            )?;
            for file in files {
                upsert.bind_text(1, &file.local)?;
                match file.read.as_ref().and_then(stamp_of) {
                    Some((modified, size)) => {
                        upsert.bind_i64(2, modified)?;
                        upsert.bind_i64(3, size)?;
                    }
                    None => {
                        upsert.bind_null(2)?;
                        upsert.bind_null(3)?;
                    }
                }
                upsert.bind_text(4, &file.text)?;
                upsert.step()?;
                upsert.reset()?;
                forget_words.bind_text(1, &file.local)?;
                forget_words.step()?;
                forget_words.reset()?;
                insert_words.bind_text(1, &file.local)?;
                insert_words.bind_text(2, &file.words.title)?;
                insert_words.bind_text(3, &file.words.labels)?;
                insert_words.bind_text(4, &file.words.body)?;
                insert_words.step()?;
                insert_words.reset()?;
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------------------
    /// Drop the rows of `locals` with their words, in one transaction.
    pub fn forget<'a>(&mut self, locals: impl IntoIterator<Item = &'a str>) -> Result<()> {
        self.transaction(|db| {
            let mut forget_words = db.prepare(FORGET_WORDS)?;
            let mut delete = db.prepare(c"DELETE FROM files WHERE local = ?1")?;
            for local in locals {
                forget_words.bind_text(1, local)?;
                forget_words.step()?;
                forget_words.reset()?;
                delete.bind_text(1, local)?;
                delete.step()?;
                delete.reset()?;
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------------------
    /// Drop every row whose path isn't one of `locals`.
    pub fn keep_only(&mut self, locals: &HashSet<&str>) -> Result<()> {
        let gone: Vec<String> = self
            .locals()?
            .into_iter()
            .filter(|local| !locals.contains(local.as_str()))
            .collect();
        self.forget(gone.iter().map(String::as_str))
    }

    // ------------------------------------------------------------------------
    /// Hand `each` the path and the words of every row that may hold all of
    /// `terms` (lower case): the terms of 3 characters or more narrow the
    /// rows by their trigrams, shorter ones are left to `each`, which returns
    /// whether to go on.
    pub fn search(
        &self,
        terms: &[String],
        mut each: impl FnMut(String, Words) -> bool,
    ) -> Result<()> {
        let query = match_query(terms);
        let mut select = if query.is_empty() {
            self.db.prepare(
                c"SELECT files.local, words.title, words.labels, words.body
                  FROM words JOIN files ON files.rowid = words.rowid",
            )?
        } else {
            let mut select = self.db.prepare(
                c"SELECT files.local, words.title, words.labels, words.body
                  FROM words JOIN files ON files.rowid = words.rowid WHERE words MATCH ?1",
            )?;
            select.bind_text(1, &query)?;
            select
        };
        while select.step()? {
            let (Some(local), Some(title), Some(labels), Some(body)) = (
                select.column_text(0),
                select.column_text(1),
                select.column_text(2),
                select.column_text(3),
            ) else {
                continue;
            };
            if !each(
                local,
                Words {
                    title,
                    labels,
                    body,
                },
            ) {
                break;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------------
    /// The vault's path the catalog was made for.
    fn root(&self) -> Result<Option<String>> {
        let mut select = self
            .db
            .prepare(c"SELECT value FROM meta WHERE key = 'root'")?;
        Ok(if select.step()? {
            select.column_text(0)
        } else {
            None
        })
    }

    // ------------------------------------------------------------------------
    /// Run `f` in a write transaction: committed if it succeeds, rolled back
    /// if not. Another instance writing makes it wait (`busy_timeout`).
    fn transaction(&mut self, f: impl FnOnce(&Db) -> Result<()>) -> Result<()> {
        self.db.exec(c"BEGIN IMMEDIATE")?;
        let done = f(&self.db).and_then(|()| self.db.exec(c"COMMIT"));
        if done.is_err() {
            let _ = self.db.exec(c"ROLLBACK");
        }
        done
    }
}

// ----------------------------------------------------------------------------
/// Whether a file last changed at `modified` had settled by `at`, so that
/// its next change changes its stamp (`SETTLED`).
pub fn settled(modified: SystemTime, at: SystemTime) -> bool {
    at.duration_since(modified).is_ok_and(|age| age >= SETTLED)
}

// ----------------------------------------------------------------------------
/// The FTS5 query that requires each of `terms` of 3 characters or more,
/// as a string, so nothing in it is read as an operator; empty if none is.
fn match_query(terms: &[String]) -> String {
    let strings: Vec<String> = terms
        .iter()
        .filter(|term| term.chars().count() >= 3)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect();
    strings.join(" AND ")
}

// ----------------------------------------------------------------------------
/// The stamp `read` keeps, as stored: none for a file that hadn't settled.
fn stamp_of(read: &Read) -> Option<(i64, i64)> {
    if !settled(read.modified, read.at) {
        return None;
    }
    Some((to_nanos(read.modified)?, i64::try_from(read.size).ok()?))
}

// ----------------------------------------------------------------------------
/// Open the catalog at `path` for the vault at `root`.
fn open(path: &Path, root: &Path, rebuild: bool) -> Result<Catalog> {
    let db = Db::open(path)?;
    db.exec(SCHEMA)?;
    db.exec(WORDS)?;
    let mut catalog = Catalog { db };
    let root = root.to_string_lossy();
    if rebuild || catalog.root()?.as_deref() != Some(&*root) {
        catalog.transaction(|db| {
            // Dropped and made again: FTS5 would delete its rows one by one.
            db.exec(c"DELETE FROM files; DROP TABLE words;")?;
            db.exec(WORDS)?;
            let mut set =
                db.prepare(c"INSERT OR REPLACE INTO meta(key, value) VALUES('root', ?1)")?;
            set.bind_text(1, &root)?;
            set.step()?;
            Ok(())
        })?;
    }
    Ok(catalog)
}

// ----------------------------------------------------------------------------
/// The catalog's file of the vault at `root` in `dir`.
fn path_of(dir: &Path, root: &Path) -> PathBuf {
    dir.join(format!("{}-v{FORMAT}.db", name_hash(root)))
}

// ----------------------------------------------------------------------------
/// The start of the vault's file names: a hash of its path, in hex.
fn name_hash(root: &Path) -> String {
    format!("{:016x}", fnv1a_64(root.as_os_str().as_encoded_bytes()))
}

// ----------------------------------------------------------------------------
/// The format in `name` if it is one of the vault `hash`'s files:
/// `<hash>-v<format>.db`, and its `-wal` and `-shm`.
fn format_of(name: &str, hash: &str) -> Option<u32> {
    let rest = name.strip_prefix(hash)?.strip_prefix("-v")?;
    let db = rest
        .strip_suffix("-wal")
        .or_else(|| rest.strip_suffix("-shm"))
        .unwrap_or(rest);
    let digits = db.strip_suffix(".db")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

// ----------------------------------------------------------------------------
/// Remove the vault `hash`'s files of other formats. One another instance
/// holds open stays: on Windows the removal fails, on Linux that instance
/// keeps the file it has.
fn remove_other_formats(dir: &Path, hash: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let format = name.to_str().and_then(|name| format_of(name, hash));
        if format.is_none_or(|format| format == FORMAT) {
            continue;
        }
        let path = entry.path();
        match fs::remove_file(&path) {
            Ok(()) => log::info!("{}: removed, of another format", path.display()),
            Err(err) => log::warn!("{}: {err}", path.display()),
        }
    }
}

// ----------------------------------------------------------------------------
/// Remove the catalog's file at `path` with its `-wal` and `-shm`, these
/// first: a `-wal` left behind would be replayed into the new file.
fn remove(path: &Path) -> io::Result<()> {
    for suffix in ["-wal", "-shm", ""] {
        let mut name = OsString::from(path.as_os_str());
        name.push(suffix);
        match fs::remove_file(&name) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
            _ => {}
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------------
/// `time` as nanoseconds since the Unix epoch, negative before it; `None`
/// beyond an `i64`'s (before 1678 or after 2262).
fn to_nanos(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_nanos()).ok(),
        Err(before) => i64::try_from(before.duration().as_nanos())
            .ok()
            .map(|nanos| -nanos),
    }
}

// ----------------------------------------------------------------------------
fn from_nanos(nanos: i64) -> Option<SystemTime> {
    let span = Duration::from_nanos(nanos.unsigned_abs());
    if nanos < 0 {
        UNIX_EPOCH.checked_sub(span)
    } else {
        UNIX_EPOCH.checked_add(span)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    /// An empty folder for the test `name`'s catalogs.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("av-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    // ------------------------------------------------------------------------
    /// The row of `local`'s `text`, read an hour after the file's change at
    /// `modified`: settled.
    fn read(local: &str, text: &str, modified: SystemTime) -> FileRow {
        FileRow {
            local: local.to_string(),
            text: text.to_string(),
            read: Some(Read {
                modified,
                size: text.len() as u64,
                at: modified + Duration::from_secs(3600),
            }),
            words: Words::default(),
        }
    }

    // ------------------------------------------------------------------------
    /// What `read` caches.
    fn cached(row: &FileRow) -> CachedFile {
        let read = row.read.unwrap();
        CachedFile {
            local: row.local.clone(),
            modified: read.modified,
            size: read.size,
            text: row.text.clone(),
        }
    }

    // ------------------------------------------------------------------------
    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    // ------------------------------------------------------------------------
    fn sorted<T: Ord>(mut items: Vec<T>) -> Vec<T> {
        items.sort();
        items
    }

    // ------------------------------------------------------------------------
    fn by_local(mut files: Vec<CachedFile>) -> Vec<CachedFile> {
        files.sort_by(|a, b| a.local.cmp(&b.local));
        files
    }

    // ------------------------------------------------------------------------
    #[test]
    fn cached_files_come_back_as_they_were() {
        let (dir, root) = (temp_dir("round-trip"), Path::new("/vault"));
        let mut catalog = Catalog::open(&dir, root, false).unwrap();
        assert!(catalog.cached().unwrap().is_empty());

        let a = read(
            "Notes/a.md",
            "\u{feff}---\r\ntitle: K\u{f6}ln\r\n---\r\nText\r\n",
            at(1),
        );
        let b = read("Notes/b.md", "B", at(2));
        catalog
            .put(&[a.clone(), b, read("c.md", "C", at(3))])
            .unwrap();
        let b = read("Notes/b.md", "B again", at(4));
        catalog.put(std::slice::from_ref(&b)).unwrap();
        catalog.forget(["c.md", "never.md"]).unwrap();
        drop(catalog);

        let catalog = Catalog::open(&dir, root, false).unwrap();
        assert_eq!(
            by_local(catalog.cached().unwrap()),
            [cached(&a), cached(&b)]
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn times_come_back_exactly() {
        let (dir, root) = (temp_dir("times"), Path::new("/vault"));
        let mut catalog = Catalog::open(&dir, root, false).unwrap();
        let written = dir.join("written.md");
        fs::write(&written, "x").unwrap();
        let real = fs::metadata(&written).unwrap().modified().unwrap();
        let before_1970 = UNIX_EPOCH - Duration::from_millis(1500);
        let after_2262 = at(300 * 365 * 24 * 3600);
        let rows = [read("real.md", "x", real), read("old.md", "x", before_1970)];
        catalog.put(&rows).unwrap();
        catalog.put(&[read("far.md", "x", after_2262)]).unwrap();
        drop(catalog);

        let catalog = Catalog::open(&dir, root, false).unwrap();
        let expected: Vec<CachedFile> = rows.iter().map(cached).collect();
        assert_eq!(by_local(catalog.cached().unwrap()), by_local(expected));
        assert_eq!(
            sorted(catalog.locals().unwrap()),
            ["far.md", "old.md", "real.md"],
            "a time out of range keeps the row, without a stamp"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn only_a_settled_read_is_cached() {
        let (dir, root) = (temp_dir("settled"), Path::new("/vault"));
        let mut catalog = Catalog::open(&dir, root, false).unwrap();
        let settled = read("settled.md", "S", at(10));
        let mut unsettled = read("unsettled.md", "U", at(10));
        unsettled.read.as_mut().unwrap().at = at(11);
        let written = FileRow {
            local: "written.md".to_string(),
            text: "W".to_string(),
            read: None,
            words: Words::default(),
        };
        catalog.put(&[settled.clone(), unsettled, written]).unwrap();

        assert_eq!(catalog.cached().unwrap(), [cached(&settled)]);
        assert_eq!(
            sorted(catalog.locals().unwrap()),
            ["settled.md", "unsettled.md", "written.md"]
        );

        // A settled read of the same text caches it again.
        catalog.put(&[read("unsettled.md", "U", at(10))]).unwrap();
        assert_eq!(catalog.cached().unwrap().len(), 2);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_file_has_settled_two_seconds_after_its_change() {
        let now = SystemTime::now();
        assert!(!settled(now, now));
        assert!(!settled(now - Duration::from_secs(1), now));
        assert!(settled(now - Duration::from_secs(3), now));
        assert!(
            !settled(now + Duration::from_secs(3), now),
            "a time to come"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn keep_only_drops_the_other_rows() {
        let (dir, root) = (temp_dir("keep-only"), Path::new("/vault"));
        let mut catalog = Catalog::open(&dir, root, false).unwrap();
        let rows: Vec<FileRow> = ["a.md", "b.md", "c.md"]
            .iter()
            .map(|local| read(local, "x", at(1)))
            .collect();
        catalog.put(&rows).unwrap();
        catalog
            .keep_only(&HashSet::from(["a.md", "c.md", "new.md"]))
            .unwrap();
        assert_eq!(sorted(catalog.locals().unwrap()), ["a.md", "c.md"]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_rebuild_or_another_vault_starts_empty() {
        let (dir, root) = (temp_dir("rebuild"), Path::new("/vault"));
        let mut catalog = Catalog::open(&dir, root, false).unwrap();
        catalog.put(&[read("a.md", "A", at(1))]).unwrap();
        drop(catalog);
        let catalog = Catalog::open(&dir, root, true).unwrap();
        assert!(catalog.locals().unwrap().is_empty());
        drop(catalog);

        // Another vault whose path hashes to the same name.
        let path = path_of(&dir, root);
        let mut catalog = open(&path, root, false).unwrap();
        catalog.put(&[read("a.md", "A", at(1))]).unwrap();
        drop(catalog);
        let catalog = open(&path, Path::new("/other"), false).unwrap();
        assert!(catalog.locals().unwrap().is_empty());
        drop(catalog);
        let catalog = open(&path, root, false).unwrap();
        assert!(
            catalog.locals().unwrap().is_empty(),
            "the file is the other vault's now"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_file_that_is_no_database_is_made_anew() {
        let (dir, root) = (temp_dir("garbage"), Path::new("/vault"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(path_of(&dir, root), vec![b'x'; 4096]).unwrap();

        let mut catalog = Catalog::open(&dir, root, false).unwrap();
        assert!(catalog.locals().unwrap().is_empty());
        let a = read("a.md", "A", at(1));
        catalog.put(std::slice::from_ref(&a)).unwrap();
        drop(catalog);
        let catalog = Catalog::open(&dir, root, false).unwrap();
        assert_eq!(catalog.cached().unwrap(), [cached(&a)]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn files_of_other_formats_are_removed() {
        let (dir, root) = (temp_dir("formats"), Path::new("/vault"));
        fs::create_dir_all(&dir).unwrap();
        let hash = name_hash(root);
        let other = name_hash(Path::new("/other"));
        let names = [
            format!("{hash}-v0.db"),
            format!("{hash}-v0.db-wal"),
            format!("{hash}-v0.db-shm"),
            format!("{other}-v0.db"),
            format!("{hash}-vectors.db"),
        ];
        for name in &names {
            fs::write(dir.join(name), "x").unwrap();
        }

        let _ = Catalog::open(&dir, root, false).unwrap();
        let kept: Vec<bool> = names.iter().map(|n| dir.join(n).exists()).collect();
        assert_eq!(kept, [false, false, false, true, true]);
        assert!(path_of(&dir, root).exists());
        assert_eq!(format_of(&format!("{hash}-v12.db-wal"), &hash), Some(12));
        assert_eq!(format_of(&format!("{hash}-v.db"), &hash), None);
        assert_eq!(format_of(&format!("{hash}-v1.db.bak"), &hash), None);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn two_connections_share_the_file() {
        let (dir, root) = (temp_dir("shared"), Path::new("/vault"));
        let mut first = Catalog::open(&dir, root, false).unwrap();
        first.put(&[read("old.md", "Old", at(1))]).unwrap();
        let second = Catalog::open(&dir, root, true).unwrap();
        assert!(second.locals().unwrap().is_empty());
        let a = read("a.md", "A", at(2));
        first.put(std::slice::from_ref(&a)).unwrap();

        assert_eq!(second.cached().unwrap(), [cached(&a)]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_folder_that_cant_be_made_is_an_error() {
        let dir = temp_dir("no-folder");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("file"), "x").unwrap();
        let below_a_file = dir.join("file").join("catalog");
        assert!(Catalog::open(&below_a_file, Path::new("/vault"), false).is_err());
    }

    // ------------------------------------------------------------------------
    fn words(title: &str, labels: &str, body: &str) -> Words {
        Words {
            title: title.to_string(),
            labels: labels.to_string(),
            body: body.to_string(),
        }
    }

    // ------------------------------------------------------------------------
    /// `local`'s settled read with `words`.
    fn worded(local: &str, words: Words) -> FileRow {
        FileRow {
            words,
            ..read(local, "x", at(1))
        }
    }

    // ------------------------------------------------------------------------
    /// The paths a search of `catalog` for `terms` hands over, sorted.
    fn found(catalog: &Catalog, terms: &[&str]) -> Vec<String> {
        let terms: Vec<String> = terms.iter().map(|term| term.to_string()).collect();
        let mut locals = Vec::new();
        catalog
            .search(&terms, |local, _| {
                locals.push(local);
                true
            })
            .unwrap();
        sorted(locals)
    }

    // ------------------------------------------------------------------------
    #[test]
    fn words_are_found_inside_longer_ones_in_any_column_and_case() {
        let (dir, root) = (temp_dir("search"), Path::new("/vault"));
        let mut catalog = Catalog::open(&dir, root, false).unwrap();
        catalog
            .put(&[
                worded(
                    "a.md",
                    words("Stromrechnung", "Haus", "Bitte bis Freitag zahlen"),
                ),
                worded("b.md", words("Brief", "", "Die RECHNUNG kommt")),
                worded("c.md", words("Notiz", "Rechnung", "")),
            ])
            .unwrap();

        assert_eq!(found(&catalog, &["rechnung"]), ["a.md", "b.md", "c.md"]);
        assert_eq!(found(&catalog, &["rechnung", "freitag"]), ["a.md"]);
        assert_eq!(found(&catalog, &["haus", "zahlen"]), ["a.md"]);
        assert!(found(&catalog, &["rechnungen"]).is_empty());
        // Short terms leave every row to the caller.
        assert_eq!(found(&catalog, &["ab"]), ["a.md", "b.md", "c.md"]);
        assert_eq!(found(&catalog, &["ab", "brief"]), ["b.md"]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_term_is_found_as_written_whatever_it_holds() {
        let (dir, root) = (temp_dir("search-quoted"), Path::new("/vault"));
        let mut catalog = Catalog::open(&dir, root, false).unwrap();
        let title = "Say \"hi\" AND-more *now* (near)";
        catalog
            .put(&[worded("a.md", words(title, "", ""))])
            .unwrap();
        for term in ["\"hi\"", "and-more", "*now*", "and", "(near)", "near("] {
            let expected: &[&str] = if term == "near(" { &[] } else { &["a.md"] };
            assert_eq!(found(&catalog, &[term]), expected, "{term}");
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_put_replaces_the_words_and_forget_and_keep_only_drop_them() {
        let (dir, root) = (temp_dir("search-replaced"), Path::new("/vault"));
        let mut catalog = Catalog::open(&dir, root, false).unwrap();
        catalog
            .put(&[worded("a.md", words("Old title", "", "old body"))])
            .unwrap();
        catalog
            .put(&[worded("a.md", words("New title", "", "new body"))])
            .unwrap();
        assert!(found(&catalog, &["old"]).is_empty());
        assert_eq!(found(&catalog, &["new", "body"]), ["a.md"]);

        catalog.forget(["a.md"]).unwrap();
        assert!(found(&catalog, &["new"]).is_empty());
        catalog
            .put(&[
                worded("b.md", words("Same", "", "")),
                worded("c.md", words("Same", "", "")),
            ])
            .unwrap();
        catalog.keep_only(&HashSet::from(["c.md"])).unwrap();
        assert_eq!(found(&catalog, &["same"]), ["c.md"]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_rebuild_leaves_no_words() {
        let (dir, root) = (temp_dir("search-rebuild"), Path::new("/vault"));
        let mut catalog = Catalog::open(&dir, root, false).unwrap();
        catalog
            .put(&[worded("a.md", words("Title", "", ""))])
            .unwrap();
        drop(catalog);
        let catalog = Catalog::open(&dir, root, true).unwrap();
        assert!(found(&catalog, &["title"]).is_empty());
        assert!(found(&catalog, &["ti"]).is_empty());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_search_stops_when_told() {
        let (dir, root) = (temp_dir("search-stop"), Path::new("/vault"));
        let mut catalog = Catalog::open(&dir, root, false).unwrap();
        let rows: Vec<FileRow> = ["a.md", "b.md", "c.md"]
            .iter()
            .map(|local| worded(local, words("Same", "", "")))
            .collect();
        catalog.put(&rows).unwrap();
        let mut seen = 0;
        let terms = ["same".to_string()];
        catalog
            .search(&terms, |_, _| {
                seen += 1;
                false
            })
            .unwrap();
        assert_eq!(seen, 1);
    }
}
