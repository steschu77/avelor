//! Markdown + YAML frontmatter: splitting, parsing, and composing.
//!
//! The envelope models the common keys from
//! vault.md#metadata. Unknown
//! keys are preserved verbatim in `extra` so round-tripping a document
//! through the app never loses user data.

use crate::envelope::Conversation;
use crate::envelope::Event;
use crate::envelope::Message;
use crate::envelope::Remote;
use crate::envelope::Task;
use crate::envelope::{DocumentType, Envelope, Roles, Source};
use crate::shared::{ContactRef, Genre};
use crate::vault::FileStamp;
use av_utils::datetime::DateTime;
use av_utils::error::Result;
use av_utils::id::Id;

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileFormat {
    Markdown,
    YAML,
}

// ----------------------------------------------------------------------------
/// The roles of a document without frontmatter.
static NO_ROLES: Roles = Roles {
    project: None,
    thread: None,
    calendar: None,
    bank_account: None,
    document: None,
    message: None,
    transactions: None,
    event: None,
    task: None,
    contact: None,
    record: None,
    list: None,
    email: None,
    conversation: None,
};

// ----------------------------------------------------------------------------
/// An entity: its frontmatter (the envelope) and its body. The envelope is
/// the only copy of the metadata; it is read through the accessors and
/// changed through the setters, which keep the file's shape (vault.md#files).
#[derive(Debug)]
pub struct Document {
    /// The entity's identity in the index: the envelope's `id`, or one made
    /// up for a file without (a new one at every scan).
    id: Id,
    /// The frontmatter; `None` for a file without (or with broken)
    /// frontmatter, which stays without one until metadata is set.
    envelope: Option<Envelope>,
    content: String,
    format: FileFormat,
    /// What the file held when the app last read or wrote it; `None` for an
    /// entity made in the app that isn't written yet.
    disk: Option<OnDisk>,
    err: Result<()>,
}

// ----------------------------------------------------------------------------
/// The file as the app last read or wrote it.
#[derive(Debug)]
struct OnDisk {
    envelope: Option<Envelope>,
    content: String,
    /// Its modified time and size then; `None` if it couldn't be read,
    /// which makes the next write a conflict: the safe side.
    stamp: Option<FileStamp>,
}

// ----------------------------------------------------------------------------
impl Document {
    // ------------------------------------------------------------------------
    /// The document `text` of a file in `format`. Its stamp is set by the
    /// reader (`stamped`).
    pub fn load(text: String, format: FileFormat) -> Self {
        let (envelope, content, err) = parse_document(format, &text);
        let content = content.to_string();
        let id = envelope
            .as_ref()
            .and_then(|e| e.id)
            .unwrap_or_else(Id::generate);
        let disk = OnDisk {
            envelope: envelope.clone(),
            content: content.clone(),
            stamp: None,
        };
        Self {
            id,
            envelope,
            content,
            format,
            disk: Some(disk),
            err,
        }
    }

    // ------------------------------------------------------------------------
    /// A new entity made in the app (not loaded from a file): a fresh `id`,
    /// dated now, already adopted, with the envelope built from the given
    /// fields. It has no on-disk state yet, so it is dirty until saved.
    pub fn create(
        roles: Roles,
        source: Source,
        title: String,
        projects: Vec<Id>,
        format: FileFormat,
    ) -> Self {
        let id = Id::generate();
        let envelope = Envelope {
            id: Some(id),
            title: Some(title),
            date: Some(DateTime::now()),
            projects,
            source: Some(source),
            roles,
            ..Default::default()
        };
        Self {
            id,
            envelope: Some(envelope),
            content: String::new(),
            format,
            disk: None,
            err: Ok(()),
        }
    }

    // ------------------------------------------------------------------------
    /// The document as read from a file stamped `stamp`.
    pub(crate) fn stamped(mut self, stamp: Option<FileStamp>) -> Self {
        if let Some(disk) = &mut self.disk {
            disk.stamp = stamp;
        }
        self
    }

    // ------------------------------------------------------------------------
    /// The document was written to its file, now stamped `stamp`: what it
    /// holds is what is on disk.
    pub(crate) fn written(&mut self, stamp: Option<FileStamp>) {
        self.disk = Some(OnDisk {
            envelope: self.envelope.clone(),
            content: self.content.clone(),
            stamp,
        });
    }

    // ------------------------------------------------------------------------
    /// The file's stamp at the last read or write.
    pub(crate) fn stamp(&self) -> Option<FileStamp> {
        self.disk.as_ref().and_then(|disk| disk.stamp)
    }

    // ------------------------------------------------------------------------
    /// Give a document without `id` the identity `id` (a file read again
    /// keeps its entity).
    pub(crate) fn keep_id(&mut self, id: Id) {
        self.id = id;
    }

    // ------------------------------------------------------------------------
    /// The editable state -- envelope and body -- to put back with
    /// `restore` if a write doesn't go through.
    pub(crate) fn snapshot(&self) -> (Option<Envelope>, String) {
        (self.envelope.clone(), self.content.clone())
    }

    // ------------------------------------------------------------------------
    pub(crate) fn restore(&mut self, (envelope, content): (Option<Envelope>, String)) {
        self.envelope = envelope;
        self.content = content;
    }

    // ------------------------------------------------------------------------
    pub fn id(&self) -> Id {
        self.id
    }

    // ------------------------------------------------------------------------
    /// The envelope, the document's frontmatter; `None` without one.
    pub fn envelope(&self) -> Option<&Envelope> {
        self.envelope.as_ref()
    }

    // ------------------------------------------------------------------------
    /// The title; empty without one.
    pub fn title(&self) -> &str {
        self.envelope
            .as_ref()
            .and_then(|e| e.title.as_deref())
            .unwrap_or("")
    }

    // ------------------------------------------------------------------------
    pub fn date(&self) -> Option<DateTime> {
        self.envelope.as_ref().and_then(|e| e.date)
    }

    // ------------------------------------------------------------------------
    pub fn source(&self) -> Option<&Source> {
        self.envelope.as_ref().and_then(|e| e.source.as_ref())
    }

    // ------------------------------------------------------------------------
    /// The `remote:` source block (vault.md#remote), if the entity came
    /// from `account` -- or from any account, for `None`.
    pub fn remote_from(&self, account: Option<&Id>) -> Option<&Remote> {
        match self.source() {
            Some(Source::Remote(remote))
                if account.is_none_or(|a| remote.account.as_ref() == Some(a)) =>
            {
                Some(remote)
            }
            _ => None,
        }
    }

    // ------------------------------------------------------------------------
    pub fn projects(&self) -> &[Id] {
        self.envelope.as_ref().map_or(&[], |e| &e.projects)
    }

    // ------------------------------------------------------------------------
    pub fn tags(&self) -> &[String] {
        self.envelope.as_ref().map_or(&[], |e| &e.tags)
    }

    // ------------------------------------------------------------------------
    /// The entity this one is linked to (`linked-to`).
    pub fn linked_to(&self) -> Option<Id> {
        self.envelope.as_ref().and_then(|e| e.linked_to)
    }

    // ------------------------------------------------------------------------
    /// The `rename` key.
    pub fn rename(&self) -> Option<bool> {
        self.envelope.as_ref().and_then(|e| e.rename)
    }

    // ------------------------------------------------------------------------
    /// Whether the entity is starred; a missing key is not starred.
    pub fn starred(&self) -> bool {
        self.envelope
            .as_ref()
            .and_then(|e| e.starred)
            .unwrap_or(false)
    }

    // ------------------------------------------------------------------------
    /// Whether the entity is archived; a missing key is not archived.
    pub fn archived(&self) -> bool {
        self.envelope
            .as_ref()
            .and_then(|e| e.archived)
            .unwrap_or(false)
    }

    // ------------------------------------------------------------------------
    pub fn roles(&self) -> &Roles {
        self.envelope.as_ref().map_or(&NO_ROLES, |e| &e.roles)
    }

    // ------------------------------------------------------------------------
    /// The body: markdown, empty for a YAML file -- or a broken file's whole
    /// text.
    pub fn content(&self) -> &str {
        &self.content
    }

    // ------------------------------------------------------------------------
    pub fn format(&self) -> FileFormat {
        self.format
    }

    // ------------------------------------------------------------------------
    /// Whether the entity has its own `id`.
    pub fn adopted(&self) -> bool {
        self.envelope.as_ref().is_some_and(|e| e.id.is_some())
    }

    // ------------------------------------------------------------------------
    /// Why the frontmatter couldn't be read, if it couldn't.
    pub fn error(&self) -> Option<&av_utils::error::Error> {
        self.err.as_ref().err()
    }

    // ------------------------------------------------------------------------
    /// The derived type: from the roles, or -- without any role -- a plain
    /// document (markdown) or a plain record (YAML).
    pub fn kind(&self) -> DocumentType {
        self.roles().derived_type().unwrap_or(match self.format {
            FileFormat::Markdown => DocumentType::Document,
            FileFormat::YAML => DocumentType::Record,
        })
    }

    // ------------------------------------------------------------------------
    pub fn save(&self) -> String {
        write_content(&self.format, &self.envelope, &self.content)
    }

    // ------------------------------------------------------------------------
    /// Whether the document differs from what its file held at the last
    /// read or write; one made in the app is, until it is written.
    pub fn is_dirty(&self) -> bool {
        self.disk
            .as_ref()
            .is_none_or(|disk| disk.envelope != self.envelope || disk.content != self.content)
    }

    // ------------------------------------------------------------------------
    pub fn is_broken(&self) -> bool {
        self.err.is_err()
    }

    // ------------------------------------------------------------------------
    /// Replace the body.
    pub fn set_content(&mut self, content: String) {
        self.content = content;
    }

    // ------------------------------------------------------------------------
    /// Set the genre of the `document` role; `None` removes it. A document
    /// without roles (a plain markdown file) gets the role for a genre. Only
    /// a document has a genre: for any other entity nothing changes and
    /// `false` is returned.
    pub fn set_genre(&mut self, genre: Option<Genre>) -> bool {
        if self.kind() != DocumentType::Document {
            return false;
        }
        let role = self
            .envelope
            .as_mut()
            .and_then(|e| e.roles.document.as_mut());
        match (role, genre) {
            (Some(document), genre) => document.genre = genre,
            (None, Some(genre)) => {
                let roles = &mut self.envelope_mut().roles;
                roles.document.get_or_insert_with(Default::default).genre = Some(genre);
            }
            (None, None) => {}
        }
        true
    }

    // ------------------------------------------------------------------------
    /// Set a task's `status` (vault.md#task): becoming `done` writes when
    /// (`done`, `now`), any other status removes it. Nothing changes for an
    /// entity that isn't a task, or a done task set to done again.
    pub fn set_task_status(&mut self, status: String, now: DateTime) {
        let Some(task) = self.task_mut() else {
            return;
        };
        if status != "done" {
            task.done = None;
        } else if task.status.as_deref() != Some("done") {
            task.done = Some(now);
        }
        task.status = Some(status);
    }

    // ------------------------------------------------------------------------
    /// Set a task's `due` date; `None` removes it. Nothing changes for an
    /// entity that isn't a task.
    pub fn set_task_due(&mut self, due: Option<DateTime>) {
        if let Some(task) = self.task_mut() {
            task.due = due;
        }
    }

    // ------------------------------------------------------------------------
    /// Set a task's `assignee`; `None` removes it. Nothing changes for an
    /// entity that isn't a task.
    pub fn set_task_assignee(&mut self, assignee: Option<ContactRef>) {
        if let Some(task) = self.task_mut() {
            task.assignee = assignee;
        }
    }

    // ------------------------------------------------------------------------
    /// The `task` block to change, if the entity has one.
    fn task_mut(&mut self) -> Option<&mut Task> {
        self.envelope.as_mut().and_then(|e| e.roles.task.as_mut())
    }

    // ------------------------------------------------------------------------
    /// Make the entity a member of `thread`, adding the `message` role if
    /// needed (vault.md#message).
    pub fn join_thread(&mut self, thread: Id) {
        let roles = &mut self.envelope_mut().roles;
        let message = roles.message.get_or_insert_with(Message::default);
        if !message.threads.contains(&thread) {
            message.threads.push(thread);
        }
    }

    // ------------------------------------------------------------------------
    /// Remove the entity from `thread`; drops the `message` role once its
    /// block is empty.
    pub fn leave_thread(&mut self, thread: &Id) {
        let Some(roles) = self.envelope.as_mut().map(|e| &mut e.roles) else {
            return;
        };
        let Some(message) = roles.message.as_mut() else {
            return;
        };
        message.threads.retain(|t| t != thread);
        if message.is_empty() {
            roles.message = None;
        }
    }

    // ------------------------------------------------------------------------
    /// Set the entity's `conversation` block (vault.md#conversation);
    /// `None` drops the role.
    pub fn set_conversation(&mut self, conversation: Option<Conversation>) {
        self.envelope_mut().roles.conversation = conversation;
    }

    // ------------------------------------------------------------------------
    /// Set the entity's `event` block (vault.md#event); `None` drops
    /// the role.
    pub fn set_event(&mut self, event: Option<Event>) {
        self.envelope_mut().roles.event = event;
    }

    // ------------------------------------------------------------------------
    /// Set the entity's source block (vault.md#source).
    pub fn set_source(&mut self, source: Option<Source>) {
        self.envelope_mut().source = source;
    }

    // ------------------------------------------------------------------------
    pub fn set_tags(&mut self, tags: Vec<String>) {
        self.envelope_mut().tags = tags;
    }

    // ------------------------------------------------------------------------
    /// Link the entity to `to`, replacing an older link; `None` removes the
    /// `linked-to` key.
    pub fn set_link(&mut self, to: Option<Id>) {
        match to {
            Some(to) => self.envelope_mut().linked_to = Some(to),
            None => {
                if let Some(envelope) = &mut self.envelope {
                    envelope.linked_to = None;
                }
            }
        }
    }

    // ------------------------------------------------------------------------
    /// Set the entity's title. An empty (or all-whitespace) title removes the
    /// `title` key altogether; a missing title loads as `""` again.
    pub fn set_title(&mut self, title: String) {
        let title = title.trim().to_string();
        self.envelope_mut().title = (!title.is_empty()).then_some(title);
    }

    // ------------------------------------------------------------------------
    /// Star or unstar the entity. Unstarring removes the `starred` key; a
    /// missing key loads as not starred.
    pub fn set_starred(&mut self, starred: bool) {
        self.envelope_mut().starred = starred.then_some(true);
    }

    // ------------------------------------------------------------------------
    /// Archive the entity or take it out of the archive. Unarchiving removes
    /// the `archived` key; a missing key loads as not archived.
    pub fn set_archived(&mut self, archived: bool) {
        self.envelope_mut().archived = archived.then_some(true);
    }

    // ------------------------------------------------------------------------
    pub fn set_date(&mut self, date: DateTime) {
        self.envelope_mut().date = Some(date);
    }

    // ------------------------------------------------------------------------
    /// Add the entity to `project`.
    pub fn join_project(&mut self, project: Id) {
        let projects = &mut self.envelope_mut().projects;
        if !projects.contains(&project) {
            projects.push(project);
        }
    }

    // ------------------------------------------------------------------------
    /// Remove the entity from `project`.
    pub fn leave_project(&mut self, project: &Id) {
        if let Some(envelope) = &mut self.envelope {
            envelope.projects.retain(|p| p != project);
        }
    }

    // ------------------------------------------------------------------------
    /// The envelope to change; a file without frontmatter gets one.
    fn envelope_mut(&mut self) -> &mut Envelope {
        self.envelope.get_or_insert_default()
    }
}

// ----------------------------------------------------------------------------
/// Split a markdown document into (frontmatter YAML, body). The frontmatter
/// block is delimited by `---` lines and must start on the very first line.
/// We allow any characters after the delimiters `---`:
/// `--- abc` is accepted but removed in a round-trip.
fn split_markdown(text: &str) -> (Option<&str>, &str) {
    if !text.starts_with("---") {
        return (None, text);
    }
    let Some(pos) = text.find("\n") else {
        return (None, text);
    };
    let yaml = &text[pos + 1..];
    if let Some(end) = yaml.find("\n---") {
        let rest = &yaml[end + 4..];
        if let Some(pos) = rest.find("\n") {
            return (Some(&yaml[..end + 1]), &rest[pos + 1..]);
        } else {
            return (Some(&yaml[..end + 1]), "");
        }
    }
    (None, text)
}

// ----------------------------------------------------------------------------
fn combine_markdown(yaml: Option<String>, body: &str) -> String {
    match yaml {
        Some(yaml) => format!("---\n{yaml}---\n{body}"),
        None => body.to_string(),
    }
}

// ----------------------------------------------------------------------------
fn parse_document(doc_type: FileFormat, text: &str) -> (Option<Envelope>, &str, Result<()>) {
    match doc_type {
        FileFormat::Markdown => {
            let (yaml, content) = split_markdown(text);
            if let Some(yaml) = yaml {
                match Envelope::parse(yaml) {
                    Ok(env) => (Some(env), content, Ok(())),
                    Err(err) => (None, text, Err(err)),
                }
            } else {
                (None, text, Ok(()))
            }
        }
        FileFormat::YAML => match Envelope::parse(text) {
            Ok(env) => (Some(env), "", Ok(())),
            Err(err) => (None, text, Err(err)),
        },
    }
}

// ----------------------------------------------------------------------------
fn write_content(doc_type: &FileFormat, envelope: &Option<Envelope>, content: &str) -> String {
    let yaml = envelope.as_ref().and_then(|e| e.compose());
    match doc_type {
        FileFormat::Markdown => {
            let body = content;
            combine_markdown(yaml, body)
        }
        FileFormat::YAML => yaml.unwrap_or(content.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::Empty;

    const DOC: &str = "---\nid: 01a06d07-2b70-708c-9c71-7ddfaa46c0f5\ntitle: Test\ntags: [a, b]\ncustom_key: kept\n---\nBody line.\n";
    const ID: &str = "01a06d07-2b70-708c-9c71-7ddfaa46c0f5";

    #[test]
    fn splits_frontmatter_and_body() {
        let (yaml, body) = split_markdown(DOC);
        assert!(yaml.unwrap().contains(ID));
        assert_eq!(body, "Body line.\n");
    }

    #[test]
    fn frontmatter_with_whitespace() {
        let (yaml, body) = split_markdown("--- \ntitle: Test\n---\t\nbody\n---");
        assert!(yaml.is_some());
        assert_eq!(body, "body\n---");
    }

    #[test]
    fn no_frontmatter_passes_through() {
        let (yaml, body) = split_markdown("just text\n---\nnot frontmatter");
        assert!(yaml.is_none());
        assert!(body.starts_with("just text"));
    }

    #[test]
    fn invalid_frontmatter_degrades_to_plain_document() {
        let text = "---\n: [ broken\n---\nbody\n";
        let (envelope, body, err) = parse_document(FileFormat::Markdown, text);
        assert!(envelope.is_none());
        assert_eq!(body, text);
        assert!(err.is_err());
    }

    #[test]
    fn load_extracts_roles_and_keeps_unknown_extra() {
        let text = "---\nid: 01a06d07-2b70-708c-9c71-7ddfaa46c0f5\nevent:\n  status: confirmed\n  location: Office\ncustom_key: kept\n---\nAgenda\n";
        let doc = Document::load(text.to_string(), FileFormat::Markdown);
        let event = doc.roles().event.as_ref().unwrap();
        assert_eq!(event.status.as_deref(), Some("confirmed"));
        assert_eq!(event.location.as_deref(), Some("Office"));
        assert_eq!(doc.kind(), DocumentType::Event);

        let envelope = doc.envelope().unwrap();
        assert!(
            !envelope.extra.contains_key("event")
                && !envelope.extra.contains_key("status")
                && !envelope.extra.contains_key("location"),
            "known role fields must not remain in extra"
        );
        assert_eq!(
            envelope.extra.get("custom_key").and_then(|v| v.as_str()),
            Some("kept")
        );
    }

    #[test]
    fn create_builds_a_dirty_adopted_entity() {
        let project = Id::generate();
        let roles = Roles {
            thread: Some(crate::envelope::Thread {}),
            ..Default::default()
        };
        let doc = Document::create(
            roles,
            Source::Created(Empty {}),
            "New thread".into(),
            vec![project],
            FileFormat::YAML,
        );

        assert_eq!(doc.kind(), DocumentType::Thread);
        assert!(doc.adopted() && doc.is_dirty() && !doc.is_broken());
        assert!(doc.date().is_some());
        let saved = doc.save();
        assert!(saved.contains(&format!("id: {}", doc.id())));
        assert!(saved.contains("created: {}"));
        assert!(saved.contains("thread: {}"));
        assert!(saved.contains(&project.to_string()));
    }

    #[test]
    fn kind_falls_back_on_file_format() {
        let md = Document::load("---\ntitle: x\n---\n".into(), FileFormat::Markdown);
        assert_eq!(md.kind(), DocumentType::Document);
        let yaml = Document::load("title: x\n".into(), FileFormat::YAML);
        assert_eq!(yaml.kind(), DocumentType::Record);
    }

    #[test]
    fn set_genre_requires_a_document() {
        let mut doc = Document::load("---\ntask: {}\n---\n".into(), FileFormat::Markdown);
        assert!(!doc.set_genre(Some(Genre::Letter)));

        let mut doc = Document::load("---\ndocument: {}\n---\n".into(), FileFormat::Markdown);
        assert!(doc.set_genre(Some(Genre::Letter)));
        assert!(doc.save().contains("genre: letter"));
        assert!(doc.set_genre(None));
        assert!(doc.save().contains("document: {}"), "{}", doc.save());
    }

    #[test]
    fn a_plain_document_gets_the_role_for_a_genre_only() {
        let text = "---\ntitle: Plain\n---\nBody\n";
        let mut doc = Document::load(text.into(), FileFormat::Markdown);
        assert!(doc.set_genre(None));
        assert!(!doc.is_dirty(), "no role for no genre");
        assert!(doc.set_genre(Some(Genre::Bill)));
        assert_eq!(doc.kind(), DocumentType::Document);
        assert!(doc.save().contains("genre: bill"), "{}", doc.save());
    }

    #[test]
    fn done_is_written_with_the_status_done_and_removed_with_another() {
        let mut doc = Document::load(
            "---\ntask:\n  status: todo\n---\n".into(),
            FileFormat::Markdown,
        );
        let now = DateTime::from_iso8601("2026-10-12T08:30:00Z").unwrap();
        doc.set_task_status("done".into(), now);
        let task = doc.roles().task.clone().unwrap();
        assert_eq!(
            (task.status.as_deref(), task.done),
            (Some("done"), Some(now))
        );

        let later = DateTime::from_iso8601("2026-10-13T09:00:00Z").unwrap();
        doc.set_task_status("done".into(), later);
        assert_eq!(doc.roles().task.as_ref().unwrap().done, Some(now), "kept");

        doc.set_task_status("waiting".into(), later);
        let task = doc.roles().task.clone().unwrap();
        assert_eq!((task.status.as_deref(), task.done), (Some("waiting"), None));
        assert!(!doc.save().contains("done"), "{}", doc.save());
    }

    #[test]
    fn task_fields_change_only_on_a_task() {
        let mut doc = Document::load("---\ndocument: {}\n---\n".into(), FileFormat::Markdown);
        let now = DateTime::from_iso8601("2026-10-12").unwrap();
        doc.set_task_status("done".into(), now);
        doc.set_task_due(Some(now));
        doc.set_task_assignee(Some(ContactRef::Name("Jane".into())));
        assert!(!doc.is_dirty());
    }

    #[test]
    fn joining_and_leaving_threads_manages_message_role() {
        let t1 = Id::generate();
        let t2 = Id::generate();
        let mut doc = Document::load("---\ndocument: {}\n---\n".into(), FileFormat::Markdown);

        doc.join_thread(t1);
        doc.join_thread(t2);
        doc.join_thread(t1);
        assert_eq!(doc.roles().message.as_ref().unwrap().threads, [t1, t2]);
        assert_eq!(doc.kind(), DocumentType::Document);

        doc.leave_thread(&t1);
        assert_eq!(doc.roles().message.as_ref().unwrap().threads, [t2]);
        doc.leave_thread(&t2);
        assert!(
            doc.roles().message.is_none(),
            "empty message role is dropped"
        );
        assert!(!doc.save().contains("message"));
    }

    #[test]
    fn leaving_last_thread_keeps_message_with_sender() {
        let t = Id::generate();
        let text = format!("---\nmessage:\n  sender: Jane\n  threads: [{t}]\n---\n");
        let mut doc = Document::load(text, FileFormat::Markdown);
        doc.leave_thread(&t);
        let message = doc.roles().message.as_ref().unwrap();
        assert!(message.threads.is_empty());
        assert!(message.sender.is_some());
    }

    #[test]
    fn leaving_a_project_updates_the_envelope() {
        let (p1, p2) = (Id::generate(), Id::generate());
        let text = format!("---\ndocument: {{}}\nprojects: [{p1}, {p2}]\n---\n");
        let mut doc = Document::load(text, FileFormat::Markdown);
        doc.leave_project(&p1);
        assert_eq!(doc.projects(), [p2]);
        assert!(doc.is_dirty());
        let saved = doc.save();
        assert!(saved.contains(&p2.to_string()) && !saved.contains(&p1.to_string()));
        doc.leave_project(&p2);
        assert!(!doc.save().contains("projects"), "empty list is omitted");
    }

    #[test]
    fn starring_round_trips_and_unstarring_drops_the_key() {
        let mut doc = Document::load("---\nproject: {}\n---\n".into(), FileFormat::Markdown);
        assert!(!doc.starred());

        doc.set_starred(true);
        let saved = doc.save();
        assert!(saved.contains("starred: true"));
        assert!(Document::load(saved, FileFormat::Markdown).starred());

        doc.set_starred(false);
        assert!(!doc.starred());
        assert!(
            !doc.save().contains("starred"),
            "unstarring removes the key"
        );
    }

    #[test]
    fn archiving_round_trips_and_unarchiving_drops_the_key() {
        let mut doc = Document::load("---\nproject: {}\n---\n".into(), FileFormat::Markdown);
        assert!(!doc.archived());

        doc.set_archived(true);
        let saved = doc.save();
        assert!(saved.contains("archived: true"));
        assert!(Document::load(saved, FileFormat::Markdown).archived());

        doc.set_archived(false);
        assert!(!doc.archived());
        assert!(
            !doc.save().contains("archived"),
            "unarchiving removes the key"
        );
    }

    #[test]
    fn a_link_is_replaced_and_unlinking_drops_the_key() {
        let mut doc = Document::load("---\ntask: {}\n---\n".into(), FileFormat::Markdown);
        assert_eq!(doc.linked_to(), None);

        let (a, b) = (Id::generate(), Id::generate());
        doc.set_link(Some(a));
        doc.set_link(Some(b));
        let saved = doc.save();
        assert!(saved.contains(&format!("linked-to: {b}")), "{saved}");
        assert!(!saved.contains(&a.to_string()), "one link: replaced");
        assert_eq!(
            Document::load(saved, FileFormat::Markdown).linked_to(),
            Some(b)
        );

        doc.set_link(None);
        assert_eq!(doc.linked_to(), None);
        assert!(
            !doc.save().contains("linked-to"),
            "unlinking removes the key"
        );

        let mut plain = Document::load("Body\n".into(), FileFormat::Markdown);
        plain.set_link(None);
        assert!(
            !plain.is_dirty(),
            "no frontmatter is made to remove nothing"
        );
    }

    #[test]
    fn invalid_yaml_degrades_to_plain_document() {
        let text = ": [ broken\n";
        let (envelope, body, err) = parse_document(FileFormat::YAML, text);
        assert!(envelope.is_none());
        assert_eq!(body, text);
        assert!(err.is_err());
    }
}
