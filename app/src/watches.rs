//! Folder watches (import.md#watch-files): one YAML file per watch
//! in the open vault's `.avelor/watches/`, its block named `folder-watch:`. A
//! watch says which folder on this PC to read, which files to take from it
//! and how to read them. The open vault's watches are a `Watches`. Also
//! lists the files a watch takes (`list_files`); importing them is up to the
//! importer of the watch's type (`scans` for images). The commands:
//! `folders`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

use crate::ai_accounts::ModelChoice;
use crate::config::is_false;
use crate::config_files::{free_stem, load_config_files, remove_if_exists, write_yaml};
use crate::sys;

// ----------------------------------------------------------------------------
/// What a watch takes, and so how it reads a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WatchKind {
    /// Scanned pages, read by OCR; several images can be one document.
    Images,
}

impl WatchKind {
    // ------------------------------------------------------------------------
    /// The name used in files and messages.
    pub fn as_str(self) -> &'static str {
        match self {
            WatchKind::Images => "images",
        }
    }
}

// ----------------------------------------------------------------------------
/// How the images of an `images` watch become documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Collect {
    /// Images taken within two minutes of each other are the pages of one
    /// document.
    Together,
    /// One document per image.
    Single,
}

// ----------------------------------------------------------------------------
/// A watch's block. Unknown keys are kept, so hand-added settings survive
/// the app rewriting the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct WatchSettings {
    #[serde(rename = "type")]
    pub kind: WatchKind,
    /// Empty: the folder's name is shown.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// As entered; `%VAR%`s are expanded where the folder is used.
    pub folder: String,
    /// Whether files in subfolders are taken too.
    #[serde(default, skip_serializing_if = "is_false")]
    pub subfolders: bool,
    /// File name patterns (`*.jpg`); a file matching any of them is taken.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<String>,
    /// Images only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collect: Option<Collect>,
    /// The model each step that sends a file to an AI picked, by step
    /// (accounts.md#model-choice).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, ModelChoice>,
    /// Off: the watch isn't listed under Sources.
    #[serde(default = "enabled_default", skip_serializing_if = "is_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<DateTime>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

fn enabled_default() -> bool {
    true
}

fn is_enabled(enabled: &bool) -> bool {
    *enabled
}

impl WatchSettings {
    // ------------------------------------------------------------------------
    /// An enabled watch of `kind` on `folder`, with nothing else set.
    pub fn new(kind: WatchKind, folder: String) -> Self {
        Self {
            kind,
            name: String::new(),
            folder,
            subfolders: false,
            patterns: Vec::new(),
            collect: None,
            models: BTreeMap::new(),
            enabled: true,
            added: None,
            rest: BTreeMap::new(),
        }
    }

    // ------------------------------------------------------------------------
    /// The name, or the folder's if it has none.
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            folder_name(&self.folder)
        } else {
            self.name.clone()
        }
    }

    // ------------------------------------------------------------------------
    /// The folder with its `%VAR%`s expanded (on Linux `~` and `$NAME`s):
    /// where the files are.
    pub fn root(&self) -> PathBuf {
        sys::expand_env(&self.folder)
    }
}

// ----------------------------------------------------------------------------
/// The watch file's content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct WatchFile {
    pub id: Id,
    pub folder_watch: WatchSettings,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

// ----------------------------------------------------------------------------
/// A configured folder watch.
#[derive(Debug, Clone)]
pub struct Watch {
    /// The watch file.
    path: PathBuf,
    file: WatchFile,
}

// ----------------------------------------------------------------------------
/// What the page gets per watch (the `watch` event).
#[derive(Serialize)]
pub struct WatchMeta<'a> {
    pub id: Id,
    #[serde(rename = "type")]
    pub kind: WatchKind,
    /// See `WatchSettings::display_name`.
    pub name: String,
    pub folder: &'a str,
    pub subfolders: bool,
    pub patterns: &'a [String],
    pub collect: Option<Collect>,
    pub models: &'a BTreeMap<String, ModelChoice>,
    pub enabled: bool,
    pub added: Option<DateTime>,
}

impl Watch {
    // ------------------------------------------------------------------------
    /// A new, unsaved watch, added now, in the first free file of `dir`:
    /// `watch_images.yaml`, then `watch_images_2.yaml`, …
    fn new(dir: &Path, mut settings: WatchSettings) -> Self {
        let base = format!("watch_{}", settings.kind.as_str());
        let stem = free_stem(dir, &base, &["yaml"]);
        settings.added.get_or_insert_with(DateTime::now);
        Self {
            path: dir.join(format!("{stem}.yaml")),
            file: WatchFile {
                id: Id::generate(),
                folder_watch: settings,
                rest: BTreeMap::new(),
            },
        }
    }

    // ------------------------------------------------------------------------
    pub fn id(&self) -> Id {
        self.file.id
    }

    // ------------------------------------------------------------------------
    pub fn settings(&self) -> &WatchSettings {
        &self.file.folder_watch
    }

    // ------------------------------------------------------------------------
    fn settings_mut(&mut self) -> &mut WatchSettings {
        &mut self.file.folder_watch
    }

    // ------------------------------------------------------------------------
    pub fn meta(&self) -> WatchMeta<'_> {
        let settings = self.settings();
        WatchMeta {
            id: self.id(),
            kind: settings.kind,
            name: settings.display_name(),
            folder: &settings.folder,
            subfolders: settings.subfolders,
            patterns: &settings.patterns,
            collect: settings.collect,
            models: &settings.models,
            enabled: settings.enabled,
            added: settings.added,
        }
    }

    // ------------------------------------------------------------------------
    fn save(&self) -> Result<()> {
        write_yaml(&self.path, &self.file)?;
        log::info!("Saved folder watch {}", self.path.display());
        Ok(())
    }

    // ------------------------------------------------------------------------
    /// Delete the watch file; what the watch imported stays in the vault.
    fn remove(&self) -> Result<()> {
        remove_if_exists(&self.path)?;
        log::info!("Removed folder watch {}", self.path.display());
        Ok(())
    }
}

// ----------------------------------------------------------------------------
/// The open vault's folder watches: their files in `folder` (the vault's
/// `.avelor/watches`). Without a vault it is empty, and nothing can be added.
#[derive(Default)]
pub struct Watches {
    folder: PathBuf,
    list: Vec<Watch>,
}

impl Watches {
    // ------------------------------------------------------------------------
    /// The watches in `folder` (`load_all`).
    pub fn load(folder: PathBuf) -> Self {
        let list = load_all(&folder);
        Self { folder, list }
    }

    // ------------------------------------------------------------------------
    pub fn iter(&self) -> std::slice::Iter<'_, Watch> {
        self.list.iter()
    }

    // ------------------------------------------------------------------------
    /// Where the watches are kept: the open vault's `.avelor/watches`; empty
    /// without a vault.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    // ------------------------------------------------------------------------
    /// The watch `id`; `NotFound` if there is none.
    pub fn get(&self, id: Id) -> Result<&Watch> {
        self.list
            .iter()
            .find(|w| w.id() == id)
            .ok_or(Error::NotFound)
    }

    // ------------------------------------------------------------------------
    fn index(&self, id: Id) -> Result<usize> {
        self.list
            .iter()
            .position(|w| w.id() == id)
            .ok_or(Error::NotFound)
    }

    // ------------------------------------------------------------------------
    /// Save a watch of `settings`, added now, in the first free file; its
    /// id. `InvalidPath` without a vault; nothing is kept if saving fails.
    pub fn create(&mut self, settings: WatchSettings) -> Result<Id> {
        if self.folder.as_os_str().is_empty() {
            return Err(Error::InvalidPath {
                path: PathBuf::new(),
            });
        }
        let watch = Watch::new(&self.folder, settings);
        watch.save()?;
        let id = watch.id();
        self.list.push(watch);
        Ok(id)
    }

    // ------------------------------------------------------------------------
    /// Change the settings of the watch `id` with `f`, and save them; a
    /// failed save leaves them as they were.
    pub fn update(&mut self, id: Id, f: impl FnOnce(&mut WatchSettings)) -> Result<()> {
        let index = self.index(id)?;
        let mut watch = self.list[index].clone();
        f(watch.settings_mut());
        watch.save()?;
        self.list[index] = watch;
        Ok(())
    }

    // ------------------------------------------------------------------------
    /// Delete the watch `id`'s file; what it imported stays in the vault.
    pub fn remove(&mut self, id: Id) -> Result<()> {
        let index = self.index(id)?;
        self.list[index].remove()?;
        self.list.remove(index);
        Ok(())
    }
}

// ----------------------------------------------------------------------------
/// The last part of `folder` once expanded (`Scans` of
/// `%USERPROFILE%\Scans`), or `folder` itself if it has none (`C:\`).
pub fn folder_name(folder: &str) -> String {
    match sys::expand_env(folder).file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => folder.to_string(),
    }
}

// ----------------------------------------------------------------------------
/// A file in a watch's folder that the watch takes.
#[derive(Debug, Clone)]
pub struct FolderFile {
    /// The path relative to the watch's folder, as a page and `scan.origin`
    /// name it (`scan.jpg`, or `2026` + separator + `scan.jpg` in a
    /// subfolder).
    pub name: String,
    pub size: u64,
    /// When the file was made: its creation time, or its modified time if
    /// that is earlier (a copied file keeps the original's modified time).
    pub taken: DateTime,
}

impl FolderFile {
    // ------------------------------------------------------------------------
    fn new(name: String, _path: PathBuf, meta: &fs::Metadata) -> Result<Self> {
        let modified = meta.modified()?;
        let created = meta.created().unwrap_or(modified);
        Ok(Self {
            name,
            size: meta.len(),
            taken: DateTime::from_system_time(created.min(modified)),
        })
    }
}

// ----------------------------------------------------------------------------
/// Whether the file name `name` matches `pattern`: `*` stands for any
/// characters, `?` for one; letters match in any case, as on Windows.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let name: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    let (mut p, mut n) = (0, 0);
    // Where the last `*` was, and the name position it was last tried at.
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                // Let the `*` take one more character, and go on after it.
                Some((star_p, star_n)) => {
                    star = Some((star_p, star_n + 1));
                    p = star_p + 1;
                    n = star_n + 1;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

// ----------------------------------------------------------------------------
/// The files in the folder of `settings` that match its patterns (in its
/// subfolders too, if it says so), by `taken`, then by name. An unreadable
/// subfolder is logged and skipped; a missing folder is an error.
pub fn list_files(settings: &WatchSettings) -> Result<Vec<FolderFile>> {
    let root = settings.root();
    if !root.is_dir() {
        return Err(Error::FileNotFound { path: root });
    }
    let mut files = Vec::new();
    collect_files(&root, &root, settings, &mut files)?;
    files.sort_by(|a, b| (a.taken, &a.name).cmp(&(b.taken, &b.name)));
    Ok(files)
}

// ----------------------------------------------------------------------------
fn collect_files(
    root: &Path,
    dir: &Path,
    settings: &WatchSettings,
    files: &mut Vec<FolderFile>,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let meta = entry.metadata()?;
        if meta.is_dir() {
            if settings.subfolders {
                if let Err(e) = collect_files(root, &path, settings, files) {
                    log::warn!("Skipping the folder {}: {e}", path.display());
                }
            }
            continue;
        }
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if !settings.patterns.iter().any(|p| glob_match(p, &file_name)) {
            continue;
        }
        let name = path.strip_prefix(root).unwrap_or(&path);
        let name = name.to_string_lossy().into_owned();
        files.push(FolderFile::new(name, path, &meta)?);
    }
    Ok(())
}

// ----------------------------------------------------------------------------
/// The folder watches in `dir`: every `*.yaml` with a `folder-watch` block;
/// other and broken files are logged and skipped.
fn load_all(dir: &Path) -> Vec<Watch> {
    load_config_files(dir, "folder watch", load)
}

// ----------------------------------------------------------------------------
/// The watch in `path`; `None` if it isn't one (some other YAML).
fn load(path: &Path) -> Result<Option<Watch>> {
    let value: Value = serde_yaml_ng::from_str(&fs::read_to_string(path)?)?;
    if value.get("folder-watch").is_none() {
        return Ok(None);
    }
    let file: WatchFile = serde_yaml_ng::from_value(value)?;
    let settings = &file.folder_watch;
    log::info!(
        "Loaded folder watch {} ({}, {}{})",
        path.display(),
        settings.kind.as_str(),
        settings.folder,
        if settings.enabled { "" } else { ", off" }
    );
    Ok(Some(Watch {
        path: path.to_path_buf(),
        file,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCANNER: &str = r"id: 01a0e1cc-a24c-7490-82fe-e75669985b40
folder-watch:
  type: images
  name: Scanner
  folder: Scans
  patterns:
    - '*.jpg'
    - '*.png'
  steps:
    - title-date
    - keep-original
  collect: together
  added: 2026-01-03T10:00:00Z
  custom: kept
note: top-level kept
";

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("avelor-watch-test-{}", Id::generate()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ------------------------------------------------------------------------
    #[test]
    fn loads_watches_and_skips_the_rest() {
        let dir = temp_dir();
        let write = |name: &str, content: &[u8]| fs::write(dir.join(name), content).unwrap();
        write("config.yaml", br"vault: C:\nowhere");
        write("watch_images.yaml", SCANNER.as_bytes());
        write(
            "watch_images_2.yaml",
            b"id: 01a0e1cc-a24c-7490-82fe-e75669985b41\nfolder-watch:\n  type: images\n  folder: Downloads\n  enabled: false\n",
        );
        write(
            "watch_pdf.yaml",
            b"id: 01a0e1cc-a24c-7490-82fe-e75669985b45\nfolder-watch:\n  type: pdf\n  folder: Downloads\n",
        );
        write(
            "google_a.yaml",
            b"id: 01a0e1cc-a24c-7490-82fe-e75669985b42\ngoogle:\n  email: a@example.com\n",
        );
        write(
            "openai.yaml",
            b"id: 01a0e1cc-a24c-7490-82fe-e75669985b43\nopenai:\n  model: gpt-5-mini\n",
        );
        write(
            "watch_bad_id.yaml",
            b"id: nope\nfolder-watch:\n  type: text\n  folder: C:\\\n",
        );
        write(
            "watch_no_type.yaml",
            b"id: 01a0e1cc-a24c-7490-82fe-e75669985b44\nfolder-watch:\n  folder: C:\\\n",
        );
        write("broken.yaml", b"folder-watch: [unclosed\n");

        let watches = load_all(&dir);
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(
            watches.len(),
            2,
            "accounts, broken files and a type the app doesn't take are skipped"
        );
        let (scanner, downloads) = (watches[0].settings(), watches[1].settings());
        assert_eq!(scanner.kind, WatchKind::Images);
        assert_eq!(scanner.patterns, ["*.jpg", "*.png"]);
        assert_eq!(scanner.collect, Some(Collect::Together));
        assert!(scanner.enabled && !scanner.subfolders);
        assert!(!downloads.enabled);
        assert_eq!(
            downloads.display_name(),
            "Downloads",
            "no name: the folder's"
        );
        assert_eq!(watches[1].meta().name, "Downloads");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn save_keeps_unknown_keys_and_writes_only_whats_set() {
        let dir = temp_dir();
        fs::write(dir.join("watch_images.yaml"), SCANNER).unwrap();
        let mut watch = load_all(&dir).pop().unwrap();
        watch.save().unwrap();
        let unchanged = fs::read_to_string(&watch.path).unwrap();

        let settings = watch.settings_mut();
        settings.enabled = false;
        settings.subfolders = true;
        settings.name.clear();
        watch.save().unwrap();
        let changed = fs::read_to_string(&watch.path).unwrap();
        let again = load_all(&dir).pop().unwrap();
        fs::remove_dir_all(&dir).unwrap();

        assert!(unchanged.starts_with("id: "), "{unchanged}");
        assert!(
            unchanged.contains("custom: kept") && unchanged.contains("note: top-level kept"),
            "{unchanged}"
        );
        assert!(
            !unchanged.contains("enabled") && !unchanged.contains("subfolders"),
            "defaults aren't written: {unchanged}"
        );
        assert!(
            changed.contains("enabled: false") && changed.contains("subfolders: true"),
            "{changed}"
        );
        assert!(!changed.contains("name:"), "an empty name isn't written");
        assert_eq!(again.id(), watch.id());
        assert_eq!(again.settings().display_name(), "Scans");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn new_watches_take_the_first_free_file() {
        let dir = temp_dir();
        let settings = || WatchSettings::new(WatchKind::Images, "Scans".into());
        let first = Watch::new(&dir, settings());
        first.save().unwrap();
        let second = Watch::new(&dir, settings());
        first.remove().unwrap();
        let left: Vec<_> = fs::read_dir(&dir).unwrap().collect();
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(first.path, dir.join("watch_images.yaml"));
        assert_eq!(second.path, dir.join("watch_images_2.yaml"));
        assert!(first.settings().added.is_some(), "added now");
        assert!(left.is_empty(), "remove deletes the file");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_folder_is_named_by_its_last_part() {
        let scans = sys::collapse_env(&std::env::temp_dir().join("Scans"));
        assert_eq!(folder_name(&scans), "Scans");
        assert_eq!(folder_name("Scans"), "Scans");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn patterns_match_file_names_in_any_case() {
        assert!(glob_match("*.jpg", "scan_0806.JPG"));
        assert!(glob_match("*.jpg", ".jpg"));
        assert!(!glob_match("*.jpg", "scan.jpeg"));
        assert!(!glob_match("*.jpg", "scan.jpg.txt"));
        assert!(glob_match(
            "WhatsApp Chat*.txt",
            "whatsapp chat with Maya.txt"
        ));
        assert!(glob_match("scan_????.png", "scan_0806.png"));
        assert!(!glob_match("scan_????.png", "scan_806.png"));
        assert!(glob_match("*a*b*c", "xxaxxbxbxxc"));
        assert!(glob_match("*", "anything"));
        assert!(!glob_match("", "x"));
        assert!(
            glob_match("K\u{f6}ln*", "K\u{d6}LN.pdf"),
            "beyond ASCII too"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn listing_takes_matching_files_and_subfolders_if_asked() {
        let dir = temp_dir();
        fs::create_dir_all(dir.join("2026")).unwrap();
        for (name, content) in [
            ("b.jpg", "bb"),
            ("a.PNG", "a"),
            ("note.txt", "n"),
            ("2026/c.jpg", "ccc"),
        ] {
            fs::write(dir.join(name), content).unwrap();
        }
        let mut settings = WatchSettings::new(WatchKind::Images, dir.to_string_lossy().into());
        settings.patterns = vec!["*.jpg".into(), "*.png".into()];

        let top = list_files(&settings).unwrap();
        settings.subfolders = true;
        let all = list_files(&settings).unwrap();
        settings.folder = dir.join("gone").to_string_lossy().into();
        let missing = list_files(&settings);
        fs::remove_dir_all(&dir).unwrap();

        let names = |files: &[FolderFile]| {
            let mut names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
            names.sort();
            names
        };
        assert_eq!(names(&top), ["a.PNG", "b.jpg"]);
        let sub = Path::new("2026")
            .join("c.jpg")
            .to_string_lossy()
            .into_owned();
        assert_eq!(names(&all), [sub.as_str(), "a.PNG", "b.jpg"]);
        let c = all.iter().find(|f| f.name == sub).unwrap();
        assert_eq!(c.size, 3);
        assert!(c.taken <= DateTime::now());
        assert!(matches!(missing, Err(Error::FileNotFound { .. })));
    }
}
