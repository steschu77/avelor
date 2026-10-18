//! A document's content in the page's editors (`saving.md`): loading
//! it, saving the editor's markdown, and looking at the files again when the
//! window is active. Typed commands (`api.md#documents`). A file changed
//! outside the app is taken in if the app had only read it -- `entity` and
//! `content` pushed before the answer -- and otherwise answered as a
//! conflict for the dialog, nothing written (`saving.md#conflicts`). The
//! page also says which documents it shows in edit mode (`set_editing`):
//! leaving edit mode is the point to sync what was edited
//! (`saving.md#edit-mode`).
//!
//! The app's version of a document in conflict -- a save refused, or its
//! saved edits written over outside the app -- is held until the dialog is
//! answered; if no one is left to answer it -- the session ends, another
//! vault opens -- it is kept as a conflict copy beside its file
//! (`keep_held`, `saving.md#held-saves`).

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::id::Id;
use av_vault::document::Document;
use av_vault::envelope::Roles;
use av_vault::envelope::{Empty, Source};
use av_vault::vault::Refresh;
use av_vault::vault::SaveOutcome;
use serde::Serialize;

use crate::app::App;
use crate::calendar;
use crate::config::is_false;
use crate::protocol::Req;
use crate::structure::Conflict;

// ----------------------------------------------------------------------------
/// The tag of a version kept beside its entity (`keep_conflict_copy`).
pub(crate) const CONFLICT_TAG: &str = "conflict";

// ----------------------------------------------------------------------------
/// The documents in the page's editors (`App::documents`).
#[derive(Default)]
pub struct DocumentState {
    /// The documents the page shows in edit mode (`set_editing`): more edits
    /// of them are coming, so leaving edit mode is the point to sync them.
    /// Cleared when a vault opens (the page loading opens it too).
    editing: HashSet<Id>,
    /// The app's version of each document whose file changed outside the
    /// app while the conflict dialog asks about it -- a refused save
    /// (`save_doc_content`), or the app's saved edits written over
    /// (`refresh`) -- until the dialog is answered: kept as conflict copies
    /// if no one is left to answer it (`keep_held`).
    held: HashMap<Id, String>,
}

impl DocumentState {
    // ------------------------------------------------------------------------
    /// Whether the page shows `id` in edit mode.
    pub fn is_editing(&self, id: &Id) -> bool {
        self.editing.contains(id)
    }

    // ------------------------------------------------------------------------
    pub fn editing(&self) -> impl Iterator<Item = &Id> {
        self.editing.iter()
    }

    // ------------------------------------------------------------------------
    /// Drop the app's version of `id` held for the conflict dialog: the file
    /// was read again (Use theirs, `structure::reload_entity`).
    pub fn release(&mut self, id: &Id) {
        self.held.remove(id);
    }

    // ------------------------------------------------------------------------
    /// The app's version of `id` held for the conflict dialog, if any.
    #[cfg(test)]
    pub fn held(&self, id: &Id) -> Option<&str> {
        self.held.get(id).map(String::as_str)
    }
}

// ----------------------------------------------------------------------------
/// The answer of `load_doc_content` and `save_doc_content`: the content as
/// the file holds it now -- its markdown `source` and the editor `html`
/// rendered from it, as in the `content` event -- or that the file is gone,
/// or the conflict to ask about.
#[derive(Default, Serialize)]
struct Content {
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<String>,
    /// The entity left the vault: its file is gone, or its `id` was changed
    /// by hand (it comes back as a new entity).
    #[serde(skip_serializing_if = "is_false")]
    gone: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    conflict: Option<Conflict>,
}

impl Content {
    // ------------------------------------------------------------------------
    /// `id`'s content as it is now.
    fn of(app: &App, id: &Id) -> Self {
        let (source, html) = app.vault.content(id);
        Self {
            source: Some(source),
            html: Some(html),
            ..Default::default()
        }
    }
}

// ----------------------------------------------------------------------------
/// The answer of `check_files`.
#[derive(Serialize)]
struct Checked {
    /// The entities taken in or gone; the page rebuilds its panel if any.
    taken_in: usize,
    /// The files changed outside the app after it saved its own edits.
    conflicts: Vec<Conflict>,
}

// ----------------------------------------------------------------------------
/// `load_doc_content`: `id`'s content, once its file was looked at -- the
/// content as it is then, `gone`, or with the `conflict` over it (the app's
/// version is shown, so Use ours can write it back).
pub fn load_doc_content(app: &mut App, req: Req, id: Id) {
    let refreshed = refresh(app, &id);
    if app.vault.index().entry(&id).is_none() {
        return req.ok(Content {
            gone: true,
            ..Default::default()
        });
    }
    let mut content = Content::of(app, &id);
    if refreshed == Some(Refresh::Conflict) {
        content.conflict = Some(Conflict::of(app, id));
    }
    req.ok(content);
}

// ----------------------------------------------------------------------------
/// `save_doc_content`: save `markdown` (the page's serialization of the
/// editor's DOM, `saving.md#messages`) as `id`'s new content, answered with
/// what the file holds then (also when nothing changed: the page takes it
/// as the acknowledgment). A file changed outside the app is answered as a
/// conflict and not written, unless `overwrite`, the dialog's Use ours.
/// A refused save is held until a save of the document goes through or it
/// is reloaded (`DocumentState::held`). A saved event may wait to be pushed
/// to Google (`calendar::changed`, which waits for the end of edit mode). A
/// failure is answered in the host's words.
pub fn save_doc_content(app: &mut App, req: Req, id: Id, markdown: String, overwrite: bool) {
    match app.vault.save_content(&id, &markdown, overwrite) {
        Ok(SaveOutcome::Conflict(_)) => {
            log::info!("Not saving {id}: changed outside the app");
            let conflict = Conflict::of(app, id);
            app.documents.held.insert(id, markdown);
            req.ok(Content {
                conflict: Some(conflict),
                ..Default::default()
            });
        }
        Ok(SaveOutcome::Saved) => {
            app.documents.held.remove(&id);
            calendar::changed(app, [&id]);
            req.ok(Content::of(app, &id));
        }
        Ok(SaveOutcome::Unchanged) => {
            app.documents.held.remove(&id);
            req.ok(Content::of(app, &id));
        }
        Err(err) => {
            log::warn!("Could not save {id}: {err}");
            let message = match err {
                Error::NotFound => "Could not save the document: it is no longer in the vault.",
                _ => "Could not save the document.",
            };
            req.fail(err, message);
        }
    }
}

// ----------------------------------------------------------------------------
/// Keep a version of `id` that a conflict would lose as a new entity beside
/// it, made here and tagged `conflict`: with `content` as its body (none:
/// the entity's own), and its roles as `roles` leaves them. Calendar sync
/// keeps the vault's version of an event Google changed too
/// (`calendar::keep_local_version`). The new entity's id.
pub(crate) fn keep_conflict_copy(
    app: &mut App,
    id: Id,
    content: Option<String>,
    roles: impl FnOnce(&mut Roles),
) -> Result<Id> {
    let Some(entry) = app.vault.index().entry(&id) else {
        return Err(Error::NotFound);
    };
    let doc = &entry.doc;
    let mut kept = doc.roles().clone();
    roles(&mut kept);
    let mut copy = Document::create(
        kept,
        Source::Created(Empty {}),
        doc.title().to_string(),
        doc.projects().to_vec(),
        doc.format(),
    );
    copy.set_content(content.unwrap_or_else(|| doc.content().to_string()));
    if let Some(date) = doc.date() {
        copy.set_date(date);
    }
    let mut tags = doc.tags().to_vec();
    if !tags.iter().any(|t| t == CONFLICT_TAG) {
        tags.push(CONFLICT_TAG.to_string());
    }
    copy.set_tags(tags);
    let folder = entry.local.parent().unwrap_or(Path::new("")).to_path_buf();
    app.edit(|index, changes| index.create(copy, &folder, changes))
}

// ----------------------------------------------------------------------------
/// Keep the app's version of every document still in conflict
/// (`DocumentState::held`) as a conflict copy (`keep_conflict_copy`), when
/// no one is left to answer the dialog: the session ends
/// (`close::on_session_end`), another vault opens (`api::open_vault`).
pub(crate) fn keep_held(app: &mut App) {
    for (id, markdown) in std::mem::take(&mut app.documents.held) {
        match keep_conflict_copy(app, id, Some(markdown), |_| {}) {
            Ok(copy) => log::info!("Kept the edits of {id} held for a conflict as {copy}"),
            Err(e) => log::warn!("Could not keep the edits of {id} held for a conflict: {e}"),
        }
    }
}

// ----------------------------------------------------------------------------
/// A vault opens (`api::open_vault`): nothing is in edit mode until the page
/// says so.
pub(crate) fn vault_opened(app: &mut App) {
    app.documents.editing.clear();
}

// ----------------------------------------------------------------------------
/// `set_editing`: the documents the page shows in edit mode now, all of
/// them, sent after every change -- after the save that ends an edit, which
/// so arrives first. A document that left edit mode is a sync point: an
/// event waiting to be pushed syncs its calendar (`calendar::changed`),
/// right away if it can.
pub fn set_editing(app: &mut App, req: Req, ids: Vec<Id>) {
    let editing: HashSet<Id> = ids.into_iter().collect();
    let ended: Vec<Id> = app
        .documents
        .editing
        .difference(&editing)
        .copied()
        .collect();
    app.documents.editing = editing;
    if !ended.is_empty() {
        calendar::changed(app, &ended);
        calendar::auto_sync(app, DateTime::now());
    }
    req.ok(());
}

// ----------------------------------------------------------------------------
/// `check_files`: look at the files of `ids` -- the documents the page shows
/// without unsaved edits, once the window is active again -- and take in
/// what was changed outside the app, unless it replaced the app's own
/// edits: those are answered as conflicts (`saving.md#conflicts`).
pub fn check_files(app: &mut App, req: Req, ids: Vec<Id>) {
    let mut checked = Checked {
        taken_in: 0,
        conflicts: Vec::new(),
    };
    for id in ids {
        match refresh(app, &id) {
            Some(Refresh::TakenIn(_)) => checked.taken_in += 1,
            Some(Refresh::Conflict) => checked.conflicts.push(Conflict::of(app, id)),
            Some(Refresh::Unchanged) | None => {}
        }
    }
    req.ok(checked);
}

// ----------------------------------------------------------------------------
/// Refresh `id` (`Vault::refresh`), logging what came of it and pushing
/// what was taken in (`post_taken_in`). A conflict -- the app's own
/// saved edits written over -- is left to the caller's answer; the app's
/// version, still the index's, is held for the dialog like a refused save
/// (`DocumentState::held`). `None` if the file couldn't be looked at.
fn refresh(app: &mut App, id: &Id) -> Option<Refresh> {
    let path = app
        .vault
        .index()
        .entry(id)
        .map(|e| e.local.display().to_string());
    let path = path.unwrap_or_default();
    match app.vault.refresh(id) {
        Ok(Refresh::TakenIn(now)) => {
            log::info!("Took in {path}: changed outside the app");
            post_taken_in(app, id, now);
            Some(Refresh::TakenIn(now))
        }
        Ok(Refresh::Conflict) => {
            log::info!("{path} was changed outside the app after the app saved it");
            let (ours, _) = app.vault.content(id);
            app.documents.held.insert(*id, ours);
            Some(Refresh::Conflict)
        }
        Ok(Refresh::Unchanged) => Some(Refresh::Unchanged),
        Err(err) => {
            log::warn!("Could not look at {path}: {err}");
            None
        }
    }
}

// ----------------------------------------------------------------------------
/// Send what reading `id`'s file again gave (`now`: the id the file has
/// now, `None` if it is gone): the entity with its content, or
/// `entity_deleted` -- both, if its `id` was changed by hand. An event
/// taken in may wait to be pushed to Google (`calendar::changed`).
pub(crate) fn post_taken_in(app: &mut App, id: &Id, now: Option<Id>) {
    if now != Some(*id) {
        app.post_entity_deleted(id);
    }
    if let Some(now) = now {
        if let Some(entry) = app.vault.index().entry(&now) {
            app.post_entity(entry);
        }
        app.post_content(&now);
        calendar::changed(app, [&now]);
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use crate::testing::{notes, shape};
    use av_utils::id::Id;
    use serde_json::json;
    use std::fs;

    // ------------------------------------------------------------------------
    /// The text of `Notes/n0.md` as changed by another program: longer, so
    /// its stamp differs within the same second too.
    fn theirs(id: Id) -> String {
        format!("---\nid: {id}\ntitle: Note 0\ndocument: {{}}\n---\nTheirs, longer\n")
    }

    // ------------------------------------------------------------------------
    #[test]
    fn loading_answers_with_the_content() {
        let (mut h, ids) = notes("load", 1);
        let sent = h.call("load_doc_content", json!({ "id": ids[0] }));
        assert_eq!(shape(&sent), ["reply"]);
        assert_eq!(sent[0]["ok"]["source"], "Body 0\n");
        assert!(sent[0]["ok"]["html"].as_str().unwrap().contains("Body 0"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn loading_takes_in_a_file_the_app_only_read() {
        let (mut h, ids) = notes("load-taken-in", 1);
        fs::write(h.vault().join("Notes/n0.md"), theirs(ids[0])).unwrap();
        let sent = h.call("load_doc_content", json!({ "id": ids[0] }));
        assert_eq!(shape(&sent), ["entity", "content", "reply"]);
        assert_eq!(sent[2]["ok"]["source"], "Theirs, longer\n");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn loading_a_removed_file_answers_gone() {
        let (mut h, ids) = notes("load-gone", 1);
        fs::remove_file(h.vault().join("Notes/n0.md")).unwrap();
        let sent = h.call("load_doc_content", json!({ "id": ids[0] }));
        assert_eq!(shape(&sent), ["entity_deleted", "reply"]);
        assert_eq!(sent[1]["ok"], json!({ "gone": true }));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn saving_writes_and_answers_with_what_the_file_holds() {
        let (mut h, ids) = notes("save", 1);
        let args = json!({ "id": ids[0], "markdown": "Ours\n" });
        let sent = h.call("save_doc_content", args);
        assert_eq!(shape(&sent), ["reply"]);
        assert_eq!(sent[0]["ok"]["source"], "Ours\n");
        let text = fs::read_to_string(h.vault().join("Notes/n0.md")).unwrap();
        assert!(text.ends_with("---\nOurs\n"), "{text}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_save_over_a_changed_file_is_a_conflict_until_overwritten() {
        let (mut h, ids) = notes("save-conflict", 1);
        let path = h.vault().join("Notes/n0.md");
        fs::write(&path, theirs(ids[0])).unwrap();

        let args = json!({ "id": ids[0], "markdown": "Ours\n" });
        let sent = h.call("save_doc_content", args);
        assert_eq!(shape(&sent), ["reply"]);
        let ok = &sent[0]["ok"];
        assert_eq!(ok["conflict"]["id"], ids[0].to_string());
        assert!(ok.get("source").is_none(), "{ok}");
        assert_eq!(fs::read_to_string(&path).unwrap(), theirs(ids[0]));

        let args = json!({ "id": ids[0], "markdown": "Ours\n", "overwrite": true });
        let sent = h.call("save_doc_content", args);
        assert_eq!(sent[0]["ok"]["source"], "Ours\n");
        assert!(fs::read_to_string(&path).unwrap().ends_with("---\nOurs\n"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_refused_save_is_held_until_the_conflict_is_answered() {
        let (mut h, ids) = notes("save-held", 1);
        let id = ids[0];
        // An edit of `markdown` over the file changed outside the app.
        let refuse = |h: &mut crate::testing::Harness, markdown: &str| {
            fs::write(h.vault().join("Notes/n0.md"), theirs(id)).unwrap();
            let sent = h.call(
                "save_doc_content",
                json!({ "id": id, "markdown": markdown }),
            );
            assert!(sent[0]["ok"]["conflict"].is_object(), "{sent:?}");
            assert_eq!(h.app.documents.held[&id], markdown);
        };

        refuse(&mut h, "Ours\n");
        let use_ours = json!({ "id": id, "markdown": "Ours\n", "overwrite": true });
        h.call("save_doc_content", use_ours);
        assert!(h.app.documents.held.is_empty(), "Use ours went through");

        refuse(&mut h, "Ours again\n");
        h.call("reload_entity", json!({ "id": id }));
        assert!(h.app.documents.held.is_empty(), "Use theirs");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_vault_switch_keeps_a_held_save_as_a_conflict_copy() {
        let (mut h, ids) = notes("save-held-switch", 1);
        let old = h.vault();
        fs::write(old.join("Notes/n0.md"), theirs(ids[0])).unwrap();
        h.call(
            "save_doc_content",
            json!({ "id": ids[0], "markdown": "Ours\n" }),
        );

        let other = old.with_file_name("other");
        fs::create_dir_all(&other).unwrap();
        crate::api::switch_to(&mut h.app, other.clone());
        h.wait_scanned();
        assert!(h.app.documents.held.is_empty());
        let notes: Vec<String> = fs::read_dir(old.join("Notes"))
            .unwrap()
            .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
            .collect();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes.contains(&theirs(ids[0])), "theirs stays");
        let copy = notes.iter().find(|n| **n != theirs(ids[0])).unwrap();
        assert!(
            copy.contains("conflict") && copy.ends_with("Ours\n"),
            "{copy}"
        );
        let _ = fs::remove_dir_all(&other);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn checking_takes_in_read_files_and_asks_about_saved_ones() {
        let (mut h, ids) = notes("check", 2);
        // n1 is saved by the app first, so a change over it is a conflict.
        let args = json!({ "id": ids[1], "markdown": "Ours\n" });
        h.call("save_doc_content", args);
        for (i, id) in ids.iter().enumerate() {
            let text =
                format!("---\nid: {id}\ntitle: Note {i}\ndocument: {{}}\n---\nTheirs, longer\n");
            fs::write(h.vault().join(format!("Notes/n{i}.md")), text).unwrap();
        }

        let sent = h.call("check_files", json!({ "ids": ids }));
        assert_eq!(shape(&sent), ["entity", "content", "reply"]);
        assert_eq!(sent[1]["data"]["id"], ids[0].to_string());
        let ok = &sent[2]["ok"];
        assert_eq!(ok["taken_in"], 1);
        assert_eq!(ok["conflicts"][0]["id"], ids[1].to_string());
        // The app's version waits for the dialog's answer.
        assert_eq!(h.app.documents.held.len(), 1);
        assert_eq!(h.app.documents.held[&ids[1]], "Ours\n");
    }
}
