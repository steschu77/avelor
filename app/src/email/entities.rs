//! An email and its attachments as entities: the pure mapping of a downloaded
//! message.

use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::email;
use av_vault::envelope::{Email, Remote, Roles, Source};
use av_vault::shared::OneOrMany;

use crate::texts;
use av_vault::schema::remote::{GMAIL_ID, MESSAGE_ID};

// ----------------------------------------------------------------------------
/// A downloaded email, parsed and converted, ready to be written.
pub(super) struct Downloaded {
    pub(super) message_id: String,
    pub(super) subject: String,
    pub(super) from: String,
    pub(super) to: Vec<String>,
    pub(super) cc: Vec<String>,
    pub(super) date: DateTime,
    pub(super) body: String,
}

impl Downloaded {
    // ------------------------------------------------------------------------
    /// The email `raw` (RFC 822) as it is written: its headers, its body as
    /// markdown, dated `date`.
    pub(super) fn parse(raw: &[u8], date: DateTime) -> Self {
        let email = email::parse(raw);
        Self {
            body: email.to_markdown(texts::text("email.attachments")),
            to: email::split_addresses(&email.to),
            cc: email::split_addresses(&email.cc),
            message_id: email.message_id,
            subject: email.subject,
            from: email.from,
            date,
        }
    }
}

// ----------------------------------------------------------------------------
/// A new email entity for `email`, from `account`.
pub(super) fn email_entity(account: Id, gmail_id: &str, email: Downloaded) -> Document {
    let addresses = |list: Vec<String>| match list.is_empty() {
        true => OneOrMany::None,
        false => OneOrMany::Many(list),
    };
    let roles = Roles {
        email: Some(Email {
            from: Some(email.from).filter(|f| !f.is_empty()),
            to: addresses(email.to),
            cc: addresses(email.cc),
            ..Email::default()
        }),
        ..Default::default()
    };

    let mut remote = Remote {
        account: Some(account),
        ..Remote::default()
    };
    if !email.message_id.is_empty() {
        remote.set(MESSAGE_ID, &email.message_id);
    }
    remote.set(GMAIL_ID, gmail_id);
    let source = Source::Remote(remote);

    let subject = email.subject.trim();
    let title = if subject.is_empty() {
        texts::text("email.no_subject")
    } else {
        subject
    };
    let mut doc = Document::create(
        roles,
        source,
        title.to_string(),
        Vec::new(),
        FileFormat::Markdown,
    );
    doc.set_date(email.date);
    doc.set_content(email.body);
    doc
}
