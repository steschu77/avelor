//! The app's own commands (`api.md#the-page-side`): `ready`, switching and
//! revealing the vault, opening a link, and the page's preferences; and
//! opening a vault with its accounts and settings (`open_vault`).

use std::path::{Path, PathBuf};

use crate::accounts;
use crate::app::App;
use crate::app::Prefs;
use crate::documents;
use crate::protocol::Req;
use crate::sys;
use crate::vault_settings;
use av_utils::error::Error;
use serde_json::json;

// ----------------------------------------------------------------------------
/// `ready`: the page is loaded and listens. A config that couldn't be read
/// at start, and was set aside (`config::load_config`, which logs why), is
/// told now, in the status bar. Opens the vault of the last session (or the
/// command line's, `ConfigStore::vault`), with its accounts; answers whether
/// this build has Google (`App::hides_google`).
pub fn ready(app: &mut App, req: Req) {
    if app.config.take_error().is_some() {
        app.notice(CONFIG_UNREAD);
    }
    if let Some(root) = app.config.vault().map(Path::to_path_buf) {
        open_vault(app, &root);
    }
    req.ok(json!({ "google": !app.hides_google }));
}

/// The notice of a config that couldn't be read at start.
const CONFIG_UNREAD: &str =
    "config.yaml couldn't be read; the app started with the default settings.";

// ----------------------------------------------------------------------------
/// `switch_vault {path?}`: open the vault at `path`, or the folder the user
/// picks, and remember it for the next start. Answers `ok {path}`, the vault
/// being opened, once the switch is under way; `path` is null when the
/// picker was cancelled, which keeps the vault that is open.
pub fn switch_vault(app: &mut App, req: Req, path: Option<PathBuf>) {
    match path {
        Some(path) => {
            switch_to(app, path.clone());
            req.ok(json!({ "path": path }));
        }
        None => app.on_ui(
            |owner| sys::pick_folder(owner, "Select Vault"),
            |app, picked| {
                if let Some(path) = &picked {
                    switch_to(app, path.clone());
                }
                req.ok(json!({ "path": picked }));
            },
        ),
    }
}

// ----------------------------------------------------------------------------
/// Remember `root` as the vault, then open it once the work bound to the
/// open vault has stopped (`App::switch_vault_when_stopped`).
pub(crate) fn switch_to(app: &mut App, root: PathBuf) {
    app.remember_vault(root.clone());
    app.switch_vault_when_stopped(root);
}

// ----------------------------------------------------------------------------
/// Make `root` the open vault with its accounts, then kick off a scan that
/// streams progress and results back via `on_scan_event`. A save of the old
/// vault still held for the conflict dialog is kept as a conflict copy
/// there first (`documents::keep_held`).
pub(crate) fn open_vault(app: &mut App, root: &Path) {
    documents::keep_held(app);
    app.vault.begin_open(root);
    accounts::open(app, root);
    vault_settings::open(app, root);
    app.run_scan(root.to_path_buf());
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
        return req.fail(error, "No vault is open.");
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
                req.fail(e, "Could not show the vault's folder.");
            }
        },
    );
}

// ----------------------------------------------------------------------------
/// Open `url` in its default app (the browser, the mail app), once the
/// handler has returned: `ShellExecuteW` may run a modal loop. Only web,
/// mail and phone links; anything else could start a program or open a
/// file.
pub fn open_url(app: &mut App, req: Req, url: String) {
    if !is_link(&url) {
        let arg = format!("not a web, mail or phone link: {url}");
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
                req.fail(e, format!("Could not open {url}."));
            }
        },
    );
}

// ----------------------------------------------------------------------------
/// Whether `url` is a link `open_url` hands to the shell: `http`, `https`,
/// `mailto` or `tel`, the absolute links the editor allows
/// (`editor.md#links`).
fn is_link(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    ["http://", "https://", "mailto:", "tel:"]
        .iter()
        .any(|scheme| url.starts_with(scheme))
}

// ----------------------------------------------------------------------------
/// `set_prefs {prefs}`: store the page's preferences (strings) as given; the
/// host never reads them, and fills them into the page it loads next
/// (`page::html`). Answers `null`, or the failed save, whose error is logged.
pub fn set_prefs(app: &mut App, req: Req, prefs: Prefs) {
    match app.config.set_prefs(prefs) {
        Ok(()) => req.ok(()),
        Err(e) => {
            log::warn!("Could not save the settings: {e}");
            req.fail(e, "Could not save the settings.");
        }
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::testing::Harness;
    use crate::testing::parse;
    use av_utils::id::Id;
    use serde_json::Value;
    use std::fs;

    // ------------------------------------------------------------------------
    /// The events `ev` among what the page got.
    fn events<'a>(sent: &'a [Value], ev: &str) -> Vec<&'a Value> {
        sent.iter().filter(|m| m["ev"] == ev).collect()
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
        let notice = json!({ "level": "error", "message": CONFIG_UNREAD });
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
        h.wait_scanned();
        sent.extend(parse(&h.take()));
        let path = root.to_str().unwrap();
        let reply = sent.iter().find(|m| m["re"] == 1).unwrap();
        assert_eq!(reply["ok"], json!({ "path": path }));
        assert_eq!(h.app.config.vault(), Some(&*root));

        let vault = events(&sent, "vault");
        let opened = |state| json!({ "state": state, "name": "vault", "path": path, "total": 2 });
        assert_eq!(vault.len(), 2, "{sent:?}");
        assert_eq!(vault[0]["data"], opened("scanning"));
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
        let err = json!({ "code": "invalid_path", "message": "No vault is open." });
        assert_eq!(sent, [json!({ "re": 1, "err": err })]);
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
