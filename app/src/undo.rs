//! Undo and redo (`saving.md#undo`): each edit the user makes through a
//! command is a step, the files it wrote as `Changes` recorded them;
//! `undo` takes the last step back, `redo` does it again, each an edit
//! through `App::edit`. The steps are the open vault's, in memory.

use av_utils::error::Result;
use av_vault::vault::{Changes, FileStep, Index, Op, SaveOutcome};

use crate::app::App;
use crate::protocol::Req;
use crate::texts::Text;

/// The most steps kept; the oldest goes first.
const KEPT: usize = 100;

// ----------------------------------------------------------------------------
/// An edit the user made, named for the notices ("Renamed The Money Pit"):
/// several, when the page made them one group.
struct Step {
    name: Text,
    group: Option<String>,
    files: Vec<FileStep>,
}

// ----------------------------------------------------------------------------
/// The open vault's steps: the done ones to undo, the last on top, and the
/// undone ones to redo.
#[derive(Default)]
pub struct History {
    done: Vec<Step>,
    undone: Vec<Step>,
}

impl History {
    // ------------------------------------------------------------------------
    /// Keep `step`, the newest; the steps to redo go with it. One of the
    /// last step's group joins it.
    fn push(&mut self, step: Step) {
        self.undone.clear();
        if let Some(last) = self.done.last_mut() {
            if step.group.is_some() && last.group == step.group {
                let files = std::mem::take(&mut last.files);
                last.files = FileStep::joined(files, step.files);
                return;
            }
        }
        self.done.push(step);
        if self.done.len() > KEPT {
            self.done.remove(0);
        }
    }

    // ------------------------------------------------------------------------
    /// The step `op` takes: the last one done to undo, the last one undone
    /// to redo.
    fn take(&mut self, op: Op) -> Option<Step> {
        match op {
            Op::Undo => self.done.pop(),
            Op::Redo => self.undone.pop(),
        }
    }

    // ------------------------------------------------------------------------
    /// Keep `step`, which `op` took, for the other way.
    fn taken(&mut self, op: Op, step: Step) {
        match op {
            Op::Undo => self.undone.push(step),
            Op::Redo => self.done.push(step),
        }
    }
}

impl App {
    // ------------------------------------------------------------------------
    /// Run the user's edit `f`, the request `req`'s, as `App::edit` does,
    /// and keep what it wrote as the step `name`, to undo.
    pub fn step<R>(
        &mut self,
        req: &Req,
        name: Text,
        f: impl FnOnce(&mut Index, &mut Changes) -> Result<R>,
    ) -> Result<R> {
        let mut files = Vec::new();
        let result = self.edit(|index, changes| {
            let result = f(index, changes);
            files = std::mem::take(&mut changes.files);
            result
        });
        self.keep_step(req, name, files);
        result
    }

    // ------------------------------------------------------------------------
    /// Keep `files`, what the request `req`'s edit wrote, as the step
    /// `name`, to undo. An edit that wrote nothing is no step; one that
    /// failed part-way is the step of what it wrote.
    pub fn keep_step(&mut self, req: &Req, name: Text, files: Vec<FileStep>) {
        if !files.is_empty() {
            let group = req.group().map(str::to_string);
            self.vault.history.push(Step { name, group, files });
        }
    }
}

// ----------------------------------------------------------------------------
/// `undo` and `redo` (`Op::Undo`, `Op::Redo`): the last step taken back, or
/// the last one undone done again, said in the status bar (`notice`). A
/// step whose files changed since is said and dropped, as is one that
/// failed. Answers `null`.
pub fn replay(app: &mut App, req: Req, op: Op) {
    let Some(mut step) = app.vault.history.take(op) else {
        app.inform(match op {
            Op::Undo => Text::new("undo.nothing"),
            Op::Redo => Text::new("undo.nothing_redo"),
        });
        return req.ok(());
    };
    let result = app.edit(|index, changes| index.replay(&mut step.files, op, changes));
    let name = step.name.clone();
    match result {
        Ok(SaveOutcome::Saved | SaveOutcome::Unchanged) => {
            app.vault.history.taken(op, step);
            app.inform(match op {
                Op::Undo => Text::new("undo.undone").arg("step", name),
                Op::Redo => Text::new("undo.redone").arg("step", name),
            });
        }
        Ok(SaveOutcome::Conflict(id)) => {
            log::info!("Step not replayed: {id} changed since");
            app.notice(match op {
                Op::Undo => Text::new("undo.changed").arg("step", name),
                Op::Redo => Text::new("undo.changed_redo").arg("step", name),
            });
        }
        Err(err) => {
            log::warn!("Step not replayed: {err}");
            app.notice(match op {
                Op::Undo => Text::new("undo.failed").arg("step", name),
                Op::Redo => Text::new("undo.failed_redo").arg("step", name),
            });
        }
    }
    req.ok(());
}

#[cfg(test)]
mod tests {
    use crate::testing::{notes, shown};
    use serde_json::{Value, json};

    /// The notice the page got: its level and its text in English.
    fn notice(sent: &[Value]) -> (String, String) {
        let notice = sent.iter().find(|m| m["ev"] == "notice").expect("a notice");
        let level = notice["data"]["level"].as_str().unwrap().to_string();
        (level, shown(&notice["data"]["text"]))
    }

    #[test]
    fn undo_and_redo_say_what_they_did() {
        let (mut h, ids) = notes("undo-says", 1);
        let sent = h.call("undo", json!({}));
        assert_eq!(notice(&sent), ("info".into(), "Nothing to undo".into()));

        h.call("set_title", json!({ "id": ids[0], "title": "Renamed" }));
        let sent = h.call("undo", json!({}));
        assert_eq!(
            notice(&sent),
            ("info".into(), "Undone: Renamed Note 0".into())
        );
        assert_eq!(sent.last().unwrap(), &json!({ "re": 1, "ok": null }));
        let sent = h.call("redo", json!({}));
        assert_eq!(
            notice(&sent),
            ("info".into(), "Redone: Renamed Note 0".into())
        );
        let sent = h.call("redo", json!({}));
        assert_eq!(notice(&sent), ("info".into(), "Nothing to redo".into()));
    }

    #[test]
    fn a_step_whose_file_changed_since_is_dropped() {
        let (mut h, ids) = notes("undo-changed", 1);
        h.call("set_title", json!({ "id": ids[0], "title": "Renamed" }));
        let path = h.vault().join("Notes/n0.md");
        std::fs::write(&path, format!("---\nid: {}\ntitle: Theirs\n---\n", ids[0])).unwrap();
        let theirs = std::fs::read_to_string(&path).unwrap();

        let sent = h.call("undo", json!({}));
        let said = (
            "error".into(),
            "Not undone, changed since: Renamed Note 0".into(),
        );
        assert_eq!(notice(&sent), said);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), theirs);
        let sent = h.call("undo", json!({}));
        assert_eq!(notice(&sent).1, "Nothing to undo", "the step is gone");
    }

    #[test]
    fn a_new_edit_drops_the_steps_to_redo() {
        let (mut h, ids) = notes("undo-new-edit", 2);
        h.call("set_title", json!({ "id": ids[0], "title": "A" }));
        h.call("undo", json!({}));
        h.call("set_title", json!({ "id": ids[1], "title": "B" }));
        let sent = h.call("redo", json!({}));
        assert_eq!(notice(&sent).1, "Nothing to redo");
    }

    #[test]
    fn opening_a_vault_forgets_its_steps() {
        let (mut h, ids) = notes("undo-reopened", 1);
        h.call("set_title", json!({ "id": ids[0], "title": "Renamed" }));
        let root = h.vault();
        crate::api::switch_to(&mut h.app, root);
        h.wait_indexed();
        let sent = h.call("undo", json!({}));
        assert_eq!(notice(&sent).1, "Nothing to undo");
    }
}
