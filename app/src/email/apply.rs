//! A downloaded email applied to the vault: written, replaced or kept, its
//! attachments saved.

use std::path::Path;

use av_utils::error::Result;
use av_utils::id::Id;

use super::entities::{Downloaded, email_entity};
use crate::app::App;
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
fn existing_email(index: &Index, account: Id, gmail_id: &str, message_id: &str) -> Option<Id> {
    index
        .find_remote(Some(&account), GMAIL_ID, gmail_id)
        .or_else(|| {
            (!message_id.is_empty())
                .then(|| index.find_remote(None, MESSAGE_ID, message_id))
                .flatten()
        })
}

// ----------------------------------------------------------------------------
/// The apply step: write `email` into the vault as `fetch_action` says -- a
/// new entity in `Email/`, the body of the one already there replaced, or
/// nothing.
pub(super) fn apply_email(
    app: &mut App,
    account: Id,
    gmail_id: &str,
    email: Downloaded,
    replace: bool,
) -> Result<Stored> {
    let existing = existing_email(app.vault.index(), account, gmail_id, &email.message_id);
    // A replaced body goes to the page with the entity (`App::edit`), so
    // a preview showing the old one loads it.
    match fetch_action(existing, replace) {
        Action::Skip => Ok(Stored::Skipped),
        Action::Replace(id) => {
            app.edit_written(|index, changes| index.set_content(&id, email.body, changes))?;
            Ok(Stored::Replaced)
        }
        Action::Create => {
            let doc = email_entity(account, gmail_id, email);
            let folder = Path::new(layout::EMAIL);
            let id = app.edit(|index, changes| index.create(doc, folder, changes))?;
            Ok(Stored::Created(id))
        }
    }
}
