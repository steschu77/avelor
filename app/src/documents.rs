//! A document's content in the page's editors (`saving.md`): loading
//! it. Typed commands (`api.md#documents`). A file changed
//! outside the app is taken in if the app had only read it -- `entity` and
//! `content` pushed before the answer -- and otherwise answered as a
//! conflict for the dialog, nothing written (`saving.md#conflicts`).

use av_utils::id::Id;
use av_vault::vault::Refresh;
use serde::Serialize;

use crate::app::App;
use crate::config::is_false;
use crate::protocol::Req;
use crate::structure::Conflict;

// ----------------------------------------------------------------------------
/// The answer of `load_doc_content`: the content as
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
/// `load_doc_content`: `id`'s content, once its file was looked at -- the
/// content as it is then, `gone`, or with the `conflict` over it.
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
/// Refresh `id` (`Vault::refresh`), logging what came of it and pushing
/// what was taken in (`post_taken_in`). A conflict -- the app's own
/// saved edits written over -- is left to the caller's answer. `None` if
/// the file couldn't be looked at.
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
/// `entity_deleted` -- both, if its `id` was changed by hand.
pub(crate) fn post_taken_in(app: &mut App, id: &Id, now: Option<Id>) {
    if now != Some(*id) {
        app.post_entity_deleted(id);
    }
    if let Some(now) = now {
        if let Some(entry) = app.vault.index().entry(&now) {
            app.post_entity(entry);
        }
        app.post_content(&now);
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
}
