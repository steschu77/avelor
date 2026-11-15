//! Who the user is in the open vault (`ui.md#settings`): a contact of it.
//! `config.yaml`, which is the system's account's, keeps the contacts' ids in
//! `me`, at most one per vault. Ids are unique across vaults, so the one that
//! is a contact of the open vault is its own.

use av_utils::error::Error;
use av_utils::id::Id;
use av_vault::envelope::DocumentType;
use av_vault::shared::ContactRef;
use serde::Deserialize;

use crate::app::App;
use crate::protocol::Req;
use crate::texts::Text;

// ----------------------------------------------------------------------------
/// `set_me`'s arguments: a contact of the open vault, none for no one.
#[derive(Deserialize)]
pub struct Pick {
    contact: Option<Id>,
}

// ----------------------------------------------------------------------------
/// `set_me {contact}`: who the user is in the open vault. Those of other
/// vaults stay, which are told by what the index has: not while it is being
/// filled. Answers `me` as it is now.
pub fn set_me(app: &mut App, req: Req, pick: Pick) {
    if app.vault.root().is_none() {
        return req.fail(Error::NotInitialized, Text::new("vault.none_open"));
    }
    if app.indexing() {
        return req.fail(Error::NotInitialized, Text::new("settings.indexing"));
    }
    if let Some(id) = pick.contact {
        if app.vault.index().entry(&id).is_none() {
            return req.err(Error::NotFound);
        }
        if !is_contact(app, &id) {
            return req.err(Error::WrongKind);
        }
    }
    let here = |id: &&String| Id::parse(id).is_ok_and(|id| is_contact(app, &id));
    let others = app.config.me().iter().filter(|id| !here(id)).cloned();
    let me = others
        .chain(pick.contact.map(|id| id.to_string()))
        .collect();
    match app.config.set_me(me) {
        Ok(()) => req.ok(app.config.me()),
        Err(e) => req.save_failed(e, Text::new("save.settings")),
    }
}

// ----------------------------------------------------------------------------
/// The user as the open vault names them; none until one is picked, or its
/// contact is read.
pub fn by(app: &App) -> Option<ContactRef> {
    let ids = app.config.me().iter().filter_map(|id| Id::parse(id).ok());
    let mut here = ids.filter(|id| is_contact(app, id));
    here.next().map(ContactRef::Id)
}

// ----------------------------------------------------------------------------
/// Whether `id` is a contact of the open vault.
fn is_contact(app: &App, id: &Id) -> bool {
    let entry = app.vault.index().entry(id);
    entry.is_some_and(|entry| entry.doc.kind() == DocumentType::Contact)
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::by;
    use crate::config::load_config;
    use crate::testing::Harness;
    use crate::{api, chat, documents};
    use av_utils::id::Id;
    use av_vault::document::{Document, FileFormat};
    use av_vault::envelope::{ConversationMessage, Roles, Source};
    use av_vault::shared::ContactRef;
    use serde_json::{Value, json};
    use std::fs;
    use std::path::Path;

    const ALEX: &str = "01a0e1cc-a24c-7490-82fe-e75669985b01";
    const MAYA: &str = "01a0e1cc-a24c-7490-82fe-e75669985b02";
    const TASK: &str = "01a0e1cc-a24c-7490-82fe-e75669985b03";
    const OMA: &str = "01a0e1cc-a24c-7490-82fe-e75669985b04";

    // ------------------------------------------------------------------------
    fn contact(id: &str, name: &str) -> String {
        format!("---\nid: {id}\ntitle: {name}\ncontact:\n  type: person\n---\n")
    }

    // ------------------------------------------------------------------------
    /// A vault with Alex, Maya and a task, and Oma's beside it.
    fn household(name: &str) -> Harness {
        let h = Harness::with_vault(
            name,
            &[
                ("Contacts/Alex.md", contact(ALEX, "Alex")),
                ("Contacts/Maya.md", contact(MAYA, "Maya")),
                (
                    "Tasks/Shop.md",
                    format!("---\nid: {TASK}\ntitle: Shop\ntask: {{}}\n---\n"),
                ),
            ],
        );
        let other = h.dir().join("other/Contacts");
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("Oma.md"), contact(OMA, "Oma")).unwrap();
        h
    }

    // ------------------------------------------------------------------------
    fn set(h: &mut Harness, contact: Value) -> Value {
        h.take();
        let sent = h.call("set_me", json!({ "contact": contact }));
        sent.last().unwrap().clone()
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_user_is_one_contact_of_the_vault() {
        let mut h = household("me");
        assert_eq!(set(&mut h, json!(ALEX))["ok"], json!([ALEX]));
        assert_eq!(set(&mut h, json!(MAYA))["ok"], json!([MAYA]), "one of them");
        assert_eq!(load_config(h.dir()).unwrap().me(), [MAYA]);
        let ready = h.call("ready", json!({}));
        assert_eq!(ready.last().unwrap()["ok"]["me"], json!([MAYA]));
        // The vault opened again, and isn't read yet.
        let refused = set(&mut h, json!(null));
        assert_eq!(refused["err"]["text"]["key"], "settings.indexing");
        assert_eq!(load_config(h.dir()).unwrap().me(), [MAYA]);
        h.wait_indexed();
        assert_eq!(set(&mut h, json!(null))["ok"], json!([]));
        let kept = fs::read_to_string(h.dir().join("config.yaml")).unwrap();
        assert!(!kept.contains("me:"), "{kept}");
    }

    type Make = fn(&mut Harness);

    // ------------------------------------------------------------------------
    /// What makes new files in `household`'s vault, by a command or a writer
    /// of the app's own.
    fn makers() -> Vec<(&'static str, Make)> {
        fn command(h: &mut Harness, cmd: &str, args: Value) {
            let sent = h.call(cmd, args);
            assert!(sent.last().unwrap()["ok"].is_object(), "{cmd}: {sent:?}");
        }
        fn write(h: &mut Harness, source: Source, file: &str) {
            let roles = Roles {
                document: Some(Default::default()),
                ..Default::default()
            };
            let doc =
                Document::create(roles, source, file.into(), Vec::new(), FileFormat::Markdown);
            let folder = Path::new("Documents");
            h.app
                .edit(|index, changes| index.create(doc, folder, changes))
                .unwrap();
        }
        vec![
            ("entity", |h| {
                let roles = json!({ "task": { "status": "todo" } });
                command(
                    h,
                    "create_entity",
                    json!({ "title": "Call", "roles": roles }),
                );
            }),
            ("new contact", |h| {
                let roles = json!({ "contact": { "type": "person" } });
                command(
                    h,
                    "create_entity",
                    json!({ "title": "Ingrid", "roles": roles }),
                );
            }),
            ("project", |h| {
                command(h, "create_project", json!({ "title": "Move" }));
            }),
            ("thread", |h| {
                let members = json!([TASK, ALEX]);
                command(
                    h,
                    "create_thread",
                    json!({ "title": "Both", "members": members }),
                );
            }),
            ("conversation", |h| {
                let message = ConversationMessage {
                    id: Some(Id::generate()),
                    content: Some("Hi".into()),
                    ..Default::default()
                };
                chat::create(&mut h.app, "Chat".into(), vec![message], None).unwrap();
            }),
            ("conflict copy", |h| {
                let id = Id::parse(TASK).unwrap();
                documents::keep_conflict_copy(&mut h.app, id, None, |_| {}).unwrap();
            }),
            ("original", |h| {
                let doc = Document::create(
                    Roles::default(),
                    Source::Remote(Default::default()),
                    "Bill".into(),
                    Vec::new(),
                    FileFormat::Markdown,
                );
                let folder = Path::new("Attachments");
                let made = h.app.edit(|index, changes| {
                    index.create_with_original(doc, folder, "bill.pdf", b"%PDF", changes)
                });
                made.unwrap();
            }),
            ("remote", |h| {
                write(h, Source::Remote(Default::default()), "r")
            }),
            ("scan", |h| write(h, Source::Scan(Default::default()), "s")),
            ("import", |h| {
                write(h, Source::Import(Default::default()), "i")
            }),
        ]
    }

    // ------------------------------------------------------------------------
    #[test]
    fn what_is_added_names_the_user() {
        for (what, make) in makers() {
            for user in [None, Some(MAYA)] {
                let mut h = household(&format!("by-{what}-{}", user.is_some()));
                if user.is_some() {
                    set(&mut h, json!(user));
                }
                let before = h.files();
                make(&mut h);
                let files = h.files();
                let (known, made): (Vec<_>, Vec<_>) = files
                    .iter()
                    .filter(|(_, text)| text.contains("title:"))
                    .partition(|(path, _)| before.contains_key(*path));
                assert!(!made.is_empty(), "{what}");
                for (path, text) in made {
                    let by = text.matches("by: ").count();
                    assert_eq!(by, usize::from(user.is_some()), "{what} {path:?}: {text}");
                    assert!(
                        user.is_none() || text.contains(&format!("by: '@{MAYA}'")),
                        "{text}"
                    );
                }
                for (path, text) in known {
                    assert!(!text.contains("by: "), "{what} changed {path:?}: {text}");
                }
            }
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_entity_by_someone_else_stays_theirs() {
        let mut h = household("by-someone");
        set(&mut h, json!(MAYA));
        let mut source = Source::Created(Default::default());
        source.set_by(Some(ContactRef::parse("Oma")));
        let doc = Document::create(
            Roles::default(),
            source,
            "Letter".into(),
            Vec::new(),
            FileFormat::Markdown,
        );
        let folder = Path::new("Documents");
        let made = h
            .app
            .edit(|index, changes| index.create(doc, folder, changes))
            .unwrap();
        let entry = h.app.vault.index().entry(&made).unwrap();
        let by = entry.doc.source().and_then(|source| source.by());
        assert_eq!(by, Some(&ContactRef::Name("Oma".into())));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_user_no_contact_of_the_vault_any_more_is_no_one() {
        let mut h = household("by-gone");
        set(&mut h, json!(MAYA));
        assert_eq!(by(&h.app), Some(ContactRef::Id(Id::parse(MAYA).unwrap())));
        h.call("delete_entity", json!({ "id": MAYA }));
        assert_eq!(by(&h.app), None);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn each_vault_has_its_own() {
        let mut h = household("me-each");
        set(&mut h, json!(ALEX));
        let other = h.dir().join("other");
        api::switch_to(&mut h.app, other);
        h.wait_indexed();
        assert_eq!(set(&mut h, json!(OMA))["ok"], json!([ALEX, OMA]));
        assert_eq!(
            set(&mut h, json!(null))["ok"],
            json!([ALEX]),
            "only its own"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn what_a_hand_wrote_there_stays() {
        let mut h = household("me-by-hand");
        fs::write(h.dir().join("config.yaml"), "me: [someone, else]\n").unwrap();
        h.app.config = load_config(h.dir()).unwrap();
        assert_eq!(
            set(&mut h, json!(ALEX))["ok"],
            json!(["someone", "else", ALEX])
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn only_a_contact_of_the_open_vault_is_the_user() {
        let mut h = household("me-refused");
        assert_eq!(set(&mut h, json!(TASK))["err"]["code"], "wrong_kind");
        assert_eq!(set(&mut h, json!(OMA))["err"]["code"], "not_found");
        assert_eq!(set(&mut h, json!("Alex"))["err"]["code"], "bad_request");
        assert_eq!(load_config(h.dir()).unwrap().me(), [] as [&str; 0]);

        let mut none = Harness::new("me-no-vault");
        assert_eq!(
            set(&mut none, json!(null))["err"]["code"],
            "not_initialized"
        );
    }
}
