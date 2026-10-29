//! An email and its attachments as entities: the pure mapping of a downloaded
//! message.

use av_utils::datetime::DateTime;
use av_utils::id::Id;
use av_vault::document::{Document, FileFormat};
use av_vault::email;
use av_vault::envelope::{Email, Remote, Roles, Source};
use av_vault::shared::OneOrMany;

use crate::google::accounts::AttachmentSettings;
use crate::texts;
use crate::watches::glob_match;
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
    /// The attached files to save (`attachment_wanted`).
    pub(super) attachments: Vec<email::Attachment>,
}

impl Downloaded {
    // ------------------------------------------------------------------------
    /// The email `raw` (RFC 822) as it is written: its headers, its body as
    /// markdown, dated `date`; with the attachments `keep` wants.
    pub(super) fn parse(raw: &[u8], date: DateTime, keep: Option<&AttachmentSettings>) -> Self {
        let email = email::parse(raw);
        let body = email.to_markdown(texts::text("email.attachments"));
        let attachments = match keep {
            Some(keep) => email
                .attachments
                .into_iter()
                .filter(|part| attachment_wanted(keep, part))
                .collect(),
            None => Vec::new(),
        };
        Self {
            body,
            to: email::split_addresses(&email.to),
            cc: email::split_addresses(&email.cc),
            message_id: email.message_id,
            subject: email.subject,
            from: email.from,
            date,
            attachments,
        }
    }
}

// ----------------------------------------------------------------------------
/// Whether `part` is a file `settings` saves: attached by the sender (not
/// an image the HTML shows), matching one of its patterns, within its size.
fn attachment_wanted(settings: &AttachmentSettings, part: &email::Attachment) -> bool {
    let max = u64::from(settings.max_size) * 1024 * 1024;
    !part.inline
        && part.bytes.len() as u64 <= max
        && settings
            .patterns
            .iter()
            .any(|p| glob_match(p, &part.filename))
}

// ----------------------------------------------------------------------------
/// The email an attachment came with, as its document names it.
pub(super) struct AttachedTo<'a> {
    /// The email's entity.
    pub(super) email: Id,
    pub(super) account: Id,
    pub(super) gmail_id: &'a str,
    pub(super) message_id: &'a str,
    pub(super) date: DateTime,
}

// ----------------------------------------------------------------------------
/// A new document for the attached file `part` (email.md#attachments):
/// titled by its name without the extension, dated as its email, linked to
/// it, its source the email's. Its original is named when it is written
/// (`Index::create_with_original`).
pub(super) fn attachment_entity(to: &AttachedTo, part: &email::Attachment) -> Document {
    let roles = Roles {
        document: Some(Default::default()),
        ..Default::default()
    };
    let source = Source::Remote(email_remote(to.account, to.gmail_id, to.message_id));
    let name = part.filename.trim();
    let title = match name.rsplit_once('.') {
        Some((stem, _)) if !stem.trim().is_empty() => stem.trim(),
        _ => name,
    };
    let mut doc = Document::create(
        roles,
        source,
        title.to_string(),
        Vec::new(),
        FileFormat::Markdown,
    );
    doc.set_date(to.date);
    doc.set_link(Some(to.email));
    doc
}

// ----------------------------------------------------------------------------
/// The `remote:` block of an email's entities (vault.md#remote): its
/// account, Message-ID (if it has one) and Gmail id.
fn email_remote(account: Id, gmail_id: &str, message_id: &str) -> Remote {
    let mut remote = Remote {
        account: Some(account),
        remote: Vec::new(),
    };
    if !message_id.is_empty() {
        remote.set(MESSAGE_ID, message_id);
    }
    remote.set(GMAIL_ID, gmail_id);
    remote
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
            subject: None,
            kind: None,
            from: Some(email.from).filter(|f| !f.is_empty()),
            to: addresses(email.to),
            cc: addresses(email.cc),
            bcc: OneOrMany::None,
        }),
        ..Default::default()
    };

    let source = Source::Remote(email_remote(account, gmail_id, &email.message_id));

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
