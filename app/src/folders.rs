//! Folder watches' commands (api.md#watches): create, change and remove
//! a watch, and pick a folder for it; and what the page gets about the
//! watches (`watches`, `watch`, `watch_removed`). The watches themselves:
//! `watches`.

use std::collections::BTreeMap;
use std::path::Path;

use av_utils::error::Error;
use av_utils::id::Id;
use av_vault::schema::layout;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ai_accounts::ModelChoice;
use crate::app::App;
use crate::config_files::unique_names;
use crate::protocol::Req;
use crate::sys;
use crate::watches::{Collect, Watch, WatchKind, WatchMeta, WatchSettings, Watches};

// ----------------------------------------------------------------------------
/// The vault `root` opens: its watches replace the ones before, and the page
/// gets them all (`watches`).
pub(crate) fn open(app: &mut App, root: &Path) {
    let folder = root.join(layout::WATCHES);
    app.watches = Watches::load(folder);
    let count = app.watches.iter().count();
    log::info!(
        "{count} folder watch(es) in {}",
        app.watches.folder().display()
    );
    post_watches(app);
}

// ----------------------------------------------------------------------------
/// The `watches` event.
#[derive(Serialize)]
struct WatchList<'a> {
    items: Vec<WatchMeta<'a>>,
}

// ----------------------------------------------------------------------------
/// Send every watch of the open vault: the `watches` event, which replaces
/// the ones the page has.
fn post_watches(app: &App) {
    let items = app.watches.iter().map(Watch::meta).collect();
    app.post_event("watches", WatchList { items });
}

// ----------------------------------------------------------------------------
/// Send the watch `id` to the page, if there is one: the `watch` event.
fn post(app: &App, id: Id) {
    if let Ok(watch) = app.watches.get(id) {
        app.post_event("watch", watch.meta());
    }
}

// ----------------------------------------------------------------------------
/// `create_watch`'s args: the new watch's settings, its kind's defaults
/// filled in by the page.
#[derive(Deserialize)]
pub(crate) struct NewWatch {
    #[serde(rename = "type")]
    kind: WatchKind,
    folder: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    patterns: Vec<String>,
    #[serde(default)]
    collect: Option<Collect>,
}

// ----------------------------------------------------------------------------
/// `set_watch`'s args: the watch, and the fields that change.
#[derive(Deserialize)]
pub(crate) struct WatchChange {
    watch: Id,
    /// Empty: the folder's name is shown.
    name: Option<String>,
    folder: Option<String>,
    subfolders: Option<bool>,
    patterns: Option<Vec<String>>,
    collect: Option<Collect>,
    /// Picks by step, set in the watch's `models`; the other steps keep
    /// theirs.
    models: Option<BTreeMap<String, ModelChoice>>,
    enabled: Option<bool>,
}

// ----------------------------------------------------------------------------
/// `folder` trimmed, if it names an existing folder once its `%VAR%`s are
/// expanded; otherwise the error, with what to tell the page.
fn check_folder(folder: &str) -> Result<String, (Error, String)> {
    let folder = folder.trim();
    if folder.is_empty() {
        let arg = "folder".to_string();
        return Err((
            Error::InvalidArgument { arg },
            "Enter a folder.".to_string(),
        ));
    }
    let path = sys::expand_env(folder);
    if path.is_dir() {
        Ok(folder.to_string())
    } else {
        let message = format!("There's no folder at {}.", path.display());
        Err((Error::FileNotFound { path }, message))
    }
}

// ----------------------------------------------------------------------------
/// Patterns, trimmed, without empty entries and repeats.
fn clean(names: Vec<String>) -> Vec<String> {
    unique_names(names.into_iter().map(|n| n.trim().to_string()).collect())
}

// ----------------------------------------------------------------------------
/// Answer a failed save of a watch with a sentence for the page; the
/// error is logged.
fn save_failed(req: Req, error: Error) {
    log::warn!("Could not save the folder watch: {error}");
    req.fail(error, "Could not save the folder watch.");
}

// ----------------------------------------------------------------------------
/// `create_watch {type, folder, name?, patterns?, collect?}`: save
/// a watch once its folder checks out. Answers its id, after the `watch`
/// event; nothing is stored if the folder isn't there, no vault is open, or
/// saving fails.
pub fn create_watch(app: &mut App, req: Req, new: NewWatch) {
    let folder = match check_folder(&new.folder) {
        Ok(folder) => folder,
        Err((error, message)) => return req.fail(error, message),
    };
    let mut settings = WatchSettings::new(new.kind, folder);
    settings.name = new.name.trim().to_string();
    settings.patterns = clean(new.patterns);
    settings.collect = new.collect;
    match app.watches.create(settings) {
        Ok(id) => {
            log::info!("Created folder watch {id}");
            post(app, id);
            req.ok(json!({ "id": id }));
        }
        Err(e @ Error::InvalidPath { .. }) => {
            let message = "Open a vault first: folder watches are kept in it.";
            req.fail(e, message);
        }
        Err(e) => save_failed(req, e),
    }
}

// ----------------------------------------------------------------------------
/// `set_watch {watch, name?, folder?, subfolders?, patterns?, collect?,
/// models?, enabled?}`: change the fields given (`models` sets
/// the picks of the steps it names; a model is checked when it is used); a
/// new folder is checked
/// like a new watch's. Answered after the `watch` event; nothing changes if
/// the check or saving fails.
pub fn set_watch(app: &mut App, req: Req, change: WatchChange) {
    let id = change.watch;
    if let Err(e) = app.watches.get(id) {
        return req.err(e);
    }
    let folder = match change.folder.as_deref().map(check_folder).transpose() {
        Ok(folder) => folder,
        Err((error, message)) => return req.fail(error, message),
    };
    let updated = app.watches.update(id, |settings| {
        if let Some(folder) = folder {
            settings.folder = folder;
        }
        if let Some(name) = change.name {
            settings.name = name.trim().to_string();
        }
        if let Some(subfolders) = change.subfolders {
            settings.subfolders = subfolders;
        }
        if let Some(patterns) = change.patterns {
            settings.patterns = clean(patterns);
        }
        if let Some(collect) = change.collect {
            settings.collect = Some(collect);
        }
        if let Some(models) = change.models {
            settings.models.extend(models);
        }
        if let Some(enabled) = change.enabled {
            settings.enabled = enabled;
        }
    });
    match updated {
        Ok(()) => {
            post(app, id);
            req.ok(());
        }
        Err(e) => save_failed(req, e),
    }
}

// ----------------------------------------------------------------------------
/// `remove_watch {watch}`: delete the watch's file; what it imported stays
/// in the vault. Answered after the `watch_removed` event.
pub fn remove_watch(app: &mut App, req: Req, id: Id) {
    match app.watches.remove(id) {
        Ok(()) => {
            app.post_event("watch_removed", json!({ "id": id }));
            req.ok(());
        }
        Err(Error::NotFound) => req.err(Error::NotFound),
        Err(e) => {
            log::warn!("Could not delete the watch file: {e}");
            req.fail(e, "Could not delete the watch file.");
        }
    }
}

// ----------------------------------------------------------------------------
/// `pick_folder {title?}`: let the user pick a folder in a dialog headed
/// `title`. Answers `{path}`, a path under the user's profile as
/// `%USERPROFILE%\…` (on Linux `~/…`), null if the dialog was cancelled.
pub fn pick_folder(app: &mut App, req: Req, title: Option<String>) {
    let title = title
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Choose a folder".to_string());
    app.on_ui(
        move |owner| sys::pick_folder(owner, &title),
        move |_, picked| {
            let path = picked.map(|path| sys::collapse_env(&path));
            req.ok(json!({ "path": path }));
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, shape};
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;

    const WATCH_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b40";

    // ------------------------------------------------------------------------
    /// A harness whose vault has an images watch named Scanner on the
    /// folder `scans` beside the vault, which exists; and that folder.
    fn with_watch(name: &str) -> (Harness, PathBuf) {
        let mut h = Harness::new(name);
        let root = h.vault();
        let folder = root.with_file_name("scans");
        fs::create_dir_all(&folder).unwrap();
        let watch = format!(
            "id: {WATCH_ID}\nfolder-watch:\n  type: images\n  name: Scanner\n  folder: '{}'\n  patterns: ['*.jpg']\n  custom: kept\n",
            folder.display()
        );
        let note =
            "---\nid: 01a0e1cc-a24c-7490-82fe-e75669985b41\ndocument: {}\ntitle: Note\n---\n";
        for (path, text) in [
            (".avelor/watches/watch_images.yaml", watch.as_str()),
            ("Notes/n.md", note),
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        crate::api::switch_to(&mut h.app, root.clone());
        h.wait_scanned();
        (h, folder)
    }

    // ------------------------------------------------------------------------
    /// The events `ev` among what the page got, parsed.
    fn events(sent: &[String], ev: &str) -> Vec<Value> {
        sent.iter()
            .filter(|text| text.starts_with('{'))
            .map(|text| serde_json::from_str::<Value>(text).unwrap())
            .filter(|message| message["ev"] == ev)
            .collect()
    }

    // ------------------------------------------------------------------------
    fn watch_file(h: &Harness, name: &str) -> PathBuf {
        h.vault().join(layout::WATCHES).join(name)
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_vault_brings_its_watches_and_another_vault_replaces_them() {
        let (mut h, _) = with_watch("vault-watches");
        let sent = h.take();
        let watches = events(&sent, "watches");
        assert_eq!(watches.len(), 1, "{sent:?}");
        let items = watches[0]["data"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], WATCH_ID);
        assert_eq!(items[0]["name"], "Scanner");
        assert_eq!(items[0]["type"], "images");
        let entities = events(&sent, "entity");
        assert_eq!(entities.len(), 1, "the watch file is no entity");

        let other = h.vault().with_file_name("other");
        fs::create_dir_all(&other).unwrap();
        crate::api::switch_to(&mut h.app, other.clone());
        h.wait_scanned();
        let watches = events(&h.take(), "watches");
        assert_eq!(watches[0]["data"]["items"], json!([]));
        assert!(h.app.watches.iter().next().is_none());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_watch_is_created_once_its_folder_is_there() {
        let (mut h, folder) = with_watch("create-watch");
        h.take();
        let missing = folder.join("gone");
        let args = json!({ "type": "images", "folder": missing, "patterns": ["*.png"] });
        let sent = h.call("create_watch", args);
        assert_eq!(shape(&sent), ["reply"]);
        assert_eq!(sent[0]["err"]["code"], "file_not_found");
        let message = sent[0]["err"]["message"].as_str().unwrap();
        assert!(message.starts_with("There's no folder at"), "{message}");
        let sent = h.call("create_watch", json!({ "type": "images", "folder": "  " }));
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        assert_eq!(sent[0]["err"]["message"], "Enter a folder.");
        assert!(!watch_file(&h, "watch_images_2.yaml").exists());

        let args = json!({
            "type": "images",
            "folder": format!(" {} ", folder.display()),
            "patterns": [" *.png", "*.png", ""],
        });
        let sent = h.call("create_watch", args);
        assert_eq!(shape(&sent), ["watch", "reply"]);
        let watch = &sent[0]["data"];
        assert_eq!(sent[1]["ok"]["id"], watch["id"]);
        assert_eq!(watch["name"], "scans", "no name: the folder's");
        assert_eq!(watch["patterns"], json!(["*.png"]));
        assert_eq!(watch["enabled"], true);
        assert!(watch["added"].is_string());
        let text = fs::read_to_string(watch_file(&h, "watch_images_2.yaml")).unwrap();
        assert!(text.contains("type: images"), "{text}");
        assert_eq!(h.app.watches.iter().count(), 2);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn set_watch_changes_only_the_fields_given() {
        let (mut h, folder) = with_watch("set-watch");
        h.take();
        let args = json!({ "watch": WATCH_ID, "name": "", "enabled": false });
        let sent = h.call("set_watch", args);
        assert_eq!(shape(&sent), ["watch", "reply"]);
        assert_eq!(sent[1]["ok"], Value::Null);
        let watch = &sent[0]["data"];
        assert_eq!(watch["name"], "scans", "an empty name shows the folder's");
        assert_eq!(watch["enabled"], false);
        assert_eq!(watch["patterns"], json!(["*.jpg"]));
        let file = watch_file(&h, "watch_images.yaml");
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("enabled: false") && text.contains("custom: kept"),
            "{text}"
        );
        assert!(!text.contains("name:"), "{text}");

        let gone = folder.join("gone");
        let sent = h.call("set_watch", json!({ "watch": WATCH_ID, "folder": gone }));
        assert_eq!(sent[0]["err"]["code"], "file_not_found");
        assert_eq!(fs::read_to_string(&file).unwrap(), text, "nothing changed");

        let unknown = Id::generate();
        let sent = h.call("set_watch", json!({ "watch": unknown, "enabled": true }));
        assert_eq!(sent[0]["err"]["code"], "not_found");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_steps_pick_leaves_the_other_steps_picks() {
        let (mut h, _) = with_watch("set-watch-models");
        h.take();
        let pick = |model: &str| json!({ "account": Id::generate(), "model": model });
        let (read, titles) = (pick("ocr"), pick("small"));
        let models = json!({ "read": read, "title-date": pick("first") });
        h.call("set_watch", json!({ "watch": WATCH_ID, "models": models }));
        let models = json!({ "title-date": titles });
        let sent = h.call("set_watch", json!({ "watch": WATCH_ID, "models": models }));
        assert_eq!(shape(&sent), ["watch", "reply"]);
        let models = &sent[0]["data"]["models"];
        assert_eq!(*models, json!({ "read": read, "title-date": titles }));
        let text = fs::read_to_string(watch_file(&h, "watch_images.yaml")).unwrap();
        assert!(
            text.contains("models:")
                && text.contains("title-date:")
                && text.contains("model: small"),
            "{text}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn remove_watch_deletes_its_file() {
        let (mut h, _) = with_watch("remove-watch");
        h.take();
        let sent = h.call("remove_watch", json!({ "watch": WATCH_ID }));
        assert_eq!(shape(&sent), ["watch_removed", "reply"]);
        assert_eq!(sent[0]["data"]["id"], WATCH_ID);
        assert!(!watch_file(&h, "watch_images.yaml").exists());
        assert!(h.app.watches.iter().next().is_none());
        let sent = h.call("remove_watch", json!({ "watch": WATCH_ID }));
        assert_eq!(sent[0]["err"]["code"], "not_found");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn without_a_vault_no_watch_is_kept() {
        let mut h = Harness::new("watch-no-vault");
        let here = std::env::temp_dir();
        let sent = h.call("create_watch", json!({ "type": "images", "folder": here }));
        assert_eq!(sent[0]["err"]["code"], "invalid_path");
        assert!(h.app.watches.iter().next().is_none());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_change_holds_only_the_fields_given() {
        let change: WatchChange =
            serde_json::from_str(&format!(r#"{{"watch": "{WATCH_ID}", "enabled": false}}"#))
                .unwrap();
        assert_eq!(change.watch, Id::parse(WATCH_ID).unwrap());
        assert_eq!(change.enabled, Some(false));
        assert_eq!(
            (change.name, change.folder, change.collect),
            (None, None, None)
        );
        let change: WatchChange = serde_json::from_str(&format!(
            r#"{{"watch": "{WATCH_ID}", "name": "", "collect": "single", "patterns": []}}"#
        ))
        .unwrap();
        assert_eq!(change.name.as_deref(), Some(""));
        assert_eq!(change.collect, Some(Collect::Single));
        assert_eq!(change.patterns, Some(Vec::new()));
        assert_eq!(change.folder, None);
        assert!(serde_json::from_str::<WatchChange>(r#"{"enabled": true}"#).is_err());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_folder_must_exist() {
        let here = std::env::temp_dir();
        let here = here.to_string_lossy();
        assert_eq!(
            check_folder(&format!("  {here} ")).ok().as_deref(),
            Some(here.as_ref())
        );
        assert!(check_folder(" ").is_err());
        let missing = std::env::temp_dir()
            .join("avelor-surely-missing")
            .join("folder");
        let (error, message) = check_folder(&missing.to_string_lossy()).unwrap_err();
        assert!(matches!(error, Error::FileNotFound { .. }));
        assert_eq!(
            message,
            format!("There's no folder at {}.", missing.display())
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn patterns_are_trimmed_and_unique() {
        let patterns = [" *.jpg", "*.png", "", "*.jpg "].map(String::from).to_vec();
        assert_eq!(clean(patterns), ["*.jpg", "*.png"]);
    }
}
