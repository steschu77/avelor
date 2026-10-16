//! The structural commands (`panel.md#saving-structure`): titles, stars,
//! archiving -- and
//! `reload_entity`, the conflict dialog's Use theirs, which ends like
//! them. Typed commands (`api.md#structure`): each edit goes through
//! `App::edit`, which pushes every entity it wrote (`entity`) before the
//! answer: `ok {id}`, `ok {conflict: {id, changed}}` if a file changed
//! outside the app (nothing is written), or an error.

use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::vault::SaveOutcome;
use serde::Serialize;

use crate::app::App;
use crate::protocol::Req;

// ----------------------------------------------------------------------------
/// The answer of a structural command.
#[derive(Serialize)]
pub struct Done {
    /// The entity the command is about: the entity itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Id>,
    /// Nothing was written: a file changed outside the app.
    #[serde(skip_serializing_if = "Option::is_none")]
    conflict: Option<Conflict>,
}

// ----------------------------------------------------------------------------
/// The entity whose file changed outside the app, to ask about: the
/// conflict dialog (`saving.md#the-dialog`).
#[derive(Serialize)]
struct Conflict {
    id: Id,
    /// When the file changed (ISO 8601), empty if it is gone.
    changed: String,
}

impl Conflict {
    // ------------------------------------------------------------------------
    /// The conflict over `id`, whose file changed outside the app.
    fn of(app: &App, id: Id) -> Self {
        Self {
            id,
            changed: app
                .vault
                .changed_at(&id)
                .map(|t| t.as_iso8601())
                .unwrap_or_default(),
        }
    }
}

// ----------------------------------------------------------------------------
/// Answer the edit's `result`: `id` if it was written (or had nothing to
/// write), the conflict if a file changed outside the app, else the error,
/// in the host's words.
fn answer(app: &App, req: Req, result: Result<SaveOutcome>, id: Option<Id>) {
    match result {
        Ok(SaveOutcome::Conflict(changed)) => {
            log::info!("Structural edit: {changed} changed outside the app");
            req.ok(Done {
                id: None,
                conflict: Some(Conflict::of(app, changed)),
            });
        }
        Ok(SaveOutcome::Saved | SaveOutcome::Unchanged) => req.ok(Done { id, conflict: None }),
        Err(err) => {
            log::warn!("Structural edit failed: {err}");
            let message = match err {
                Error::NotWritable { .. } => {
                    "Could not save the change: its file is read-only or its frontmatter can't be read."
                }
                Error::NotFound => "Could not save the change: it is no longer in the vault.",
                _ => "Could not save the change.",
            };
            req.fail(err, message);
        }
    }
}

// ----------------------------------------------------------------------------
/// `set_title`: the title of `id`, any entity; an empty title removes it.
pub fn set_title(app: &mut App, req: Req, id: Id, title: String) {
    let result = app.edit(|index, changes| index.set_title(&id, title, changes));
    answer(app, req, result, Some(id));
}

// ----------------------------------------------------------------------------
/// `set_starred`: star or unstar `id`, any entity; unstarring removes the
/// key.
pub fn set_starred(app: &mut App, req: Req, id: Id, starred: bool) {
    let result = app.edit(|index, changes| index.set_starred(&id, starred, changes));
    answer(app, req, result, Some(id));
}

// ----------------------------------------------------------------------------
/// `set_archived`: archive `id`, any entity, or take it out of the archive;
/// unarchiving removes the key.
pub fn set_archived(app: &mut App, req: Req, id: Id, archived: bool) {
    let result = app.edit(|index, changes| index.set_archived(&id, archived, changes));
    answer(app, req, result, Some(id));
}

// ----------------------------------------------------------------------------
/// `reload_entity`: read `id`'s file again and take in what it holds now,
/// the conflict dialog's Use theirs (`saving.md#the-dialog`). The entity as
/// it is now is pushed, or `entity_deleted` if the file is gone (a file
/// whose `id` was changed by hand comes back as a new entity); the answer
/// names the id the file has now.
pub fn reload_entity(app: &mut App, req: Req, id: Id) {
    match app.vault.reload(&id) {
        Ok(now) => {
            match now {
                Some(_) => log::info!("Reloaded {id}"),
                None => log::info!("Reloaded {id}: the file is gone"),
            }
            post_taken_in(app, &id, now);
            req.ok(Done {
                id: now,
                conflict: None,
            });
        }
        Err(err) => {
            log::warn!("Could not reload {id}: {err}");
            req.fail(err, "Could not load the version on disk.");
        }
    }
}

// ----------------------------------------------------------------------------
/// Send what reading `id`'s file again gave (`now`: the id the file has
/// now, `None` if it is gone): the entity, or `entity_deleted` -- both, if
/// its `id` was changed by hand.
fn post_taken_in(app: &App, id: &Id, now: Option<Id>) {
    if now != Some(*id) {
        app.post_entity_deleted(id);
    }
    if let Some(entry) = now.and_then(|now| app.vault.index().entry(&now)) {
        app.post_entity(entry);
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use crate::testing::Harness;
    use crate::testing::parse;
    use crate::testing::{notes, shape};
    use av_utils::error::Error;
    use av_utils::id::Id;
    use av_vault::vault::SaveOutcome;
    use serde_json::json;
    use std::fs;

    // ------------------------------------------------------------------------
    #[test]
    fn a_title_is_written_pushed_and_then_answered() {
        let (mut h, ids) = notes("set-title", 1);
        let sent = h.call("set_title", json!({ "id": ids[0], "title": "Renamed" }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let entity = &sent[0]["data"];
        assert_eq!(entity["id"], ids[0].to_string());
        assert_eq!(
            (&entity["title"], &entity["kind"]),
            (&json!("Renamed"), &json!("document"))
        );
        assert_eq!(sent[1], json!({ "re": 1, "ok": { "id": ids[0] } }));
        let text = fs::read_to_string(h.vault().join("Notes/n0.md")).unwrap();
        assert!(text.contains("title: Renamed"), "{text}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn starring_and_archiving_push_the_project_before_the_answer() {
        let project = Id::generate();
        let text = format!("id: {project}\nproject: {{}}\ntitle: Move\n");
        let mut h = Harness::with_vault("star-archive", &[("Projects/move.yaml", text)]);

        let sent = h.call("set_starred", json!({ "id": project, "starred": true }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        assert_eq!(sent[0]["data"]["starred"], true);
        let sent = h.call("set_archived", json!({ "id": project, "archived": true }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        assert_eq!(sent[0]["data"]["archived"], true);
        assert_eq!(sent[1]["ok"]["id"], project.to_string());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_file_changed_outside_is_answered_with_the_conflict() {
        let (mut h, ids) = notes("conflict", 1);
        let path = h.vault().join("Notes/n0.md");
        let theirs = format!(
            "---\nid: {}\ntitle: Theirs, longer\ndocument: {{}}\n---\nBody\n",
            ids[0]
        );
        fs::write(&path, &theirs).unwrap();

        let sent = h.call("set_title", json!({ "id": ids[0], "title": "Ours" }));
        assert_eq!(shape(&sent), ["reply"], "nothing written, nothing pushed");
        let conflict = &sent[0]["ok"]["conflict"];
        assert_eq!(conflict["id"], ids[0].to_string());
        assert!(!conflict["changed"].as_str().unwrap().is_empty());
        assert_eq!(fs::read_to_string(&path).unwrap(), theirs);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_failed_edit_is_answered_in_the_hosts_words() {
        let (mut h, _) = notes("failed", 1);
        let sent = h.call("set_title", json!({ "id": Id::generate(), "title": "X" }));
        assert_eq!(shape(&sent), ["reply"]);
        assert_eq!(sent[0]["err"]["code"], "not_found");
        assert_eq!(
            sent[0]["err"]["message"],
            "Could not save the change: it is no longer in the vault."
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn reloading_pushes_the_file_as_it_is_now() {
        let (mut h, ids) = notes("reload", 2);
        let path = h.vault().join("Notes/n0.md");
        let theirs = format!(
            "---\nid: {}\ntitle: Theirs\ndocument: {{}}\n---\nTheirs\n",
            ids[0]
        );
        fs::write(&path, theirs).unwrap();

        let sent = h.call("reload_entity", json!({ "id": ids[0] }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        assert_eq!(sent[0]["data"]["title"], "Theirs");
        assert_eq!(sent[1], json!({ "re": 1, "ok": { "id": ids[0] } }));

        // A file that is gone leaves the vault.
        fs::remove_file(h.vault().join("Notes/n1.md")).unwrap();
        let sent = h.call("reload_entity", json!({ "id": ids[1] }));
        assert_eq!(shape(&sent), ["entity_deleted", "reply"]);
        assert_eq!(sent[0]["data"]["id"], ids[1].to_string());
        assert_eq!(sent[1], json!({ "re": 1, "ok": {} }));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_edit_that_fails_part_way_still_pushes_what_it_wrote() {
        let (mut h, ids) = notes("part-way", 2);
        let edited = h.app.edit(|index, changes| {
            let written = index.set_title(&ids[0], "Written".into(), changes)?;
            assert_eq!(written, SaveOutcome::Saved);
            index.set_title(&Id::generate(), "Unknown".into(), changes)
        });
        assert!(matches!(edited, Err(Error::NotFound)));
        let sent = parse(&h.take());
        assert_eq!(shape(&sent), ["entity"]);
        assert_eq!(sent[0]["data"]["id"], ids[0].to_string());
    }
}
