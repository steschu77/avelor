//! YAML envelope: splitting, parsing, and composing.
//!
//! The envelope models the common keys from
//! vault.md#metadata. Unknown
//! keys are preserved verbatim in `extra` so round-tripping a document
//! through the app never loses user data.

use core::fmt;
use std::collections::BTreeMap;

use crate::shared::{ColorRef, ContactRef, Genre, OneOrMany};
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use serde::{Deserialize, Serialize};

// ----------------------------------------------------------------------------
type Extra = BTreeMap<String, serde_yaml_ng::Value>;

// ----------------------------------------------------------------------------
/// The derived type of an entity (vault.md#type-derivation). Never
/// stored: see [`Roles::derived_type`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DocumentType {
    Project,
    Thread,
    Calendar,
    BankAccount,
    Document,
    Message,
    Transactions,
    Event,
    Task,
    Contact,
    Record,
    List,
    Email,
    Conversation,
}

// ----------------------------------------------------------------------------
impl DocumentType {
    /// The three-letter code used in filenames (vault.md#filenames).
    pub fn type_id(&self) -> &'static str {
        match self {
            DocumentType::Project => "prj",
            DocumentType::Thread => "thd",
            DocumentType::Calendar => "cal",
            DocumentType::BankAccount => "bnk",
            DocumentType::Document => "doc",
            DocumentType::Message => "msg",
            DocumentType::Transactions => "txs",
            DocumentType::Event => "evt",
            DocumentType::Task => "tsk",
            DocumentType::Contact => "cnt",
            DocumentType::Record => "rec",
            DocumentType::List => "lst",
            DocumentType::Email => "eml",
            DocumentType::Conversation => "cvn",
        }
    }
}

// ----------------------------------------------------------------------------
impl fmt::Display for DocumentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DocumentType::Project => write!(f, "project"),
            DocumentType::Thread => write!(f, "thread"),
            DocumentType::Calendar => write!(f, "calendar"),
            DocumentType::BankAccount => write!(f, "bank-account"),
            DocumentType::Document => write!(f, "document"),
            DocumentType::Message => write!(f, "message"),
            DocumentType::Transactions => write!(f, "transactions"),
            DocumentType::Event => write!(f, "event"),
            DocumentType::Task => write!(f, "task"),
            DocumentType::Contact => write!(f, "contact"),
            DocumentType::Record => write!(f, "record"),
            DocumentType::List => write!(f, "list"),
            DocumentType::Email => write!(f, "email"),
            DocumentType::Conversation => write!(f, "conversation"),
        }
    }
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {}

// ----------------------------------------------------------------------------
/// A thread has no fields of its own: its members are the entities whose
/// `message.threads` lists the thread's id.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Thread {}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Calendar {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorRef>,
    #[serde(rename = "timeZone", default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BankAccount {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub account_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<ContactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub institution: Option<String>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<Genre>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<ContactRef>,
}

// ----------------------------------------------------------------------------
/// Thread membership: the entity is a member of every thread in `threads`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Message {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<ContactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient: Option<ContactRef>,
}

// ----------------------------------------------------------------------------
impl Message {
    /// Whether the block carries nothing and can be dropped.
    pub fn is_empty(&self) -> bool {
        self.threads.is_empty() && self.sender.is_none() && self.recipient.is_none()
    }
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Transaction {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<ContactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver: Option<ContactRef>,
    #[serde(rename = "linked-to", default, skip_serializing_if = "Option::is_none")]
    pub linked_to: Option<Id>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Transactions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transactions: Vec<Transaction>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Event {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<DateTime>,
    /// For an all-day event (dates without a time) the day after the last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<DateTime>,
    /// The IANA time zone the event's times are meant in (e.g.
    /// `Europe/Berlin`); a series repeats in it.
    #[serde(rename = "timeZone", default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    /// A series' RFC 5545 lines (`RRULE:…`, `EXDATE:…`); empty for a single
    /// event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recurrence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<ContactRef>,
    /// A series' occurrences that differ from it (moved, retitled or
    /// cancelled in Google), by their original start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exceptions: Vec<Exception>,
}

// ----------------------------------------------------------------------------
/// An occurrence of a series that differs from it: only what differs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exception {
    /// The start the series gives the occurrence.
    pub original: DateTime,
    /// `cancelled` for an occurrence deleted from the series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Set if the occurrence was moved, or its length changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Task {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<ContactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<DateTime>,
    /// When the task was marked done, written with the status `done`
    /// ([`Document::set_task_status`](crate::document::Document::set_task_status)).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<DateTime>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContactType {
    Personal,
    Business,
}

// ----------------------------------------------------------------------------
impl fmt::Display for ContactType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContactType::Personal => write!(f, "personal"),
            ContactType::Business => write!(f, "business"),
        }
    }
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Contact {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ContactType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub email: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phone: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub address: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birthday: Option<DateTime>,
    #[serde(rename = "IBAN", default, skip_serializing_if = "Option::is_none")]
    pub iban: Option<String>,
}

// ----------------------------------------------------------------------------
/// A record's fields are entirely custom per-document; nothing is typed
/// here, they stay in `Envelope::extra`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct List {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Email {
    /// The subject as the mail came, kept once the title differs from it
    /// (the mail analysis, mail.md#analysis); absent: the title is the
    /// subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// What kind of mail it is, as the mail analysis found (mail.md#analysis):
    /// `request`, `reply`, `notification`, `newsletter` or `other`. A string,
    /// so a value written by hand never keeps the file from being read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "OneOrMany::is_none")]
    pub to: OneOrMany<String>,
    #[serde(default, skip_serializing_if = "OneOrMany::is_none")]
    pub cc: OneOrMany<String>,
    #[serde(default, skip_serializing_if = "OneOrMany::is_none")]
    pub bcc: OneOrMany<String>,
}

// ----------------------------------------------------------------------------
/// One message inside a conversation. Messages form a tree via `parent`;
/// the first message has none. A message with a `model` is an AI reply.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConversationMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<ContactRef>,
    /// The AI model that wrote the message; absent for a person's message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The AI account the model was called through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Questions offered to ask next, while the message ends the path shown
    /// (the mail analysis's chat, mail.md#analysis).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
}

// ----------------------------------------------------------------------------
/// A whole message tree held inline in one file; `active` is the leaf of
/// the path currently shown.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<ConversationMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<Id>,
}

// ----------------------------------------------------------------------------
/// The roles an entity carries (vault.md#roles). Each role is a
/// top-level, role-named block in the frontmatter; any number of them may
/// be present at once. An adopted role is always written, even when empty
/// (`document: {}`), so the block itself is the role marker.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Roles {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Project>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<Thread>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<Calendar>,
    #[serde(
        rename = "bank-account",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub bank_account: Option<BankAccount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<Document>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transactions: Option<Transactions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<Event>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<Task>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contact: Option<Contact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<Record>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<List>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<Email>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<Conversation>,
}

// ----------------------------------------------------------------------------
impl Roles {
    /// Whether no role block is present.
    pub fn is_empty(&self) -> bool {
        *self == Roles::default()
    }

    /// The type derived from the roles, applying the ordered rules of
    /// vault.md#type-derivation: specific roles win over the
    /// generic `record`, `document`, and `message`. `None` if there are no
    /// roles; the caller then falls back on the file format.
    pub fn derived_type(&self) -> Option<DocumentType> {
        let rules = [
            (self.project.is_some(), DocumentType::Project),
            (self.calendar.is_some(), DocumentType::Calendar),
            (self.thread.is_some(), DocumentType::Thread),
            (self.bank_account.is_some(), DocumentType::BankAccount),
            (self.contact.is_some(), DocumentType::Contact),
            (self.event.is_some(), DocumentType::Event),
            (self.task.is_some(), DocumentType::Task),
            (self.list.is_some(), DocumentType::List),
            (self.email.is_some(), DocumentType::Email),
            (self.conversation.is_some(), DocumentType::Conversation),
            (self.transactions.is_some(), DocumentType::Transactions),
            (self.record.is_some(), DocumentType::Record),
            (self.document.is_some(), DocumentType::Document),
            (self.message.is_some(), DocumentType::Message),
        ];
        rules.into_iter().find(|(has, _)| *has).map(|(_, t)| t)
    }
}

// ----------------------------------------------------------------------------
/// Marker payload for sources without fields; written as `name: {}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Empty {}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Import {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Remote {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Id>,
    /// Properties identifying the entity inside the account (e.g. a mail's
    /// message-id); kept as raw YAML, their shape depends on the account.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remote: Vec<serde_yaml_ng::Value>,
}

impl Remote {
    // ------------------------------------------------------------------------
    /// The values of the property `key` (vault.md#remote: a list of
    /// `{key: value}` maps), in order.
    pub fn values<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.remote
            .iter()
            .filter_map(move |property| property.get(key)?.as_str())
    }

    // ------------------------------------------------------------------------
    /// The first value of the property `key`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.remote
            .iter()
            .find_map(|property| property.get(key)?.as_str())
    }

    // ------------------------------------------------------------------------
    /// Set the property `key` to `value`: the first `{key: …}` entry is
    /// replaced, or a new one appended.
    pub fn set(&mut self, key: &str, value: &str) {
        let mut map = serde_yaml_ng::Mapping::new();
        map.insert(key.into(), value.into());
        let property = serde_yaml_ng::Value::Mapping(map);
        match self.remote.iter_mut().find(|p| p.get(key).is_some()) {
            Some(existing) => *existing = property,
            None => self.remote.push(property),
        }
    }
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Scan {
    /// The folder watch the scan was imported from;
    /// absent for scans that came another way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<Id>,
    /// The scanned files, in page order: relative to the watch's folder
    /// when `watch` is given.
    #[serde(default, skip_serializing_if = "OneOrMany::is_none")]
    pub origin: OneOrMany<String>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Web {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

// ----------------------------------------------------------------------------
/// The entity's provenance (vault.md#source): exactly one
/// top-level, channel-named block. Every payload is lenient (all fields
/// optional), since a flattened `Option<Source>` quietly becomes `None`
/// if its block fails to parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Created(Empty),
    File(Empty),
    Remote(Remote),
    Scan(Scan),
    Web(Web),
    Ai(Empty),
    Import(Import),
}

// ----------------------------------------------------------------------------
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The one entity this one is linked to
    /// (vault.md#metadata).
    #[serde(rename = "linked-to", default, skip_serializing_if = "Option::is_none")]
    pub linked_to: Option<Id>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starred: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
    #[serde(flatten)]
    pub roles: Roles,
    #[serde(flatten)]
    pub extra: Extra,
}

impl Envelope {
    pub fn parse(yaml: &str) -> Result<Envelope> {
        serde_yaml_ng::from_str(yaml).map_err(Error::from)
    }

    pub fn compose(&self) -> Option<String> {
        serde_yaml_ng::to_string(self).ok()
    }

    /// Set a non-standard key (stored via the flattened `extra` map).
    pub fn set_extra(&mut self, key: &str, value: impl Into<serde_yaml_ng::Value>) {
        self.extra.insert(key.into(), value.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::OneOrMany::Many;

    const DOC: &str = "\nid: 01a06d07-2b70-708c-9c71-7ddfaa46c0f5\ndocument: {}\nprojects: [01a077f7-7e89-7263-b36d-ad63c20809e2]\ntitle: Test\ntags: [a, b]\ncustom_key: kept\nremote:\n  account: 01a077f7-7e89-7263-b36d-ad63c20809e2\nemail:\n  to: []";
    const ID: &str = "01a06d07-2b70-708c-9c71-7ddfaa46c0f5";
    const PROJ_ID: &str = "01a077f7-7e89-7263-b36d-ad63c20809e2";

    const MULTI: &str = "id: 01a0c33f-4a20-74d4-bcd3-7cf22e58f379
title: ID Card Renewal
date: 2026-09-21T09:15:00Z
remote:
  account: 01a0c342-0940-7d5a-b215-6123a44c9144
  remote: [{message-id: \"<abc@example.com>\"}]
document:
  genre: contract
email:
  kind: request
  from: \"Alexander Yahnek <alex@example.com>\"
  to: [\"Registration Office <registration@springfield.com>\"]
message:
  threads: [01a0c33f-4a20-7269-bc56-cf275a720bc1]
custom_key: kept
";

    fn roles(yaml: &str) -> Roles {
        Envelope::parse(yaml).unwrap().roles
    }

    #[test]
    fn parses_envelope_with_extras_preserved() {
        let env = Envelope::parse(DOC).unwrap();
        let id = Id::parse(ID).unwrap();
        assert_eq!(env.id, Some(id));
        assert_eq!(env.roles.document, Some(Document::default()));
        assert_eq!(
            env.roles.email,
            Some(Email {
                to: Many(vec![]),
                ..Default::default()
            })
        );
        assert_eq!(env.tags, ["a", "b"]);
        let proj_id = Id::parse(PROJ_ID).unwrap();
        assert_eq!(env.projects, [proj_id]);
        let remote = Source::Remote(Remote {
            account: Some(proj_id),
            ..Default::default()
        });
        assert_eq!(env.source.as_ref(), Some(&remote));
        assert_eq!(
            env.extra.get("custom_key").and_then(|v| v.as_str()),
            Some("kept")
        );
    }

    #[test]
    fn compose_round_trips() {
        let env1 = Envelope::parse(DOC).unwrap();
        let recomposed = env1.compose().unwrap();
        let env2 = Envelope::parse(&recomposed).unwrap();
        assert_eq!(env1, env2);
    }

    #[test]
    fn parses_multiple_roles_at_once() {
        let env = Envelope::parse(MULTI).unwrap();
        let r = &env.roles;
        assert_eq!(r.document.as_ref().unwrap().genre, Some(Genre::Contract));
        assert_eq!(
            r.email.as_ref().unwrap().from.as_deref(),
            Some("Alexander Yahnek <alex@example.com>")
        );
        assert_eq!(r.email.as_ref().unwrap().kind.as_deref(), Some("request"));
        assert_eq!(r.message.as_ref().unwrap().threads.len(), 1);
        assert!(matches!(env.source, Some(Source::Remote(ref rm)) if rm.remote.len() == 1));

        let keys: Vec<_> = env.extra.keys().collect();
        assert_eq!(
            keys,
            ["custom_key"],
            "known blocks must not remain in extra"
        );

        let env2 = Envelope::parse(&env.compose().unwrap()).unwrap();
        assert_eq!(env, env2);
    }

    #[test]
    fn derives_type_by_precedence() {
        assert_eq!(roles(MULTI).derived_type(), Some(DocumentType::Email));
        assert_eq!(
            roles("document: {}\nmessage: {}\n").derived_type(),
            Some(DocumentType::Document)
        );
        assert_eq!(
            roles("record: {}\nlist: {}\n").derived_type(),
            Some(DocumentType::List)
        );
        assert_eq!(
            roles("message: {}\n").derived_type(),
            Some(DocumentType::Message)
        );
        assert_eq!(
            roles("bank-account: {}\n").derived_type(),
            Some(DocumentType::BankAccount)
        );
        assert_eq!(roles("title: plain\n").derived_type(), None);
        assert!(roles("title: plain\n").is_empty());
    }

    #[test]
    fn type_ids_are_three_letters() {
        assert_eq!(DocumentType::Thread.type_id(), "thd");
        assert_eq!(DocumentType::Conversation.type_id(), "cvn");
        assert_eq!(DocumentType::BankAccount.to_string(), "bank-account");
    }

    #[test]
    fn sources_round_trip() {
        let cases = [
            "created: {}\n",
            "file: {}\n",
            "ai: {}\n",
            "scan:\n  origin: scan.jpg\n",
            "web:\n  url: https://example.com\n",
            "import:\n  origin: export.csv\n",
            "remote:\n  account: 01a077f7-7e89-7263-b36d-ad63c20809e2\n",
        ];
        for yaml in cases {
            let env = Envelope::parse(yaml).unwrap();
            assert!(env.source.is_some(), "source not parsed: {yaml}");
            assert!(env.extra.is_empty(), "source left in extra: {yaml}");
            assert_eq!(env.compose().unwrap(), yaml);
        }
        assert_eq!(Envelope::parse("title: x\n").unwrap().source, None);
    }

    #[test]
    fn a_messages_suggestions_round_trip() {
        let yaml = format!(
            "conversation:\n  messages:\n  - id: {ID}\n    model: m\n    content: Pay by Friday.\n    suggestions: [Is the amount right?, When is it due?]\n  active: {ID}\n"
        );
        let env = Envelope::parse(&yaml).unwrap();
        let message = &env.roles.conversation.as_ref().unwrap().messages[0];
        assert_eq!(
            message.suggestions,
            ["Is the amount right?", "When is it due?"]
        );
        assert_eq!(Envelope::parse(&env.compose().unwrap()).unwrap(), env);

        let plain = "conversation:\n  messages:\n  - content: Hi\n";
        let composed = Envelope::parse(plain).unwrap().compose().unwrap();
        assert!(!composed.contains("suggestions"), "{composed}");
    }

    #[test]
    fn remote_properties_are_read_and_set() {
        let yaml = "remote:\n  account: 01a077f7-7e89-7263-b36d-ad63c20809e2\n  remote: [{event-id: abc}, {etag: '\"1\"'}, {other: [1, 2]}]\n";
        let Some(Source::Remote(mut remote)) = Envelope::parse(yaml).unwrap().source else {
            panic!("no remote source");
        };
        assert_eq!(remote.get("event-id"), Some("abc"));
        assert_eq!(remote.get("missing"), None);

        remote.set("etag", "\"2\"");
        remote.set("snapshot", "00ff");
        assert_eq!(remote.get("etag"), Some("\"2\""));
        assert_eq!(remote.values("etag").count(), 1, "replaced, not appended");
        assert_eq!(remote.get("snapshot"), Some("00ff"));
        assert_eq!(remote.remote.len(), 4, "unknown properties are kept");
    }

    #[test]
    fn the_link_is_a_top_level_field() {
        let yaml = format!("tags:\n- a\nlinked-to: {ID}\ntask:\n  status: todo\n");
        let env = Envelope::parse(&yaml).unwrap();
        assert_eq!(env.linked_to, Some(Id::parse(ID).unwrap()));
        assert!(env.extra.is_empty());
        assert_eq!(env.compose().unwrap(), yaml);

        // Only the top level is read; the key in a role
        // block is dropped on the next write.
        let old = format!("task:\n  status: todo\n  linked-to: {ID}\n");
        let env = Envelope::parse(&old).unwrap();
        assert_eq!(env.linked_to, None);
        assert_eq!(env.compose().unwrap(), "task:\n  status: todo\n");
    }

    #[test]
    fn event_series_round_trip() {
        let yaml = "event:
  start: 2026-10-05T08:00:00Z
  end: 2026-10-05T09:00:00Z
  timeZone: Europe/Berlin
  recurrence: ['RRULE:FREQ=WEEKLY;BYDAY=MO']
";
        let env = Envelope::parse(yaml).unwrap();
        let event = env.roles.event.as_ref().unwrap();
        assert_eq!(event.time_zone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(event.recurrence, ["RRULE:FREQ=WEEKLY;BYDAY=MO"]);
        assert!(env.extra.is_empty());
        assert_eq!(Envelope::parse(&env.compose().unwrap()).unwrap(), env);
    }

    #[test]
    fn event_exceptions_round_trip() {
        let yaml = "event:
  start: 2026-10-05T08:00:00Z
  recurrence:
  - RRULE:FREQ=WEEKLY;BYDAY=MO
  exceptions:
  - original: 2026-10-12T08:00:00Z
    start: 2026-10-13T09:00:00Z
    end: 2026-10-13T10:00:00Z
  - original: 2026-10-19T08:00:00Z
    status: cancelled
  - original: 2026-10-26T08:00:00Z
    title: Retro
    location: Room 2
";
        let env = Envelope::parse(yaml).unwrap();
        let exceptions = &env.roles.event.as_ref().unwrap().exceptions;
        assert_eq!(exceptions.len(), 3);
        assert_eq!(exceptions[1].status.as_deref(), Some("cancelled"));
        assert_eq!(exceptions[2].start, None);
        assert_eq!(exceptions[2].title.as_deref(), Some("Retro"));
        assert!(env.extra.is_empty());
        assert_eq!(env.compose().unwrap(), yaml);
    }

    #[test]
    fn parses_conversation_tree() {
        let yaml = "conversation:
  active: 01a0c36d-5730-70b5-8f32-ec65cc4c3bb0
  messages:
    - id: 01a0c36d-37f0-7254-8ff5-db84b32e57d8
      sender: Alex
      date: 2026-09-21T10:05:10Z
      content: Hi
    - id: 01a0c36d-5730-70b5-8f32-ec65cc4c3bb0
      parent: 01a0c36d-37f0-7254-8ff5-db84b32e57d8
      sender: AI Assistant
      content: |-
        Hello

        there
";
        let conv = roles(yaml).conversation.unwrap();
        assert_eq!(conv.messages.len(), 2);
        assert_eq!(conv.messages[1].parent, conv.messages[0].id);
        assert_eq!(conv.active, conv.messages[1].id);
        assert_eq!(conv.messages[1].content.as_deref(), Some("Hello\n\nthere"));
        assert_eq!(conv.messages[1].model, None);
    }

    #[test]
    fn ai_replies_name_their_model_and_account() {
        let yaml = "conversation:
  active: 01a0c36d-5730-70b5-8f32-ec65cc4c3bb0
  messages:
  - id: 01a0c36d-37f0-7254-8ff5-db84b32e57d8
    date: 2026-09-21T10:05:10Z
    content: Hi
  - id: 01a0c36d-5730-70b5-8f32-ec65cc4c3bb0
    parent: 01a0c36d-37f0-7254-8ff5-db84b32e57d8
    model: gpt-5-mini
    account: 01a0c342-0940-7d5a-b215-6123a44c9144
    date: 2026-09-21T10:05:12Z
    content: Hello
";
        let env = Envelope::parse(yaml).unwrap();
        let conv = env.roles.conversation.as_ref().unwrap();
        assert_eq!(conv.messages[0].model, None);
        assert_eq!(conv.messages[0].sender, None);
        assert_eq!(conv.messages[1].model.as_deref(), Some("gpt-5-mini"));
        assert_eq!(
            conv.messages[1].account,
            Some(Id::parse("01a0c342-0940-7d5a-b215-6123a44c9144").unwrap())
        );
        let composed = env.compose().unwrap();
        assert!(composed.contains("model: gpt-5-mini"), "{composed}");
        assert_eq!(Envelope::parse(&composed).unwrap(), env);
    }
}
