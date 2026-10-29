//! The app's own commands (`api.md#the-page-side`): `ready`, switching,
//! revealing and rebuilding the vault, opening a link or a document's
//! original, and the page's preferences; and opening a vault with its
//! accounts, watches and settings (`open_vault`).

use std::path::{Path, PathBuf};

use crate::accounts;
use crate::app::App;
use crate::app::Prefs;
use crate::calendar;
use crate::documents;
use crate::email;
use crate::protocol::Req;
use crate::sys;
use crate::texts::{self, Text};
use crate::vault_settings;
use crate::watches;
use av_utils::error::Error;
use av_utils::id::Id;
use av_utils::url::is_link;
use serde_json::json;

// ----------------------------------------------------------------------------
/// `ready`: the page is loaded and listens. A config that couldn't be read
/// at start, and was set aside (`config::load_config`, which logs why), is
/// told now, in the status bar. Opens the vault of the last session (or the
/// command line's, `ConfigStore::vault`), with its accounts and watches;
/// answers whether this build has Google (`App::has_google`).
pub fn ready(app: &mut App, req: Req) {
    if app.config.take_error().is_some() {
        app.notice(Text::new("config.unread"));
    }
    if let Some(root) = app.config.vault().map(Path::to_path_buf) {
        open_vault(app, &root, false);
    }
    req.ok(json!({ "google": app.has_google }));
}

// ----------------------------------------------------------------------------
/// `switch_vault {path?}`: open the vault at `path`, or the folder the user
/// picks, and remember it for the next start. Answers `ok {path}`, the vault
/// being opened, once the switch is under way; `path` is null when the
/// picker was cancelled, which keeps the vault that is open.
pub fn switch_vault(app: &mut App, req: Req, path: Option<PathBuf>) {
    match path {
        Some(path) => switch_answering(app, req, Some(path)),
        None => app.on_ui(
            |owner| sys::pick_folder(owner, texts::text("vault.pick")),
            |app, picked| switch_answering(app, req, picked),
        ),
    }
}

// ----------------------------------------------------------------------------
/// Switch to `path` (none: the dialog was cancelled) and answer `{path}`; a
/// vault of a newer format is `unsupported`, in words
/// (`vault_settings::refused`), and the open vault stays.
fn switch_answering(app: &mut App, req: Req, path: Option<PathBuf>) {
    if let Some(path) = &path {
        if let Some(why) = vault_settings::refused(path) {
            let what = "a vault of a newer format".to_string();
            return req.fail(Error::Unsupported { what }, why);
        }
        switch_to(app, path.clone());
    }
    req.ok(json!({ "path": path }));
}

// ----------------------------------------------------------------------------
/// Remember `root` as the vault, then open it once the work bound to the
/// open vault has stopped (`App::switch_vault_when_stopped`).
pub(crate) fn switch_to(app: &mut App, root: PathBuf) {
    app.remember_vault(root.clone());
    app.switch_vault_when_stopped(root, false);
}

// ----------------------------------------------------------------------------
/// `rebuild_index`: open the vault again with its catalog emptied, so
/// every file is read (`vault.md#catalog`), as a switch to the same
/// folder does; nothing is remembered, unlike a switch. Answers `null` once
/// it is under way; `not_initialized` if no vault is open.
pub fn rebuild_index(app: &mut App, req: Req) {
    if app.rebuild_when_stopped() {
        req.ok(());
    } else {
        req.fail(Error::NotInitialized, Text::new("vault.none_open"));
    }
}

// ----------------------------------------------------------------------------
/// Make `root` the open vault with its accounts and folder watches, then
/// kick off an indexing that streams progress and results back via
/// `on_index_event`, its catalog emptied first for `rebuild`. A save of
/// the old vault still held for the conflict dialog is kept as a conflict
/// copy there first (`documents::keep_held`).
pub(crate) fn open_vault(app: &mut App, root: &Path, rebuild: bool) {
    if let Some(why) = vault_settings::refused(root) {
        log::warn!("{}: {why}", root.display());
        return app.notice(why);
    }
    documents::keep_held(app);
    app.vault.begin_open(root, rebuild);
    documents::vault_opened(app);
    accounts::open(app, root);
    watches::folders::open(app, root);
    vault_settings::open(app, root);
    email::vault_opened(app);
    calendar::vault_opened(app);
    app.run_indexing(root.to_path_buf());
}

// ----------------------------------------------------------------------------
/// `reveal_vault`: show the open vault's folder in the file manager once the
/// handler has returned, as `open_url`. Answers `null`; `invalid_path` if no
/// vault is open.
pub fn reveal_vault(app: &mut App, req: Req) {
    let Some(root) = app.vault.root().map(Path::to_path_buf) else {
        let error = Error::InvalidPath {
            path: PathBuf::new(),
        };
        return req.fail(error, Text::new("vault.none_open"));
    };
    app.on_ui(
        {
            let root = root.clone();
            move |_| sys::open_path(&root)
        },
        move |_, opened| match opened {
            Ok(()) => req.ok(()),
            Err(e) => {
                log::warn!("Could not show {} in its folder: {e}", root.display());
                req.fail(e, Text::new("vault.show_failed"));
            }
        },
    );
}

// ----------------------------------------------------------------------------
/// Open `url` in its default app (the browser, the email app), once the
/// handler has returned: `ShellExecuteW` may run a modal loop. Only web,
/// email and phone links; anything else could start a program or open a
/// file.
pub fn open_url(app: &mut App, req: Req, url: String) {
    if !is_link(&url) {
        let arg = format!("not a web, email or phone link: {url}");
        return req.err(Error::InvalidArgument { arg });
    }
    app.on_ui(
        {
            let url = url.clone();
            move |_| sys::open_url(&url)
        },
        move |_, opened| match opened {
            Ok(()) => req.ok(()),
            Err(e) => {
                log::warn!("Could not open {url}: {e}");
                req.fail(e, Text::new("open.url_failed").arg("url", url.as_str()));
            }
        },
    );
}

// ----------------------------------------------------------------------------
/// `open_file {id}`: open the document `id`'s original (`vault.md#document`)
/// in its default app once the handler has returned, as `open_url`. Only a
/// PDF or an image ([`may_open`]): an attachment could be a program.
/// Answers `null`; `not_found` without an original, or with its file gone.
pub fn open_file(app: &mut App, req: Req, id: Id) {
    let index = app.vault.index();
    let Some(path) = index.original_path(&id).filter(|p| p.is_file()) else {
        return req.err(Error::NotFound);
    };
    let original = index.get(&id).and_then(|doc| doc.original());
    let name = match original.and_then(|o| o.name.clone()) {
        Some(name) => name,
        None => path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
    };
    if !may_open(&path) {
        let arg = format!("a type the app doesn't open: {name}");
        let text = Text::new("open.other_type").arg("name", name.as_str());
        return req.fail(Error::InvalidArgument { arg }, text);
    }
    app.on_ui(
        {
            let path = path.clone();
            move |_| sys::open_path(&path)
        },
        move |_, opened| match opened {
            Ok(()) => req.ok(()),
            Err(e) => {
                log::warn!("Could not open {}: {e}", path.display());
                req.fail(e, Text::new("open.file_failed").arg("name", name.as_str()));
            }
        },
    );
}

// ----------------------------------------------------------------------------
/// Whether the app opens the file at `path` in its default app: only a PDF
/// or an image, the types it takes in itself (the watches', the
/// attachments'). Any other could be a program, a script or a shortcut.
pub fn may_open(path: &Path) -> bool {
    const OPENED: [&str; 5] = ["pdf", "png", "jpg", "jpeg", "webp"];
    let ext = path.extension().unwrap_or_default().to_string_lossy();
    OPENED.contains(&ext.to_ascii_lowercase().as_str())
}

// ----------------------------------------------------------------------------
/// `set_prefs {prefs}`: store the page's preferences (strings) as given; the
/// host never reads them, and fills them into the page it loads next
/// (`page::html`). Answers `null`, or the failed save, whose error is logged.
pub fn set_prefs(app: &mut App, req: Req, prefs: Prefs) {
    match app.config.set_prefs(prefs) {
        Ok(()) => req.ok(()),
        Err(e) => req.save_failed(e, Text::new("save.settings")),
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::testing::parse;
    use crate::testing::{Harness, shown};
    use serde_json::Value;
    use std::fs;

    // ------------------------------------------------------------------------
    /// The events `ev` among what the page got.
    fn events<'a>(sent: &'a [Value], ev: &str) -> Vec<&'a Value> {
        sent.iter().filter(|m| m["ev"] == ev).collect()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_vault_of_a_newer_format_isnt_opened() {
        let mut h = Harness::new("newer-format");
        let root = h.vault();
        fs::create_dir_all(root.join(".avelor")).unwrap();
        fs::write(root.join(".avelor/vault.yaml"), "format: 2\n").unwrap();

        let sent = h.call("switch_vault", json!({ "path": root }));
        assert_eq!(sent.len(), 1, "only the answer: {sent:?}");
        assert_eq!(sent[0]["err"]["code"], "unsupported");
        let message = shown(&sent[0]["err"]["text"]);
        assert!(
            message.contains("vault format 2; this one reads 1"),
            "{message}"
        );
        assert_ne!(h.app.config.vault(), Some(&*root), "not remembered");

        open_vault(&mut h.app, &root, false);
        let sent = parse(&h.take());
        assert_eq!(events(&sent, "notice").len(), 1, "{sent:?}");
        assert!(events(&sent, "vault").is_empty() && h.app.vault.root().is_none());
        assert!(!root.join(".avelor/accounts").exists(), "nothing written");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn ready_answers_at_once() {
        let mut h = Harness::new("ready");
        let sent = h.call("ready", json!({}));
        assert_eq!(sent, [json!({ "re": 1, "ok": { "google": true } })]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_config_that_could_not_be_read_is_told_once_the_page_listens() {
        let mut h = Harness::new("unread");
        std::fs::write(h.dir().join("config.yaml"), "window: [unclosed\n").unwrap();
        h.app.config = load_config(h.dir()).unwrap();

        let sent = h.call("ready", json!({}));
        let notice = json!({ "level": "error", "text": { "key": "config.unread" } });
        let ready = json!({ "re": 1, "ok": { "google": true } });
        assert_eq!(
            sent,
            [json!({ "ev": "notice", "data": notice }), ready.clone()]
        );
        assert_eq!(h.call("ready", json!({})), [ready], "told once");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_switch_opens_the_vault_and_tells_how_its_scan_went() {
        let mut h = Harness::new("switch");
        let root = h.vault();
        let note = format!(
            "---\nid: {}\ntitle: A\ndocument: {{}}\n---\nA\n",
            Id::generate()
        );
        fs::create_dir_all(root.join("Notes")).unwrap();
        fs::write(root.join("Notes/a.md"), note).unwrap();
        fs::write(root.join("Notes/b.md"), b"---\ntitle: \xff\n---\n").unwrap();

        let mut sent = h.call("switch_vault", json!({ "path": root }));
        h.wait_indexed();
        sent.extend(parse(&h.take()));
        let path = root.to_str().unwrap();
        let reply = sent.iter().find(|m| m["re"] == 1).unwrap();
        assert_eq!(reply["ok"], json!({ "path": path }));
        assert_eq!(h.app.config.vault(), Some(&*root));

        let vault = events(&sent, "vault");
        let opened = |state| json!({ "state": state, "name": "vault", "path": path, "total": 2 });
        assert_eq!(vault.len(), 2, "{sent:?}");
        assert_eq!(vault[0]["data"], opened("indexing"));
        assert_eq!(vault[1]["data"], opened("open"));
        let [failed] = &events(&sent, "vault_error")[..] else {
            panic!("one file couldn't be read: {sent:?}");
        };
        assert!(failed["data"]["path"].as_str().unwrap().ends_with("b.md"));
        assert!(!failed["data"]["error"].as_str().unwrap().is_empty());
        let at = |m: &Value| sent.iter().position(|s| s == m).unwrap();
        assert!(at(vault[0]) < at(failed) && at(failed) < at(vault[1]));
        assert_eq!(events(&sent, "entity").len(), 1);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn revealing_the_vault_needs_one() {
        let mut h = Harness::new("reveal");
        let sent = h.call("reveal_vault", json!({}));
        let err = json!({ "code": "invalid_path", "text": { "key": "vault.none_open" } });
        assert_eq!(sent, [json!({ "re": 1, "err": err })]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn only_an_original_that_is_a_pdf_or_an_image_is_opened() {
        let (setup, note, gone) = (Id::generate(), Id::generate(), Id::generate());
        let document =
            |id: Id, original: &str| format!("---\nid: {id}\ndocument:{original}\n---\n");
        let files = [
            (
                "A/setup.md",
                document(
                    setup,
                    "\n  original:\n    file: setup.EXE\n    name: Rechnung.exe",
                ),
            ),
            ("A/setup.EXE", "MZ".to_string()),
            ("A/note.md", document(note, " {}")),
            (
                "A/gone.md",
                document(gone, "\n  original:\n    file: gone.pdf"),
            ),
        ];
        let mut h = Harness::with_vault("open-file", &files);
        let sent = h.call("open_file", json!({ "id": setup }));
        let message = "Rechnung.exe isn't opened from the app: only PDFs and images are.";
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        assert_eq!(shown(&sent[0]["err"]["text"]), message);
        for id in [note, gone, Id::generate()] {
            let sent = h.call("open_file", json!({ "id": id }));
            assert_eq!(sent[0]["err"]["code"], "not_found", "{id}");
        }
        for opened in ["bill.pdf", "a/Scan.JPG", "photo.jpeg", "p.png", "p.webp"] {
            assert!(may_open(Path::new(opened)), "{opened}");
        }
        for refused in ["setup.exe", "link.url", "a.docx", "t.csv", "README"] {
            assert!(!may_open(Path::new(refused)), "{refused}");
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn rebuilding_the_index_needs_a_vault() {
        let mut h = Harness::new("rebuild");
        let sent = h.call("rebuild_index", json!({}));
        let err = json!({ "code": "not_initialized", "text": { "key": "vault.none_open" } });
        assert_eq!(sent, [json!({ "re": 1, "err": err })]);
        assert!(!h.app.indexing());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_pages_preferences_are_stored_as_given() {
        let mut h = Harness::new("prefs");
        let prefs = json!({ "panel_width": "420", "sidebar_width": "240" });
        let sent = h.call("set_prefs", json!({ "prefs": prefs }));
        assert_eq!(sent, [json!({ "re": 1, "ok": null })]);
        let stored = load_config(h.dir()).unwrap();
        assert_eq!(json!(stored.prefs()), prefs);

        let sent = h.call("set_prefs", json!({ "prefs": { "panel_width": 420 } }));
        assert_eq!(sent[0]["err"]["code"], "bad_request", "strings only");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_window_active_again_is_an_event() {
        let mut h = Harness::new("activated");
        h.app.on_activated();
        h.run_queue();
        let sent = parse(&h.take());
        assert_eq!(sent, [json!({ "ev": "activated", "data": null })]);
    }
}
