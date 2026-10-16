//! The structural commands (`panel.md#saving-structure`): new entities,
//! threads, projects, titles, stars, archiving, labels, links, deletes -- and
//! `reload_entity`, the conflict dialog's Use theirs, which ends like
//! them. Typed commands (`api.md#structure`): each edit goes through
//! `App::step`, `App::edit` keeping the step to undo, which pushes every
//! entity it wrote (`entity`, `entity_deleted`) before the answer:
//! `ok {id}`, `ok {conflict: {id, changed}}` if a file changed outside
//! the app (nothing is written), or an error.

use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_vault::vault::NewEntity;
use av_vault::vault::SaveOutcome;
use av_vault::vault::{Changes, Index};
use serde::Serialize;

use crate::app::App;
use crate::documents;
use crate::protocol::Req;
use crate::texts::Text;

// ----------------------------------------------------------------------------
/// The answer of a structural command.
#[derive(Serialize)]
pub struct Done {
    /// The entity the command is about: the new thread or project, the
    /// thread or project of a membership edit, the entity itself; none for
    /// a delete.
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
pub(crate) struct Conflict {
    id: Id,
    /// When the file changed (ISO 8601), empty if it is gone.
    changed: String,
}

impl Conflict {
    // ------------------------------------------------------------------------
    /// The conflict over `id`, whose file changed outside the app.
    pub(crate) fn of(app: &App, id: Id) -> Self {
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
            let text = match err {
                Error::NotWritable { .. } => Text::new("save.change_not_writable"),
                Error::NotFound => Text::new("save.change_gone"),
                _ => Text::new("save.change"),
            };
            req.fail(err, text);
        }
    }
}

// ----------------------------------------------------------------------------
/// What a step names `id` by (`saving.md#undo`): its title, or `instead`
/// while it has none.
pub(crate) fn named(app: &App, id: &Id, instead: &str) -> String {
    let title = app
        .vault
        .index()
        .get(id)
        .map(|d| d.title())
        .unwrap_or_default();
    if title.is_empty() { instead } else { title }.to_string()
}

// ----------------------------------------------------------------------------
/// `create_thread`: a thread of `members`, titled `title`; it is written
/// beside its first member (`panel.md#structural-edits`).
pub fn create_thread(app: &mut App, req: Req, title: String, members: Vec<Id>) {
    let mut created = None;
    let step = Text::new("undo.create_thread").arg("title", title.clone());
    let result = app.step(&req, step, |index, changes| {
        let outcome = index.create_thread(title, &members, changes);
        created = changes.created;
        outcome
    });
    answer(app, req, result, created);
}

// ----------------------------------------------------------------------------
/// `create_entity`: the entity `new`, made with New -- a document, task,
/// event, contact, list or record (`Index::create_new`), in its type's
/// folder.
pub fn create_entity(app: &mut App, req: Req, new: NewEntity) {
    let step = Text::new("undo.create_entity").arg("title", new.title.clone());
    let created = app.step(&req, step, |index, changes| index.create_new(new, changes));
    let id = created.as_ref().ok().copied();
    answer(app, req, created.map(|_| SaveOutcome::Saved), id);
}

// ----------------------------------------------------------------------------
/// `create_project`: a project titled `title`, in `Projects/`.
pub fn create_project(app: &mut App, req: Req, title: String) {
    let step = Text::new("undo.create_project").arg("title", title.clone());
    let created = app.step(&req, step, |index, changes| {
        index.create_project(title, changes)
    });
    let id = created.as_ref().ok().copied();
    answer(app, req, created.map(|_| SaveOutcome::Saved), id);
}

// ----------------------------------------------------------------------------
/// An entity joining or leaving a group -- a thread or a project; the
/// answer names the group. Leaving a thread dissolves it below two members.
/// The step `name` names both.
pub fn membership(
    app: &mut App,
    req: Req,
    id: Id,
    group: Id,
    name: Text,
    op: impl FnOnce(&mut Index, &Id, &Id, &mut Changes) -> Result<SaveOutcome>,
) {
    let step = name
        .arg("title", named(app, &id, ""))
        .arg("group", named(app, &group, ""));
    let result = app.step(&req, step, |index, changes| op(index, &id, &group, changes));
    answer(app, req, result, Some(group));
}

// ----------------------------------------------------------------------------
/// `set_title`: the title of `id`, any entity; an empty title removes it.
pub fn set_title(app: &mut App, req: Req, id: Id, title: String) {
    let step = Text::new("undo.set_title").arg("title", named(app, &id, &title));
    let result = app.step(&req, step, |index, changes| {
        index.set_title(&id, title, changes)
    });
    answer(app, req, result, Some(id));
}

// ----------------------------------------------------------------------------
/// `set_labels`: the labels of `id`, any entity (`Index::set_labels`); none
/// removes the key.
pub fn set_labels(app: &mut App, req: Req, id: Id, labels: Vec<String>) {
    let step = Text::new("undo.set_labels").arg("title", named(app, &id, ""));
    let result = app.step(&req, step, |index, changes| {
        index.set_labels(&id, labels, changes)
    });
    answer(app, req, result, Some(id));
}

// ----------------------------------------------------------------------------
/// `set_link`: link `id`, any entity, to `to`, another one in the vault
/// (`Index::set_link`); none removes the key.
pub fn set_link(app: &mut App, req: Req, id: Id, to: Option<Id>) {
    let step = Text::new("undo.set_link").arg("title", named(app, &id, ""));
    let result = app.step(&req, step, |index, changes| {
        index.set_link(&id, to, changes)
    });
    answer(app, req, result, Some(id));
}

// ----------------------------------------------------------------------------
/// `set_starred`: star or unstar `id`, any entity; unstarring removes the
/// key.
pub fn set_starred(app: &mut App, req: Req, id: Id, starred: bool) {
    let step = match starred {
        true => Text::new("undo.set_starred"),
        false => Text::new("undo.set_unstarred"),
    };
    let step = step.arg("title", named(app, &id, ""));
    let result = app.step(&req, step, |index, changes| {
        index.set_starred(&id, starred, changes)
    });
    answer(app, req, result, Some(id));
}

// ----------------------------------------------------------------------------
/// `set_archived`: archive `id`, any entity, or take it out of the archive;
/// unarchiving removes the key.
pub fn set_archived(app: &mut App, req: Req, id: Id, archived: bool) {
    let step = match archived {
        true => Text::new("undo.set_archived"),
        false => Text::new("undo.set_unarchived"),
    };
    let step = step.arg("title", named(app, &id, ""));
    let result = app.step(&req, step, |index, changes| {
        index.set_archived(&id, archived, changes)
    });
    answer(app, req, result, Some(id));
}

// ----------------------------------------------------------------------------
/// `delete_entity`: move `id`'s file to `.trash`; threads it leaves with
/// fewer than two members are dissolved (`panel.md#structural-edits`), and a
/// project's members leave it first.
pub fn delete_entity(app: &mut App, req: Req, id: Id) {
    let step = Text::new("undo.delete_entity").arg("title", named(app, &id, ""));
    let result = app.step(&req, step, |index, changes| index.delete(&id, changes));
    answer(app, req, result, None);
}

// ----------------------------------------------------------------------------
/// `reload_entity`: read `id`'s file again and take in what it holds now,
/// the conflict dialog's Use theirs (`saving.md#the-dialog`). The entity as it
/// is now and its content are pushed, or `entity_deleted` if the file is
/// gone (a file whose `id` was changed by hand comes back as a new entity);
/// the answer names the id the file has now. A save held for the conflict
/// is dropped with it.
pub fn reload_entity(app: &mut App, req: Req, id: Id) {
    match app.vault.reload(&id) {
        Ok(now) => {
            app.documents.release(&id);
            match now {
                Some(_) => log::info!("Reloaded {id}"),
                None => log::info!("Reloaded {id}: the file is gone"),
            }
            documents::post_taken_in(app, &id, now);
            req.ok(Done {
                id: now,
                conflict: None,
            });
        }
        Err(err) => {
            log::warn!("Could not reload {id}: {err}");
            req.fail(err, Text::new("documents.reload_failed"));
        }
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use crate::testing::parse;
    use crate::testing::trashed;
    use crate::testing::{notes, shape, shown, undoes};
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
    fn a_title_is_undone_and_redone() {
        let (mut h, ids) = notes("undo-title", 1);
        undoes(
            &mut h,
            "set_title",
            json!({ "id": ids[0], "title": "Renamed" }),
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_new_thread_and_its_members_are_pushed_before_the_answer() {
        let (mut h, ids) = notes("create-thread", 2);
        let sent = h.call("create_thread", json!({ "title": "Trip", "members": ids }));
        assert_eq!(shape(&sent), ["entity", "entity", "entity", "reply"]);
        assert_eq!(sent[0]["data"]["kind"], "thread");
        assert_eq!(sent[3]["ok"]["id"], sent[0]["data"]["id"]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_thread_made_or_dissolved_is_undone() {
        let (mut h, ids) = notes("undo-thread", 2);
        let sent = undoes(
            &mut h,
            "create_thread",
            json!({ "title": "Trip", "members": ids }),
        );
        let thread = sent[0]["data"]["id"].as_str().unwrap().to_string();
        let sent = h.call("undo", json!({}));
        assert_eq!(
            shape(&sent),
            ["entity", "entity", "entity_deleted", "notice", "reply"],
            "the members left it, the thread went to the trash"
        );
        h.call("redo", json!({}));

        // Leaving dissolves it; the undo brings it back from the trash.
        let args = json!({ "id": ids[0], "thread": thread });
        let sent = undoes(&mut h, "leave_thread", args);
        assert!(shape(&sent).contains(&"entity_deleted"), "{sent:?}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn deleting_a_member_pushes_the_dissolved_thread_too() {
        let (mut h, ids) = notes("delete", 2);
        let sent = h.call("create_thread", json!({ "title": "Trip", "members": ids }));
        let thread = sent[0]["data"]["id"].as_str().unwrap().to_string();

        let sent = h.call("delete_entity", json!({ "id": ids[0] }));
        assert_eq!(
            shape(&sent),
            ["entity", "entity_deleted", "entity_deleted", "reply"]
        );
        assert_eq!(sent[0]["data"]["id"], ids[1].to_string(), "the member left");
        let mut deleted = [&sent[1]["data"]["id"], &sent[2]["data"]["id"]].map(|v| v.to_string());
        deleted.sort();
        let mut expected = [json!(thread), json!(ids[0])].map(|v| v.to_string());
        expected.sort();
        assert_eq!(deleted, expected);
        assert_eq!(sent[3], json!({ "re": 1, "ok": {} }));
        assert!(trashed(&h.vault(), "Notes/n0.md"));

        // The undo brings back the member, its thread and the other member's
        // place in it.
        h.call("undo", json!({}));
        h.call("redo", json!({}));
        let sent = h.call("undo", json!({}));
        let back = shape(&sent).iter().filter(|e| **e == "entity").count();
        assert_eq!(back, 3, "{sent:?}");
        assert!(!trashed(&h.vault(), "Notes/n0.md"));
        let thread_file = h
            .files()
            .into_keys()
            .filter(|p| p.extension().is_some_and(|e| e == "yaml"));
        assert_eq!(thread_file.count(), 1, "the thread back");
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
            shown(&sent[0]["err"]["text"]),
            "Could not save the change: it is no longer in the vault."
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_new_project_is_undone_and_made_again() {
        let (mut h, _) = notes("undo-new-project", 1);
        let sent = undoes(&mut h, "create_project", json!({ "title": "Move" }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let project = sent[1]["ok"]["id"].as_str().unwrap().to_string();

        let sent = h.call("undo", json!({}));
        assert_eq!(shape(&sent), ["entity_deleted", "notice", "reply"]);
        assert_eq!(sent[0]["data"]["id"], project);
        let sent = h.call("redo", json!({}));
        assert_eq!(shape(&sent), ["entity", "notice", "reply"]);
        assert_eq!(sent[0]["data"]["id"], project);
        assert_eq!(shown(&sent[1]["data"]["text"]), "Redone: Created Move");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_new_task_is_pushed_before_the_answer() {
        let (mut h, ids) = notes("create-entity", 1);
        let sent = h.call(
            "create_entity",
            json!({
                "roles": { "task": { "status": "todo", "due": "2026-10-09" } },
                "title": "Call the landlord",
                "labels": ["home"],
                "linked_to": ids[0],
            }),
        );
        assert_eq!(shape(&sent), ["entity", "reply"]);
        let entity = &sent[0]["data"];
        assert_eq!(entity["kind"], "task");
        assert_eq!(entity["source"], json!({ "created": {} }));
        assert_eq!(entity["linked_to"], ids[0].to_string());
        assert_eq!(entity["task"]["status"], "todo");
        assert!(
            entity["path"].as_str().unwrap().starts_with("Tasks"),
            "{entity}"
        );
        assert_eq!(sent[1]["ok"]["id"], entity["id"]);

        let args = json!({ "roles": { "task": { "status": "todo" } }, "title": "Pay the rent" });
        undoes(&mut h, "create_entity", args);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_new_entity_of_another_type_is_refused() {
        let (mut h, _) = notes("create-refused", 1);
        let args = json!({ "roles": { "project": {} }, "title": "Move" });
        let sent = h.call("create_entity", args);
        assert_eq!(shape(&sent), ["reply"]);
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn joining_a_project_answers_with_the_project() {
        let (mut h, ids) = notes("join-project", 1);
        let sent = h.call("create_project", json!({ "title": "Move" }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        assert_eq!(sent[0]["data"]["kind"], "project");
        let project = sent[1]["ok"]["id"].clone();

        let sent = h.call("join_project", json!({ "id": ids[0], "project": project }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        assert_eq!(sent[0]["data"]["projects"], json!([project]));
        assert_eq!(sent[1]["ok"]["id"], project);

        let args = json!({ "id": ids[0], "project": project });
        undoes(&mut h, "leave_project", args.clone());
        undoes(&mut h, "join_project", args);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn archiving_pushes_the_project_before_the_answer() {
        let (mut h, _) = notes("archive", 1);
        let sent = h.call("create_project", json!({ "title": "Move" }));
        assert_eq!(sent[0]["data"]["archived"], false);
        let project = sent[1]["ok"]["id"].clone();

        let sent = h.call("set_archived", json!({ "id": project, "archived": true }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        assert_eq!(sent[0]["data"]["archived"], true);
        assert_eq!(sent[1]["ok"]["id"], project);

        undoes(
            &mut h,
            "set_archived",
            json!({ "id": project, "archived": false }),
        );
        undoes(
            &mut h,
            "set_starred",
            json!({ "id": project, "starred": true }),
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn deleting_a_project_pushes_its_members_then_the_project() {
        let (mut h, ids) = notes("delete-project", 2);
        let sent = h.call("create_project", json!({ "title": "Move" }));
        let project = sent[1]["ok"]["id"].clone();
        for id in &ids {
            h.call("join_project", json!({ "id": id, "project": project }));
        }

        let sent = h.call("delete_entity", json!({ "id": project }));
        assert_eq!(
            shape(&sent),
            ["entity", "entity", "entity_deleted", "reply"]
        );
        assert_eq!(sent[0]["data"]["projects"], json!([]));
        assert_eq!(sent[1]["data"]["projects"], json!([]));
        assert_eq!(sent[2]["data"]["id"], project);
        assert_eq!(sent[3], json!({ "re": 1, "ok": {} }));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn labels_are_set_on_the_entity_and_pushed_before_the_answer() {
        let (mut h, ids) = notes("set-labels", 1);
        let sent = h.call(
            "set_labels",
            json!({ "id": ids[0], "labels": ["bill", " paid "] }),
        );
        assert_eq!(shape(&sent), ["entity", "reply"]);
        assert_eq!(sent[0]["data"]["labels"], json!(["bill", "paid"]));
        assert_eq!(sent[0]["data"]["writable"], true);
        assert_eq!(sent[1], json!({ "re": 1, "ok": { "id": ids[0] } }));
        let text = fs::read_to_string(h.vault().join("Notes/n0.md")).unwrap();
        assert!(text.contains("- bill") && text.contains("- paid"), "{text}");

        undoes(
            &mut h,
            "set_labels",
            json!({ "id": ids[0], "labels": ["tax"] }),
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_link_is_written_pushed_and_removed() {
        let (mut h, ids) = notes("set-link", 2);
        let sent = h.call("set_link", json!({ "id": ids[0], "to": ids[1] }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        assert_eq!(sent[0]["data"]["linked_to"], ids[1].to_string());
        assert_eq!(sent[1], json!({ "re": 1, "ok": { "id": ids[0] } }));
        let text = fs::read_to_string(h.vault().join("Notes/n0.md")).unwrap();
        assert!(text.contains(&format!("linked-to: {}", ids[1])), "{text}");

        let sent = h.call("set_link", json!({ "id": ids[0], "to": null }));
        assert_eq!(shape(&sent), ["entity", "reply"]);
        assert_eq!(sent[0]["data"]["linked_to"], json!(null));
        let text = fs::read_to_string(h.vault().join("Notes/n0.md")).unwrap();
        assert!(!text.contains("linked-to"), "{text}");

        undoes(&mut h, "set_link", json!({ "id": ids[0], "to": ids[1] }));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_link_to_itself_or_outside_the_vault_is_refused() {
        let (mut h, ids) = notes("set-link-refused", 1);
        for to in [ids[0], Id::generate()] {
            let sent = h.call("set_link", json!({ "id": ids[0], "to": to }));
            assert_eq!(shape(&sent), ["reply"], "nothing written, nothing pushed");
            assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        }
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
        assert_eq!(shape(&sent), ["entity", "content", "reply"]);
        assert_eq!(sent[0]["data"]["title"], "Theirs");
        assert_eq!(sent[1]["data"]["source"], "Theirs\n");
        assert_eq!(sent[2], json!({ "re": 1, "ok": { "id": ids[0] } }));

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
