//! Markdown + YAML frontmatter: splitting, parsing, and composing.
//!
//! The envelope models the common keys from
//! vault.md#metadata. Unknown
//! keys are preserved verbatim in `extra` so round-tripping a document
//! through the app never loses user data.

use crate::envelope::{DocumentType, Envelope, Roles, Source};
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
    /// What the file held when the app last read or wrote it.
    disk: OnDisk,
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
            disk,
            err,
        }
    }

    // ------------------------------------------------------------------------
    /// The document as read from a file stamped `stamp`.
    pub(crate) fn stamped(mut self, stamp: Option<FileStamp>) -> Self {
        self.disk.stamp = stamp;
        self
    }

    // ------------------------------------------------------------------------
    /// The document was written to its file, now stamped `stamp`: what it
    /// holds is what is on disk.
    pub(crate) fn written(&mut self, stamp: Option<FileStamp>) {
        self.disk = OnDisk {
            envelope: self.envelope.clone(),
            content: self.content.clone(),
            stamp,
        };
    }

    // ------------------------------------------------------------------------
    /// The file's stamp at the last read or write.
    pub(crate) fn stamp(&self) -> Option<FileStamp> {
        self.disk.stamp
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
    /// read or write.
    pub fn is_dirty(&self) -> bool {
        self.disk.envelope != self.envelope || self.disk.content != self.content
    }

    // ------------------------------------------------------------------------
    pub fn is_broken(&self) -> bool {
        self.err.is_err()
    }

    // ------------------------------------------------------------------------
    /// Set the entity's title. An empty (or all-whitespace) title removes the
    /// `title` key altogether; a missing title loads as `""` again.
    pub fn set_title(&mut self, title: String) {
        let title = title.trim().to_string();
        self.envelope_mut().title = (!title.is_empty()).then_some(title);
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
    fn kind_falls_back_on_file_format() {
        let md = Document::load("---\ntitle: x\n---\n".into(), FileFormat::Markdown);
        assert_eq!(md.kind(), DocumentType::Document);
        let yaml = Document::load("title: x\n".into(), FileFormat::YAML);
        assert_eq!(yaml.kind(), DocumentType::Record);
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
