//! The trash (`vault.md#trash`): listing what a delete moved to
//! `.trash`, restoring it, and deleting it for good -- Workspace > Trash.
//! Typed commands (`api.md#trash`). A restore is an edit like the
//! structural ones: the restored entity is pushed as `entity` before the
//! answer `ok {id}`. A failure is answered in the host's words.

use std::path::Path;

use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::id::Id;
use av_vault::envelope::{DocumentType, Source};
use av_vault::shared::Genre;
use av_vault::vault::Trashed;
use serde::Serialize;

use crate::app::App;
use crate::protocol::Req;

// ----------------------------------------------------------------------------
/// The answer of `list_trash`.
#[derive(Serialize)]
struct TrashList {
    items: Vec<TrashItem>,
}

// ----------------------------------------------------------------------------
/// A file in the trash as the page lists it: `path` relative to `.trash`,
/// which names it in `restore_entity` and `delete_trashed`, and what it
/// holds, as far as the `entity` event shows it in the table.
#[derive(Serialize)]
struct TrashItem {
    path: String,
    /// None for a file without `id`, which gets a new one when restored.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<DocumentType>,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    genre: Option<Genre>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<Source>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date: Option<DateTime>,
    /// Why the file couldn't be read, or what is wrong with its frontmatter.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl TrashItem {
    // ------------------------------------------------------------------------
    fn of(trashed: &Trashed) -> Self {
        let path = trashed.local.display().to_string();
        match &trashed.doc {
            Ok(doc) => Self {
                path,
                id: doc.adopted().then(|| doc.id()),
                kind: Some(doc.kind()),
                title: doc.title().to_string(),
                genre: doc.roles().document.as_ref().and_then(|d| d.genre.clone()),
                source: doc.source().cloned(),
                date: doc.date(),
                error: doc
                    .error()
                    .map(|_| "Its frontmatter can't be read.".to_string()),
            },
            Err(_) => Self {
                path,
                id: None,
                kind: None,
                title: String::new(),
                genre: None,
                source: None,
                date: None,
                error: Some("It can't be read.".to_string()),
            },
        }
    }
}

// ----------------------------------------------------------------------------
/// The answer of `restore_entity`: the restored entity.
#[derive(Serialize)]
struct Restored {
    id: Id,
}

// ----------------------------------------------------------------------------
/// The answer of `delete_trashed` and `empty_trash`: `{}`.
#[derive(Serialize)]
struct Deleted {}

// ----------------------------------------------------------------------------
/// `list_trash`: the files in the trash, newest first.
pub fn list_trash(app: &mut App, req: Req) {
    let items = app
        .vault
        .index()
        .trashed()
        .iter()
        .map(TrashItem::of)
        .collect();
    req.ok(TrashList { items });
}

// ----------------------------------------------------------------------------
/// `restore_entity`: move the trashed file `path` back to where it was; the
/// answer names the restored entity.
pub fn restore_entity(app: &mut App, req: Req, path: String) {
    match app.edit(|index, changes| index.restore(Path::new(&path), changes)) {
        Ok(id) => {
            log::info!("Restored {path} as {id}");
            req.ok(Restored { id });
        }
        Err(err) => {
            log::warn!("Could not restore {path}: {err}");
            let message = match err {
                Error::FileExists { .. } => format!(
                    "Could not restore {path}: a file in its place, or an entity with its id, is in the vault already."
                ),
                _ => format!("Could not restore {path}."),
            };
            req.fail(err, message);
        }
    }
}

// ----------------------------------------------------------------------------
/// `delete_trashed`: delete the trashed file `path` for good.
pub fn delete_trashed(app: &mut App, req: Req, path: String) {
    match app.vault.index().delete_trashed(Path::new(&path)) {
        Ok(()) => {
            log::info!("Deleted {path} from the trash");
            req.ok(Deleted {});
        }
        Err(err) => {
            log::warn!("Could not delete {path} from the trash: {err}");
            req.fail(err, format!("Could not delete {path} for good."));
        }
    }
}

// ----------------------------------------------------------------------------
/// `empty_trash`: delete every file in the trash for good.
pub fn empty_trash(app: &mut App, req: Req) {
    match app.vault.index().empty_trash() {
        Ok(n) => {
            log::info!("Emptied the trash: {n} files");
            req.ok(Deleted {});
        }
        Err(err) => {
            log::warn!("Could not empty the trash: {err}");
            req.fail(err, "Could not empty the trash.");
        }
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use crate::testing::{Harness, notes, shape, trashed};
    use av_utils::id::Id;
    use serde_json::json;

    // ------------------------------------------------------------------------
    /// A vault of three notes, named `name`, the first deleted.
    fn with_a_deleted_note(name: &str) -> (Harness, Vec<Id>) {
        let (mut h, ids) = notes(name, 3);
        h.call("delete_entity", json!({ "id": ids[0] }));
        (h, ids)
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_trash_lists_a_deleted_note() {
        let (mut h, ids) = with_a_deleted_note("trash-list");
        let sent = h.call("list_trash", json!({}));
        let items = sent[0]["ok"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], json!(ids[0]));
        assert_eq!(
            (&items[0]["title"], &items[0]["kind"]),
            (&json!("Note 0"), &json!("document"))
        );
        assert!(items[0]["path"].as_str().unwrap().ends_with("n0.md"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_restore_pushes_the_entity_before_the_answer() {
        let (mut h, ids) = with_a_deleted_note("trash-restore");
        let sent = h.call("restore_entity", json!({ "path": "Notes/n0.md" }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        assert_eq!(sent[0]["data"]["id"], json!(ids[0]));
        assert_eq!(sent[1], json!({ "re": 1, "ok": { "id": ids[0] } }));
        assert!(h.vault().join("Notes/n0.md").exists());
        assert!(!trashed(&h.vault(), "Notes/n0.md"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_restore_into_a_taken_place_is_refused() {
        let (mut h, _) = with_a_deleted_note("trash-taken");
        std::fs::write(h.vault().join("Notes/n0.md"), "taken\n").unwrap();
        let sent = h.call("restore_entity", json!({ "path": "Notes/n0.md" }));
        assert_eq!(shape(&sent), ["reply"]);
        assert_eq!(sent[0]["err"]["code"], "file_exists");
        assert!(trashed(&h.vault(), "Notes/n0.md"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn deleting_and_emptying_leave_the_trash_empty() {
        let (mut h, ids) = with_a_deleted_note("trash-empty");
        h.call("delete_entity", json!({ "id": ids[1] }));

        let sent = h.call("delete_trashed", json!({ "path": "Notes/n0.md" }));
        assert_eq!(sent, [json!({ "re": 1, "ok": {} })]);
        assert!(!trashed(&h.vault(), "Notes/n0.md"));
        assert!(trashed(&h.vault(), "Notes/n1.md"));

        let sent = h.call("empty_trash", json!({}));
        assert_eq!(sent, [json!({ "re": 1, "ok": {} })]);
        let sent = h.call("list_trash", json!({}));
        assert_eq!(sent[0]["ok"]["items"], json!([]));
    }
}
