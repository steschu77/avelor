//! What Avelor keeps on this PC outside the vaults (`architecture.md#storage`):
//! its settings, the sign-ins and keys, the vaults' catalogs, the web view's
//! files and the logs, as Settings shows them (ui.md#settings), and their
//! removal. Each is known by its name, the web view's by their folder: a file
//! of another name isn't counted, and stays.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use av_store::catalog::is_catalog_file;
use av_utils::id::Id;
use av_utils::logger::is_run_log;
use serde::Serialize;

use crate::app::App;
use crate::close;
use crate::config;
use crate::google::oauth;
use crate::protocol::Req;
use crate::sys;
use crate::texts::{self, Text};

/// How long a removal tries again: the web view's processes end once the
/// window is gone, and hold their files until then.
const REMOVE_WAIT: Duration = Duration::from_secs(10);
const REMOVE_AGAIN: Duration = Duration::from_millis(200);
/// How many of the files left a removal names.
const LEFT_NAMED: usize = 8;

// ----------------------------------------------------------------------------
/// The app's folders (`sys::Paths`): `web` are the web view's, its own as a
/// whole.
#[derive(Clone)]
pub struct Folders {
    pub config: PathBuf,
    pub catalog: PathBuf,
    pub log: PathBuf,
    pub web: Vec<PathBuf>,
}

// ----------------------------------------------------------------------------
/// What a file of the app's is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum What {
    Settings,
    GoogleSignIn,
    AiKey,
    GoogleClient,
    Catalog,
    Log,
}

// ----------------------------------------------------------------------------
/// What is kept: the folder of the settings, each kind's bytes, and how many
/// Google sign-ins and AI keys there are, the user's own Google client among
/// the secrets too.
#[derive(Default, Serialize)]
struct Kept {
    folder: PathBuf,
    settings: u64,
    secrets: u64,
    google: usize,
    ai: usize,
    client: bool,
    catalog: u64,
    web: u64,
    log: u64,
}

impl Folders {
    // ------------------------------------------------------------------------
    pub fn of(paths: &sys::Paths) -> Self {
        Self {
            config: paths.config_dir(),
            catalog: paths.catalog_dir(),
            log: paths.log_dir(),
            web: paths.web_dirs(),
        }
    }

    // ------------------------------------------------------------------------
    /// A test's, in `dir`.
    #[cfg(test)]
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            config: dir.to_path_buf(),
            catalog: dir.join("catalog"),
            log: dir.join("log"),
            web: vec![dir.join("web")],
        }
    }

    // ------------------------------------------------------------------------
    /// The files of the app's, but for the web view's, each with what it is.
    fn files(&self) -> Vec<(What, PathBuf)> {
        let catalog = |name: &str| is_catalog_file(name).then_some(What::Catalog);
        let log = |name: &str| is_run_log(name).then_some(What::Log);
        let mut files = files_in(&self.config, config_file);
        files.extend(files_in(&self.catalog, catalog));
        files.extend(files_in(&self.log, log));
        files
    }

    // ------------------------------------------------------------------------
    fn kept(&self) -> Kept {
        let mut kept = Kept {
            folder: self.config.clone(),
            web: self.web.iter().map(|dir| bytes_under(dir)).sum(),
            ..Kept::default()
        };
        for (what, path) in self.files() {
            let bytes = fs::metadata(path).map_or(0, |file| file.len());
            match what {
                What::Settings => kept.settings += bytes,
                What::Catalog => kept.catalog += bytes,
                What::Log => kept.log += bytes,
                What::GoogleSignIn | What::AiKey | What::GoogleClient => kept.secrets += bytes,
            }
            kept.google += usize::from(what == What::GoogleSignIn);
            kept.ai += usize::from(what == What::AiKey);
            kept.client |= what == What::GoogleClient;
        }
        kept
    }

    // ------------------------------------------------------------------------
    /// Remove what is the app's, again until `wait` is over while some of it
    /// can't be, and the folders that are empty then. Returns what is left.
    fn remove(&self, wait: Duration) -> Vec<PathBuf> {
        let end = Instant::now() + wait;
        loop {
            let left = self.remove_once();
            if left.is_empty() || Instant::now() >= end {
                return left;
            }
            std::thread::sleep(REMOVE_AGAIN);
        }
    }

    // ------------------------------------------------------------------------
    fn remove_once(&self) -> Vec<PathBuf> {
        for (_, path) in self.files() {
            let _ = fs::remove_file(path);
        }
        for dir in &self.web {
            let _ = fs::remove_dir_all(dir);
        }
        // Each with the folder of the app's it is in, and that of the
        // settings, unless a file of another name is there.
        for dir in [&self.catalog, &self.log].into_iter().chain(&self.web) {
            let _ = fs::remove_dir(dir);
            let _ = dir.parent().map(fs::remove_dir);
        }
        let _ = fs::remove_dir(&self.config);

        let files = self.files().into_iter().map(|(_, path)| path);
        let web = self.web.iter().filter(|dir| dir.exists()).cloned();
        files.chain(web).collect()
    }
}

// ----------------------------------------------------------------------------
/// What the file `name` of the config folder is, one a write left half done
/// (`.{name}.tmp`, `av_utils::fs`) too; none for another's.
fn config_file(name: &str) -> Option<What> {
    let half_done = name.strip_prefix('.').and_then(|n| n.strip_suffix(".tmp"));
    let name = half_done.unwrap_or(name);
    if config::FILES.contains(&name) {
        return Some(What::Settings);
    }
    if name == oauth::FILE {
        return Some(What::GoogleClient);
    }
    let (id, kind) = name.split_once('.')?;
    Id::parse(id).ok()?;
    match kind {
        "token" => Some(What::GoogleSignIn),
        "key" => Some(What::AiKey),
        _ => None,
    }
}

// ----------------------------------------------------------------------------
/// The files in `dir` that `what_of` knows by their name.
fn files_in(dir: &Path, what_of: impl Fn(&str) -> Option<What>) -> Vec<(What, PathBuf)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let files = entries.flatten().filter(|entry| !is_dir(entry));
    files
        .filter_map(|entry| Some((what_of(entry.file_name().to_str()?)?, entry.path())))
        .collect()
}

// ----------------------------------------------------------------------------
/// The bytes of the files under `dir`; a link isn't followed.
fn bytes_under(dir: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let bytes = |entry: fs::DirEntry| match is_dir(&entry) {
        true => bytes_under(&entry.path()),
        false => entry.metadata().map_or(0, |file| file.len()),
    };
    entries.flatten().map(bytes).sum()
}

fn is_dir(entry: &fs::DirEntry) -> bool {
    entry.file_type().is_ok_and(|kind| kind.is_dir())
}

// ----------------------------------------------------------------------------
/// `kept {}`: what Avelor keeps on this PC, counted on a worker.
pub fn kept(app: &mut App, req: Req) {
    let folders = app.folders.clone();
    app.run(move |_| folders.kept(), |_, kept| req.ok(kept));
}

// ----------------------------------------------------------------------------
/// `reveal_data {}`: the folder of the settings in Explorer.
pub fn reveal_data(app: &mut App, req: Req) {
    let folder = app.folders.config.clone();
    crate::api::reveal(app, req, folder, Text::new("settings.show_failed"));
}

// ----------------------------------------------------------------------------
/// `remove_data {}`: the window closes as its Close does, its edits saved
/// first, and what Avelor keeps on this PC is removed then (`remove`); a
/// close that doesn't come about leaves both. Answers `null`.
pub fn remove_data(app: &mut App, req: Req) {
    close::and_remove(app);
    req.ok(());
}

// ----------------------------------------------------------------------------
/// The window is gone: remove what Avelor keeps on this PC. What is left is
/// named in a box.
pub fn remove(folders: &Folders) {
    let left = folders.remove(REMOVE_WAIT);
    if left.is_empty() {
        return;
    }
    let named = left.iter().take(LEFT_NAMED);
    let mut files: Vec<_> = named.map(|path| path.display().to_string()).collect();
    if left.len() > LEFT_NAMED {
        files.push("\u{2026}".to_string());
    }
    let text = texts::filled("settings.left", &[("files", &files.join("\n"))]);
    sys::show_error("Avelor", &text);
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::UiTask;
    use crate::testing::{Harness, parse};
    use serde_json::json;

    const SIGN_IN: &str = "01a0e1cc-a24c-7490-82fe-e75669985b32";
    const HASH: &str = "00000000000000ab";

    /// The app's files, and how long each is.
    const OURS: &[(&str, usize)] = &[
        ("config.yaml", 10),
        ("config.yaml.broken", 20),
        (".config.yaml.tmp", 1),
        ("google.client", 100),
        ("catalog/00000000000000ab-v1.db", 1000),
        ("catalog/00000000000000ab-v1.db-wal", 2000),
        ("catalog/00000000000000cd-v12.db", 4000),
        ("log/20261125_101500.log", 10_000),
        ("web/Default/Cache/data_0", 100_000),
        ("web/Local State", 200_000),
    ];

    /// Those that aren't, two of them folders with a name of the app's.
    const OTHERS: &[&str] = &[
        "notes.txt",
        "wishes.token",
        "config.yaml.bak",
        "google.client/file",
        "catalog/photo.db",
        "catalog/00000000000000ab.db",
        "log/20261125.log",
        "log/20261125_101500.log.bak",
    ];

    // ------------------------------------------------------------------------
    fn folders(h: &Harness) -> Folders {
        h.app.folders.clone()
    }

    // ------------------------------------------------------------------------
    /// The app's files in the harness's folder, a sign-in and two keys among
    /// them; the paths.
    fn written(h: &Harness) -> Vec<PathBuf> {
        let id = |n: u8| format!("01a0e1cc-a24c-7490-82fe-e75669985b{n:02}");
        let secrets = [
            format!("{}.token", id(1)),
            format!("{}.key", id(2)),
            format!("{SIGN_IN}.key"),
        ];
        let secrets = secrets.iter().map(|name| (name.as_str(), 100));
        let files: Vec<_> = OURS.iter().copied().chain(secrets).collect();
        let paths = files.iter().map(|(name, len)| {
            let path = h.dir().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, vec![b'x'; *len]).unwrap();
            path
        });
        paths.collect()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn what_is_kept_is_counted_by_its_name() {
        let mut h = Harness::new("kept");
        written(&h);
        for name in OTHERS
            .iter()
            .filter(|name| !name.starts_with("google.client"))
        {
            fs::write(h.dir().join(name), "xxx").unwrap();
        }
        let sent = h.call_waiting("kept", json!({}));
        let expected = json!({
            "folder": h.dir(), "settings": 31, "secrets": 400, "google": 1, "ai": 2,
            "client": true, "catalog": 7000, "web": 300_000, "log": 10_000,
        });
        assert_eq!(sent[0]["ok"], expected);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn nothing_kept_yet_is_nothing() {
        let mut h = Harness::new("nothing-kept");
        let sent = h.call_waiting("kept", json!({}));
        let ok = &sent[0]["ok"];
        assert_eq!((&ok["settings"], &ok["web"]), (&json!(0), &json!(0)));
        assert_eq!((&ok["google"], &ok["client"]), (&json!(0), &json!(false)));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_removal_leaves_nothing_of_the_app_s() {
        let h = Harness::new("removed");
        let folders = folders(&h);
        written(&h);
        assert_eq!(folders.remove(Duration::ZERO), Vec::<PathBuf>::new());
        assert!(!h.dir().exists(), "its folder went too");
        assert_eq!(
            folders.remove(Duration::ZERO),
            Vec::<PathBuf>::new(),
            "again"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_removal_leaves_what_isn_t_the_app_s() {
        let h = Harness::new("removed-but");
        let (folders, ours) = (folders(&h), written(&h));
        let client = h.dir().join("google.client");
        fs::remove_file(&client).unwrap();
        let vault = h.dir().join("vault/.avelor/accounts/a.yaml");
        let others = OTHERS.iter().map(|name| h.dir().join(name));
        let others: Vec<_> = others.chain([vault]).collect();
        for path in &others {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "theirs").unwrap();
        }
        assert_eq!(folders.remove(Duration::ZERO), Vec::<PathBuf>::new());
        for path in ours.iter().filter(|path| **path != client) {
            assert!(!path.exists(), "{}", path.display());
        }
        for path in &others {
            assert_eq!(fs::read_to_string(path).unwrap(), "theirs");
        }
        assert!(!h.dir().join("web").exists());
    }

    // ------------------------------------------------------------------------
    /// As on Linux, each folder in a base folder of the user's, which stays.
    #[test]
    fn a_removal_leaves_the_folders_the_app_s_are_in() {
        let h = Harness::new("removed-apart");
        let of = |base: &str, name: &str| h.dir().join(base).join("avelor").join(name);
        let folders = Folders {
            config: h.dir().join("config/avelor"),
            catalog: of("cache", "catalog"),
            log: of("state", "log"),
            web: vec![of("data", "webkit"), of("cache", "webkit")],
        };
        let files = [
            folders.config.join("config.yaml"),
            folders.catalog.join(format!("{HASH}-v1.db")),
            folders.log.join("20261125_101500.log"),
            folders.web[0].join("storage/a"),
            folders.web[1].join("a"),
        ];
        for path in &files {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "x").unwrap();
        }
        assert_eq!(folders.remove(Duration::ZERO), Vec::<PathBuf>::new());
        for base in ["config", "cache", "state", "data"] {
            let base = h.dir().join(base);
            assert!(base.exists(), "{}", base.display());
            assert!(!base.join("avelor").exists(), "{}", base.display());
        }
    }

    // ------------------------------------------------------------------------
    /// Windows keeps a file that is held open; what can't be removed is left,
    /// and named, once the wait is over.
    #[cfg(target_os = "windows")]
    #[test]
    fn a_removal_tries_again_and_names_what_is_left() {
        use std::os::windows::fs::OpenOptionsExt;
        let hold = |path: &Path| {
            let mut options = fs::OpenOptions::new();
            options.read(true).share_mode(0).open(path).unwrap()
        };
        let h = Harness::new("held");
        let folders = folders(&h);
        written(&h);
        let catalog = h.dir().join(format!("catalog/{HASH}-v1.db"));
        let web = h.dir().join("web/Default/Cache/data_0");
        let held = (hold(&catalog), hold(&web));
        let left = folders.remove(Duration::ZERO);
        assert_eq!(left, [catalog, h.dir().join("web")]);
        assert!(!h.dir().join("config.yaml").exists(), "the rest went");

        let ends = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        });
        assert_eq!(
            folders.remove(Duration::from_secs(10)),
            Vec::<PathBuf>::new()
        );
        ends.join().unwrap();
        assert!(!h.dir().exists());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_removal_takes_the_open_vault_s_catalog_and_not_the_vault() {
        let h = Harness::with_vault("removed-open", &[("a.md", "# A\n")]);
        let catalogs = || files_in(&h.dir().join("catalog"), |_| Some(What::Catalog)).len();
        assert!(catalogs() > 0);
        h.app.vault.catalog().close();
        let left = folders(&h).remove(Duration::from_secs(10));
        assert_eq!((left, catalogs()), (Vec::new(), 0));
        assert_eq!(fs::read_to_string(h.vault().join("a.md")).unwrap(), "# A\n");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn remove_data_asks_the_page_first_as_a_close_does() {
        let mut h = Harness::new("remove-data");
        let ask = |h: &mut Harness| {
            let sent = h.call("remove_data", json!({}));
            assert_eq!(sent[0]["ev"], "close", "{sent:?}");
            assert_eq!(sent[1]["ok"], json!(null));
            sent[0]["data"]["id"].clone()
        };
        // A save that can't be made: the window stays, and the data.
        let id = ask(&mut h);
        h.call("close_ready", json!({ "id": id, "ready": false }));
        assert!(!h.ui.window_closed() && !h.app.close.removes());

        let id = ask(&mut h);
        h.call("close_ready", json!({ "id": id, "ready": true }));
        assert!(h.ui.window_closed() && h.app.close.removes());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_close_afterwards_removes_nothing() {
        let mut h = Harness::new("remove-then-close");
        let sent = h.call("remove_data", json!({}));
        let id = sent[0]["data"]["id"].clone();
        // The window's Close while the page is asked: still the removal.
        close::on_close(&mut h.app);
        assert!(h.app.close.removes());
        h.call("close_ready", json!({ "id": id, "ready": true }));
        assert!(h.app.close.removes());

        // The window couldn't close; its Close later is just that.
        close::on_close(&mut h.app);
        h.run_queue();
        let sent = parse(&h.take());
        let id = sent[0]["data"]["id"].clone();
        h.call("close_ready", json!({ "id": id, "ready": true }));
        assert!(h.ui.window_closed() && !h.app.close.removes());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_session_that_ends_removes_nothing() {
        let mut h = Harness::new("remove-session-end");
        let sent = h.call("remove_data", json!({}));
        let id = sent[0]["data"]["id"].clone();
        h.call("close_ready", json!({ "id": id, "ready": true }));
        close::on_session_end(&mut h.app);
        assert!(!h.app.close.removes());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_folder_is_shown_once_the_handler_has_returned() {
        let mut h = Harness::new("reveal-data");
        let request = json!({ "id": 1, "cmd": "reveal_data", "args": {} });
        h.app.on_web_message(request.to_string());
        let shown = h.ui.pop();
        assert!(matches!(shown, Some(UiTask::Modal(_))));
        assert!(h.take().is_empty(), "not answered before");
    }
}
