//! YAML envelope: splitting, parsing, and composing.
//!
//! The envelope models the common keys from
//! vault.md#metadata. Unknown
//! keys are preserved verbatim in `extra` so round-tripping a document
//! through the app never loses user data.

use core::fmt;
use std::collections::BTreeMap;

use crate::shared::{ColorRef, ContactRef, Genre, OneOrMany};
use av_utils::datetime::{Date, DateTime};
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use av_utils::is_false;
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
    Statement,
    Event,
    Task,
    Contact,
    Record,
    List,
    Email,
    Conversation,
    RecurringPayment,
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
            DocumentType::Statement => "txs",
            DocumentType::Event => "evt",
            DocumentType::Task => "tsk",
            DocumentType::Contact => "cnt",
            DocumentType::Record => "rec",
            DocumentType::List => "lst",
            DocumentType::Email => "eml",
            DocumentType::Conversation => "cvn",
            DocumentType::RecurringPayment => "rcp",
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
            DocumentType::Statement => write!(f, "statement"),
            DocumentType::Event => write!(f, "event"),
            DocumentType::Task => write!(f, "task"),
            DocumentType::Contact => write!(f, "contact"),
            DocumentType::Record => write!(f, "record"),
            DocumentType::List => write!(f, "list"),
            DocumentType::Email => write!(f, "email"),
            DocumentType::Conversation => write!(f, "conversation"),
            DocumentType::RecurringPayment => write!(f, "recurring-payment"),
        }
    }
}

// ----------------------------------------------------------------------------
/// A project's blueprint (vault.md#project): what it needs, as data in its
/// own file. Every part is optional; a project without one is `project: {}`.
/// Keys the app doesn't know stay, at every level (`extra`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// The shipped blueprint it came from (its file's name, `travel`);
    /// absent for one of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blueprint: Option<String>,
    /// Its own values by name; what each is, the blueprint says.
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub facts: Extra,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub documents: Vec<ExpectedDocument>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub payments: Vec<ExpectedPayment>,
    /// The gaps the header shows, by their kind (`documents`, `budget`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<String>,
    /// Tasks written when the project is archived.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub afterwards: Vec<Afterwards>,
    /// What the project is about, carried by its members and its tasks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// A trip's route while its days are open: where, how long, how.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sketch: Option<Sketch>,
    #[serde(flatten)]
    pub extra: Extra,
}

// ----------------------------------------------------------------------------
/// A trip sketched while its days are open (vault.md#project): when it
/// may be and how long, its stops in order, and the way back home. Fixing
/// its days turns it into held events.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    /// The days it falls within.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
    /// How long it takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nights: Option<Nights>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stops: Vec<Stop>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<Back>,
    #[serde(flatten)]
    pub extra: Extra,
}

// ----------------------------------------------------------------------------
/// The days a sketched trip falls within, the first and the last.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Window {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<DateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<DateTime>,
    #[serde(flatten)]
    pub extra: Extra,
}

// ----------------------------------------------------------------------------
/// A sketched stop, and the way that leads to it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stop {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nights: Option<Nights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The way there: one, or several for a via without a night.
    #[serde(default, skip_serializing_if = "OneOrMany::is_none")]
    pub by: OneOrMany<String>,
    /// The way there takes a night.
    #[serde(default, skip_serializing_if = "is_false")]
    pub overnight: bool,
    #[serde(flatten)]
    pub extra: Extra,
}

// ----------------------------------------------------------------------------
/// A sketched trip's way back home.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Back {
    #[serde(default, skip_serializing_if = "OneOrMany::is_none")]
    pub by: OneOrMany<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

// ----------------------------------------------------------------------------
/// How many nights a sketched trip or stop takes: a count, or as text
/// about one (`~6`) or a range (`2-3`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Nights {
    Count(u32),
    Text(String),
}

// ----------------------------------------------------------------------------
/// A document a project expects: how to know one, and the entities that
/// meet it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExpectedDocument {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<Genre>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<ContactRef>,
    /// How recent it must be, a span (`90d`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
    /// The fact it comes after (`handover`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub several: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by: Vec<Id>,
    #[serde(flatten)]
    pub extra: Extra,
}

// ----------------------------------------------------------------------------
/// A payment a project waits for, or makes: met by a booking linked to
/// what it is `for`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExpectedPayment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<ContactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<ContactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<DateTime>,
    #[serde(rename = "for", default, skip_serializing_if = "Option::is_none")]
    pub for_entity: Option<Id>,
    #[serde(flatten)]
    pub extra: Extra,
}

// ----------------------------------------------------------------------------
/// A task written when the project is archived, due from one of its facts
/// plus a span (`handover + 5y`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Afterwards {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(flatten)]
    pub extra: Extra,
}

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
    /// Whose work calendar it is: its events' `work` say when they are off
    /// or work remotely.
    #[serde(rename = "work-of", default, skip_serializing_if = "Option::is_none")]
    pub work_of: Option<ContactRef>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<Original>,
}

// ----------------------------------------------------------------------------
/// The file a document stands for, such as an email's attachment
/// (vault.md#document): next to the document's own file, and moved with it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Original {
    /// Its name in the document's folder; a plain name, never a path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The name it came with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its size in bytes when it was saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// An encrypted PDF, which no model is sent (import.md#reading-pdfs).
    #[serde(default, skip_serializing_if = "is_false")]
    pub encrypted: bool,
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
pub struct Booking {
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
    /// What the booking is for, in the bank's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// `card`, `direct-debit`, `transfer`, `cash` or `fee`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The bank's reference, which tells a booking imported twice.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// The other side's account number or IBAN, and its BIC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iban: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bic: Option<String>,
    #[serde(rename = "linked-to", default, skip_serializing_if = "Option::is_none")]
    pub linked_to: Option<Id>,
    /// The other half of a transfer between two of the vault's accounts:
    /// its booking's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer: Option<Id>,
    /// The recurring payment the booking belongs to: its entity's id.
    #[serde(
        rename = "recurring-payment",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub recurring_payment: Option<Id>,
}

impl Booking {
    pub fn is_pending(&self) -> bool {
        self.status.as_deref() == Some("pending")
    }

    pub fn day(&self) -> Option<Date> {
        self.date.map(|date| date.date)
    }

    pub fn cents(&self) -> Option<i64> {
        self.amount.map(to_cents)
    }

    /// The other side: who was paid for money out, who paid for money in.
    pub fn party(&self) -> Option<&ContactRef> {
        if self.amount.is_some_and(|amount| amount < 0.0) {
            self.receiver.as_ref()
        } else {
            self.sender.as_ref()
        }
    }
}

// ----------------------------------------------------------------------------
/// An amount of money in cents.
pub fn to_cents(amount: f64) -> i64 {
    (amount * 100.0).round() as i64
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Statement {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    /// The account's balance at the statement's end, its booked bookings
    /// counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bookings: Vec<Booking>,
}

// ----------------------------------------------------------------------------
/// A payment the vault expects on a schedule (vault.md#recurring).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecurringPayment {
    /// The bank account it is paid from or into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Id>,
    /// The other side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub party: Option<ContactRef>,
    /// RFC 5545 lines (`RRULE:…`), as an event's `schedule`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedule: Vec<String>,
    /// The first occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<DateTime>,
    /// The last day an occurrence may fall on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<DateTime>,
    /// From each date on, the amount; the last one current.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub amounts: Vec<Amount>,
    /// How far a booking may differ from the amount and still pay it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vary: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

// ----------------------------------------------------------------------------
/// A recurring payment's amount from a date on; negative for money out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Amount {
    pub from: DateTime,
    pub amount: f64,
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
    /// `Europe/Berlin`), the end's too unless it has its own; a recurring event
    /// repeats in it.
    #[serde(rename = "timeZone", default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    /// The end's own time zone, if it isn't the start's (a flight's
    /// arrival).
    #[serde(
        rename = "end-time-zone",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub end_time_zone: Option<String>,
    /// A recurring event's RFC 5545 lines (`RRULE:…`, `EXDATE:…`); empty for a single
    /// event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedule: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<ContactRef>,
    /// A recurring event's occurrences that differ from it (moved, retitled or
    /// cancelled in Google), by their original start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exceptions: Vec<Exception>,
    /// What part of a trip it is: `transport`, `stay` or `activity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// A transport's way: `flight`, `train`, `bus`, `ferry` or `car`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub way: Option<String>,
    /// Where a transport leaves from and arrives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Where a stay or an activity is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// A transport's booking reference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub booking: Option<String>,
    /// In a work calendar: `off` or `remote`; other text is kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<String>,
}

// ----------------------------------------------------------------------------
/// An occurrence of a recurring event that differs from it: only what differs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exception {
    /// The start the recurring event gives the occurrence.
    pub original: DateTime,
    /// `cancelled` for an occurrence deleted from the recurring event.
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
    /// The subject as the email came, kept once the title differs from it
    /// (the email analysis, email.md#analysis); absent: the title is the
    /// subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// What kind of email it is, as the email analysis found (email.md#analysis):
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
    /// (the email analysis's chat, email.md#analysis).
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
    pub statement: Option<Statement>,
    #[serde(
        rename = "recurring-payment",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub recurring_payment: Option<RecurringPayment>,
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
            (self.statement.is_some(), DocumentType::Statement),
            (self.record.is_some(), DocumentType::Record),
            (self.document.is_some(), DocumentType::Document),
            (
                self.recurring_payment.is_some(),
                DocumentType::RecurringPayment,
            ),
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
    /// The folder watch the file was imported from (`import.md#folder-watches`);
    /// absent for a file that came another way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<Id>,
    /// The file's name: relative to the watch's folder when `watch` is
    /// given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Remote {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Id>,
    /// Properties identifying the entity inside the account (e.g. an email's
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
    /// The folder watch the scan was imported from (`import.md#folder-watches`);
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
    #[serde(rename = "tags", default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
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

    /// The texts a search reads besides the title, the labels and the body
    /// (`vault.md#search`): the roles' names, places, items and messages,
    /// then every string a record keeps in `extra`. Typed fields that are
    /// ids, dates, states or codes are left out by naming the others.
    pub fn text(&self) -> Vec<&str> {
        let mut out = Vec::new();
        let roles = &self.roles;
        if let Some(document) = &roles.document {
            names(&mut out, &document.author);
        }
        if let Some(message) = &roles.message {
            names(&mut out, message.sender.iter().chain(&message.recipient));
        }
        if let Some(statement) = &roles.statement {
            for booking in &statement.bookings {
                out.extend(booking.title.as_deref());
                out.extend(booking.purpose.as_deref());
                names(&mut out, booking.sender.iter().chain(&booking.receiver));
            }
        }
        if let Some(recurring) = &roles.recurring_payment {
            names(&mut out, &recurring.party);
            out.extend(recurring.category.as_deref());
        }
        if let Some(event) = &roles.event {
            out.extend(event.location.as_deref());
            names(&mut out, &event.attendees);
            for exception in &event.exceptions {
                out.extend(exception.title.as_deref());
                out.extend(exception.location.as_deref());
            }
        }
        if let Some(task) = &roles.task {
            names(&mut out, &task.assignee);
        }
        if let Some(contact) = &roles.contact {
            let lists = [&contact.email, &contact.phone, &contact.address];
            out.extend(lists.into_iter().flatten().map(String::as_str));
            out.extend(contact.iban.as_deref());
        }
        if let Some(list) = &roles.list {
            out.extend(list.items.iter().map(String::as_str));
        }
        if let Some(email) = &roles.email {
            out.extend(email.subject.as_deref());
            out.extend(email.from.as_deref());
            let to = [&email.to, &email.cc, &email.bcc];
            out.extend(
                to.into_iter()
                    .flat_map(OneOrMany::as_slice)
                    .map(String::as_str),
            );
        }
        if let Some(conversation) = &roles.conversation {
            let contents = conversation
                .messages
                .iter()
                .filter_map(|m| m.content.as_deref());
            out.extend(contents);
        }
        if let Some(account) = &roles.bank_account {
            out.extend(account.number.as_deref());
            out.extend(account.institution.as_deref());
            names(&mut out, &account.owner);
        }
        for value in self.extra.values() {
            strings(&mut out, value);
        }
        out.retain(|text| !text.trim().is_empty());
        out
    }
}

// ----------------------------------------------------------------------------
/// The names among `contacts`; a link to a contact is its id.
fn names<'a>(out: &mut Vec<&'a str>, contacts: impl IntoIterator<Item = &'a ContactRef>) {
    out.extend(contacts.into_iter().filter_map(|contact| match contact {
        ContactRef::Name(name) => Some(name.as_str()),
        ContactRef::Id(_) => None,
    }));
}

// ----------------------------------------------------------------------------
/// The strings in `value`, at any depth.
fn strings<'a>(out: &mut Vec<&'a str>, value: &'a serde_yaml_ng::Value) {
    use serde_yaml_ng::Value;
    match value {
        Value::String(text) => out.push(text),
        Value::Sequence(values) => values.iter().for_each(|v| strings(out, v)),
        Value::Mapping(map) => map.values().for_each(|v| strings(out, v)),
        Value::Tagged(tagged) => strings(out, &tagged.value),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
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
        assert_eq!(env.labels, ["a", "b"]);
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
    fn the_text_is_the_roles_words_and_the_records_strings() {
        let yaml = "id: 01a06d07-2b70-708c-9c71-7ddfaa46c0f5
title: Title
date: 2026-09-21T09:15:00Z
tags: [label]
remote:
  account: 01a077f7-7e89-7263-b36d-ad63c20809e2
  remote: [{message-id: \"<abc@example.com>\"}]
email:
  kind: request
  from: \"Alex <alex@example.com>\"
  to: [\"Office <office@example.com>\"]
message:
  sender: '@01a077f7-7e89-7263-b36d-ad63c20809e2'
  recipient: Jane Doe
statement:
  bookings:
    - title: Tyres
      date: 2026-09-01T10:00:00Z
      status: booked
      amount: -89.5
      sender: Garage Miller
list:
  items: [Milk, Bread]
event:
  status: confirmed
  location: Town hall
  timeZone: Europe/Berlin
plate: AB-123
service:
  - {date: 2026-03-01, note: oil changed, km: 42000}
";
        let envelope = Envelope::parse(yaml).unwrap();
        assert_eq!(
            envelope.text(),
            [
                "Jane Doe",
                "Tyres",
                "Garage Miller",
                "Town hall",
                "Milk",
                "Bread",
                "Alex <alex@example.com>",
                "Office <office@example.com>",
                "AB-123",
                "2026-03-01",
                "oil changed",
            ]
        );
    }

    #[test]
    fn a_conversation_and_a_contact_give_their_texts() {
        let yaml = "conversation:
  messages:
    - {id: 01a06d07-2b70-708c-9c71-7ddfaa46c0f5, model: some-model, content: The answer}
contact:
  type: personal
  email: [jane@example.com]
  phone: ['+49 30 1234']
  IBAN: DE02 1203
";
        let envelope = Envelope::parse(yaml).unwrap();
        assert_eq!(
            envelope.text(),
            ["jane@example.com", "+49 30 1234", "DE02 1203", "The answer"]
        );
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
        assert_eq!(
            roles("recurring-payment: {}\n").derived_type(),
            Some(DocumentType::RecurringPayment)
        );
        assert_eq!(
            roles("document: {}\nrecurring-payment: {}\n").derived_type(),
            Some(DocumentType::Document)
        );
        assert_eq!(roles("title: plain\n").derived_type(), None);
        assert!(roles("title: plain\n").is_empty());
    }

    #[test]
    fn type_ids_are_three_letters() {
        assert_eq!(DocumentType::Thread.type_id(), "thd");
        assert_eq!(DocumentType::Conversation.type_id(), "cvn");
        assert_eq!(DocumentType::BankAccount.to_string(), "bank-account");
        assert_eq!(DocumentType::RecurringPayment.type_id(), "rcp");
        assert_eq!(
            DocumentType::RecurringPayment.to_string(),
            "recurring-payment"
        );
    }

    #[test]
    fn a_recurring_payment_and_its_booking_round_trip() {
        let yaml = format!(
            "title: StreamFlix\nrecurring-payment:\n  account: {ID}\n  party: StreamFlix Inc.\n  schedule:\n  - RRULE:FREQ=MONTHLY;BYMONTHDAY=28\n  from: 2026-08-28\n  amounts:\n  - from: 2026-08-28\n    amount: -15.99\n  vary: 0.5\n  category: subscriptions\n"
        );
        let env = Envelope::parse(&yaml).unwrap();
        let recurring = env.roles.recurring_payment.as_ref().unwrap();
        assert_eq!(recurring.schedule, ["RRULE:FREQ=MONTHLY;BYMONTHDAY=28"]);
        assert_eq!(recurring.amounts[0].amount, -15.99);
        assert!(env.extra.is_empty(), "the role left in extra");
        assert_eq!(env.text(), ["StreamFlix Inc.", "subscriptions"]);
        assert_eq!(env.compose().unwrap(), yaml);

        let booking =
            format!("statement:\n  bookings:\n  - amount: -15.99\n    recurring-payment: {ID}\n");
        let env = Envelope::parse(&booking).unwrap();
        let item = &env.roles.statement.as_ref().unwrap().bookings[0];
        assert_eq!(
            item.recurring_payment.map(|id| id.to_string()),
            Some(ID.to_string())
        );
        assert_eq!(env.compose().unwrap(), booking);
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
    fn a_projects_blueprint_round_trips_with_unknown_keys() {
        let yaml = format!(
            "title: Bad 2027
project:
  blueprint: renovation
  facts:
    budget: 15000
    days:
      from: 2027-07-08
      to: 2027-07-18
    where: Neue Str. 2, Hamburg
  documents:
  - name: quotes
    genre: quote
    several: true
    by:
    - {ID}
    hint: kept
  - name: handover report
    within: 90d
    after: handover
  payments:
  - name: payout
    amount: 1150.0
    from: '@{PROJ_ID}'
    due: 2027-06-30
    for: {ID}
    note: kept
  checks:
  - documents
  - budget
  afterwards:
  - title: The tiler's warranty ends
    due: handover + 5y
    labels: kept
  labels:
  - bathroom
  sketch:
    window:
      from: 2028-08-01
      to: 2028-08-31
    nights: 4-5
    stops:
    - place: Hamburg
      nights: ~6
      note: at Oma's
      by: flight
      overnight: true
      hint: kept
    - place: Isle Royale
      nights: 2-3
      by:
      - car
      - ferry
    - place: Duluth
      nights: 1
    back:
      by: train
  phase: kept
"
        );
        let env = Envelope::parse(&yaml).unwrap();
        let project = env.roles.project.as_ref().unwrap();
        assert_eq!(project.blueprint.as_deref(), Some("renovation"));
        assert_eq!(project.facts["budget"].as_u64(), Some(15000));
        assert_eq!(
            project.documents[0].genre,
            Some(Genre::Other("quote".into()))
        );
        assert!(project.documents[0].several);
        assert_eq!(project.documents[0].by, [Id::parse(ID).unwrap()]);
        assert_eq!(project.payments[0].for_entity, Some(Id::parse(ID).unwrap()));
        assert_eq!(project.checks, ["documents", "budget"]);
        assert_eq!(project.afterwards[0].due.as_deref(), Some("handover + 5y"));
        let sketch = project.sketch.as_ref().unwrap();
        let window = sketch.window.as_ref().unwrap();
        assert_eq!(window.to.unwrap().as_iso8601(), "2028-08-31");
        assert_eq!(sketch.nights, Some(Nights::Text("4-5".into())));
        let stops = &sketch.stops;
        assert_eq!(stops[0].nights, Some(Nights::Text("~6".into())));
        assert!(stops[0].overnight && stops[0].extra.contains_key("hint"));
        assert_eq!(stops[1].by.as_slice(), ["car", "ferry"]);
        assert_eq!(stops[2].nights, Some(Nights::Count(1)));
        assert_eq!(sketch.back.as_ref().unwrap().by.as_slice(), ["train"]);
        assert!(project.extra.contains_key("phase"));
        assert!(project.documents[0].extra.contains_key("hint"));
        assert!(project.payments[0].extra.contains_key("note"));
        assert!(project.afterwards[0].extra.contains_key("labels"));
        assert!(env.extra.is_empty(), "the block left in extra");
        assert_eq!(env.compose().unwrap(), yaml);
    }

    #[test]
    fn a_project_without_a_blueprint_stays_empty() {
        let env = Envelope::parse("title: P\nproject: {}\n").unwrap();
        assert_eq!(env.roles.project, Some(Project::default()));
        assert_eq!(env.compose().unwrap(), "title: P\nproject: {}\n");
    }

    #[test]
    fn a_trip_event_and_a_work_calendar_round_trip() {
        let yaml = format!(
            "event:
  calendar: {ID}
  status: tentative
  start: 2027-07-08T23:05:00Z
  end: 2027-07-09T12:20:00Z
  timeZone: America/Chicago
  end-time-zone: Europe/Berlin
  kind: transport
  way: flight
  from: ORD
  to: HAM
  booking: X7K2PL
"
        );
        let env = Envelope::parse(&yaml).unwrap();
        let event = env.roles.event.as_ref().unwrap();
        assert_eq!(event.end_time_zone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(
            (event.kind.as_deref(), event.way.as_deref()),
            (Some("transport"), Some("flight"))
        );
        assert_eq!(event.booking.as_deref(), Some("X7K2PL"));
        assert!(env.extra.is_empty());
        assert_eq!(env.compose().unwrap(), yaml);

        let yaml = format!(
            "calendar:
  work-of: '@{ID}'
"
        );
        let env = Envelope::parse(&yaml).unwrap();
        let calendar = env.roles.calendar.as_ref().unwrap();
        assert_eq!(
            calendar.work_of,
            Some(ContactRef::Id(Id::parse(ID).unwrap()))
        );
        assert_eq!(env.compose().unwrap(), yaml);
        let day = "event:
  start: 2027-07-08
  end: 2027-07-19
  work: off
";
        assert_eq!(Envelope::parse(day).unwrap().compose().unwrap(), day);
    }

    #[test]
    fn recurring_event_round_trip() {
        let yaml = "event:
  start: 2026-10-05T08:00:00Z
  end: 2026-10-05T09:00:00Z
  timeZone: Europe/Berlin
  schedule: ['RRULE:FREQ=WEEKLY;BYDAY=MO']
";
        let env = Envelope::parse(yaml).unwrap();
        let event = env.roles.event.as_ref().unwrap();
        assert_eq!(event.time_zone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(event.schedule, ["RRULE:FREQ=WEEKLY;BYDAY=MO"]);
        assert!(env.extra.is_empty());
        assert_eq!(Envelope::parse(&env.compose().unwrap()).unwrap(), env);
    }

    #[test]
    fn event_exceptions_round_trip() {
        let yaml = "event:
  start: 2026-10-05T08:00:00Z
  schedule:
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

    #[test]
    fn a_booking_names_its_day_cents_and_other_side() {
        let shop = ContactRef::Name("Shop".into());
        let me = ContactRef::Name("Me".into());
        let out = Booking {
            date: Some(DateTime::from_iso8601("2026-12-01").unwrap()),
            status: Some("pending".into()),
            amount: Some(-12.34),
            sender: Some(me.clone()),
            receiver: Some(shop.clone()),
            ..Booking::default()
        };
        assert!(out.is_pending());
        assert_eq!(out.day(), Some(Date::from_ymd(2026, 12, 1).unwrap()));
        assert_eq!(out.cents(), Some(-1234));
        assert_eq!(out.party(), Some(&shop));
        let back = Booking {
            amount: Some(20.0),
            sender: Some(shop.clone()),
            receiver: Some(me),
            ..Booking::default()
        };
        assert!(!back.is_pending());
        assert_eq!(back.party(), Some(&shop));
        assert_eq!(Booking::default().cents(), None);
    }
}
