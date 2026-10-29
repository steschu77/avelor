//! A downloaded email applied to the vault: written, replaced or kept, its
//! attachments saved.

use std::collections::HashSet;
use std::path::Path;

use av_utils::error::Result;
use av_utils::id::Id;
use av_vault::email;
use av_vault::envelope::DocumentType;

use super::entities::{AttachedTo, Downloaded, attachment_entity, email_entity};
use crate::app::App;
use crate::sys;
use av_vault::schema::layout;
use av_vault::schema::remote::{GMAIL_ID, MESSAGE_ID};
use av_vault::vault::Index;

// ----------------------------------------------------------------------------
/// How one email ended up in the vault.
#[derive(Debug, PartialEq)]
pub(super) enum Stored {
    /// A new entity, by its id.
    Created(Id),
    Replaced,
    /// Already in the vault, and not to be replaced.
    Skipped,
}

// ----------------------------------------------------------------------------
/// What to do with a downloaded email.
#[derive(Debug, PartialEq)]
pub(super) enum Action {
    Create,
    /// Replace the body of the entity that has the email.
    Replace(Id),
    Skip,
}

// ----------------------------------------------------------------------------
/// What to do with an email the vault has as `existing` (or doesn't);
/// `replace`: the user confirmed its body may be replaced.
pub(super) fn fetch_action(existing: Option<Id>, replace: bool) -> Action {
    match existing {
        None => Action::Create,
        Some(id) if replace => Action::Replace(id),
        Some(_) => Action::Skip,
    }
}

// ----------------------------------------------------------------------------
/// The entity the vault has an email as: by its Gmail id from `account`, else
/// by its Message-ID from anywhere.
pub(super) fn existing_email(
    index: &Index,
    account: Id,
    gmail_id: &str,
    message_id: &str,
) -> Option<Id> {
    index
        .find_remote(DocumentType::Email, Some(&account), GMAIL_ID, gmail_id)
        .or_else(|| {
            (!message_id.is_empty())
                .then(|| index.find_remote(DocumentType::Email, None, MESSAGE_ID, message_id))
                .flatten()
        })
}

// ----------------------------------------------------------------------------
/// What saving an email's attachments came to: the documents written, those
/// it had already whose text is still empty (to read), and the files that
/// failed.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Saved {
    pub(super) created: Vec<Id>,
    pub(super) unread: Vec<Id>,
    pub(super) failed: usize,
}

// ----------------------------------------------------------------------------
/// The apply step: write `email` into the vault as `fetch_action` says -- a
/// new entity in `Email/`, the body of the one already there replaced, or
/// nothing -- then the attachments it doesn't have yet
/// (`save_attachments`).
pub(super) fn apply_email(
    app: &mut App,
    account: Id,
    gmail_id: &str,
    mut email: Downloaded,
    replace: bool,
) -> Result<(Stored, Saved)> {
    let existing = existing_email(app.vault.index(), account, gmail_id, &email.message_id);
    let parts = std::mem::take(&mut email.attachments);
    let (message_id, date) = (email.message_id.clone(), email.date);
    // A replaced body goes to the page with the entity (`App::edit`), so
    // a preview showing the old one loads it.
    let (stored, id) = match fetch_action(existing, replace) {
        Action::Skip => return Ok((Stored::Skipped, Saved::default())),
        Action::Replace(id) => {
            app.edit_written(|index, changes| index.set_content(&id, email.body, changes))?;
            (Stored::Replaced, id)
        }
        Action::Create => {
            let doc = email_entity(account, gmail_id, email);
            let folder = Path::new(layout::EMAIL);
            let id = app.edit(|index, changes| index.create(doc, folder, changes))?;
            (Stored::Created(id), id)
        }
    };
    let to = AttachedTo {
        email: id,
        account,
        gmail_id,
        message_id: &message_id,
        date,
    };
    Ok((stored, save_attachments(app, &to, parts)))
}

// ----------------------------------------------------------------------------
/// Write each of `parts` that `to.email` has no document for yet -- one
/// linked to it whose original has the part's name and size -- as a
/// document with its original in `Attachments/`, each in a vault edit of
/// its own, the file marked as downloaded (`sys::mark_downloaded`). A part
/// that fails is logged and counted; the others are still written. The
/// documents `to.email` has already without text are named too.
fn save_attachments(app: &mut App, to: &AttachedTo, parts: Vec<email::Attachment>) -> Saved {
    let mut saved = Saved {
        unread: unread_originals(app.vault.index(), to.email),
        ..Saved::default()
    };
    if parts.is_empty() {
        return saved;
    }
    let has = saved_originals(app.vault.index(), to.email);
    let folder = Path::new(layout::ATTACHMENTS);
    for part in parts {
        if has.contains(&(part.filename.clone(), part.bytes.len() as u64)) {
            continue;
        }
        let doc = attachment_entity(to, &part);
        let written = app.edit(|index, changes| {
            index.create_with_original(doc, folder, &part.filename, &part.bytes, changes)
        });
        match written {
            Ok(id) => {
                saved.created.push(id);
                if let Some(path) = app.vault.index().original_path(&id) {
                    sys::mark_downloaded(&path);
                }
            }
            Err(e) => {
                let name = &part.filename;
                log::warn!("Could not save {name} of email {}: {e}", to.gmail_id);
                saved.failed += 1;
            }
        }
    }
    saved
}

// ----------------------------------------------------------------------------
/// The name and size of each original of the documents linked to `email`.
fn saved_originals(index: &Index, email: Id) -> HashSet<(String, u64)> {
    index
        .attachments_of(email)
        .filter_map(|entry| {
            let original = entry.doc.original()?;
            Some((original.name.clone()?, original.size?))
        })
        .collect()
}

// ----------------------------------------------------------------------------
/// The documents linked to `email` that have an original and no text yet.
fn unread_originals(index: &Index, email: Id) -> Vec<Id> {
    index
        .attachments_of(email)
        .filter(|entry| entry.doc.content().trim().is_empty())
        .map(|entry| entry.doc.id())
        .collect()
}
