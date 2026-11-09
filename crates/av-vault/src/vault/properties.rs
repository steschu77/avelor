//! An entity's properties changed: its title, labels, links, a transaction's
//! fields, its body.

use super::{Changes, Index, SaveOutcome};
use crate::change::Change;
use crate::document::Document;
use crate::envelope::Conversation;
use crate::envelope::Transaction;
use av_utils::datetime::DateTime;
use av_utils::error::Error;
use av_utils::error::Result;
use av_utils::id::Id;
use av_utils::text::unique_trimmed;

// ----------------------------------------------------------------------------
impl Index {
    /// Set the title of `id`, which may be any entity. An empty title removes
    /// it ([`Document::set_title`]). Only the file's content changes; the file
    /// is not renamed to match (vault.md#filenames).
    pub fn set_title(
        &mut self,
        id: &Id,
        title: String,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_title(title))
    }

    /// Set the labels (`tags`) of `id`, which may be any entity: each one
    /// trimmed, empty ones dropped, a repeated one kept once where it first
    /// appears. No labels removes the key.
    pub fn set_labels(
        &mut self,
        id: &Id,
        labels: Vec<String>,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        let labels = unique_trimmed(labels);
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_labels(labels))
    }

    /// Link `id`, which may be any entity, to `to`, another entity in the
    /// vault, replacing an older link; `None` removes the `linked-to` key
    /// ([`Document::set_link`]). Deleting an entity leaves the links to it
    /// in place, so restoring it brings them back.
    pub fn set_link(
        &mut self,
        id: &Id,
        to: Option<Id>,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_link(id, to.as_ref())?;
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_link(to))
    }

    /// Link the transaction `transaction` of the statement `id` to `to`, an entity
    /// in the vault other than the statement, replacing an older link;
    /// `None` removes the transaction's `linked-to` key. A transaction is an item of
    /// its statement's file, so the statement is written.
    pub fn set_transaction_link(
        &mut self,
        id: &Id,
        transaction: &Id,
        to: Option<Id>,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_link(id, to.as_ref())?;
        self.check_writable(id)?;
        self.edit_transaction(id, transaction, changes, |item| item.linked_to = to)
    }

    /// Unpair the transfer whose half is the transaction `transaction` of the
    /// statement `id`: its `transfer` key is removed, and so is the other
    /// half's, in whichever statement of the vault holds it, while it names
    /// this transaction back. Both statements are written; a transaction that is no
    /// transfer's half is left as it is.
    pub fn unpair_transfer(
        &mut self,
        id: &Id,
        transaction: &Id,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        let Some(other) = self.transaction(id, transaction)?.transfer else {
            return Ok(SaveOutcome::Unchanged);
        };
        let names_back =
            |item: &Transaction| item.id == Some(other) && item.transfer == Some(*transaction);
        let holder = self.documents.iter().map(|e| &e.doc).find(|doc| {
            let statement = doc.roles().statement.as_ref();
            statement.is_some_and(|t| t.transactions.iter().any(names_back))
        });
        let holder = holder.map(Document::id);
        if let Some(holder) = &holder {
            self.check_writable(holder)?;
        }
        let rewritten: Vec<Id> = std::iter::once(*id).chain(holder).collect();
        if let Some(changed) = self.first_changed(&rewritten)? {
            return Ok(SaveOutcome::Conflict(changed));
        }

        let outcome =
            self.edit_transaction(id, transaction, changes, |item| item.transfer = None)?;
        match holder {
            Some(holder) if !matches!(outcome, SaveOutcome::Conflict(_)) => {
                let unpaired = |item: &mut Transaction| item.transfer = None;
                Ok(outcome.and(self.edit_transaction(&holder, &other, changes, unpaired)?))
            }
            _ => Ok(outcome),
        }
    }

    /// The transaction `transaction` of the statement `id`; `invalid_argument` if
    /// the statement has none of that id.
    fn transaction(&self, id: &Id, transaction: &Id) -> Result<&Transaction> {
        let statement = self.get(id).and_then(|doc| doc.roles().statement.as_ref());
        let items = statement
            .map(|t| t.transactions.as_slice())
            .unwrap_or_default();
        let found = items
            .iter()
            .find(|item| item.id.as_ref() == Some(transaction));
        found.ok_or_else(|| Error::InvalidArgument {
            arg: "transaction".into(),
        })
    }

    /// Apply `change` to the transaction `transaction` of the statement `id`, and
    /// write the statement: a transaction is an item of its statement's file.
    fn edit_transaction(
        &mut self,
        id: &Id,
        transaction: &Id,
        changes: &mut Changes,
        mut change: impl FnMut(&mut Transaction),
    ) -> Result<SaveOutcome> {
        self.transaction(id, transaction)?;
        let is_it = |item: &Transaction| item.id.as_ref() == Some(transaction);
        let mut statement = self
            .get(id)
            .and_then(|doc| doc.roles().statement.clone())
            .unwrap_or_default();
        for item in statement.transactions.iter_mut().filter(|item| is_it(item)) {
            change(item);
        }
        self.update(id, changes, |d| d.set_statement(Some(statement)))
    }

    /// Star or unstar `id`, which may be any entity; unstarring removes the
    /// key ([`Document::set_starred`]). Nothing is written if it already is.
    pub fn set_starred(
        &mut self,
        id: &Id,
        starred: bool,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_starred(starred))
    }

    /// Archive `id`, which may be any entity, or take it out of the archive;
    /// unarchiving removes the key ([`Document::set_archived`]). Nothing is
    /// written if it already is.
    pub fn set_archived(
        &mut self,
        id: &Id,
        archived: bool,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_archived(archived))
    }

    /// Replace the markdown body of `id`, keeping its frontmatter.
    pub fn set_content(
        &mut self,
        id: &Id,
        content: String,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_content(content))
    }

    /// Apply `change` to `id`, which may be any entity, as one write: for
    /// edits that touch several properties at once (e.g. a synced event
    /// taking a remote version). Nothing is written if nothing changed.
    pub fn edit(
        &mut self,
        id: &Id,
        changes: &mut Changes,
        change: impl FnOnce(&mut Document),
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, change)
    }

    /// Write `change` to each of `ids` as its own write: the outcome per
    /// id, in their order. An entity the change doesn't fit ([`Change::fits`])
    /// is refused (`InvalidArgument`), one that can't be written as by
    /// [`Index::edit`]; either way the others are still written. `now` is
    /// when a task is marked done.
    pub fn edit_entities(
        &mut self,
        ids: &[Id],
        change: &Change,
        now: DateTime,
        changes: &mut Changes,
    ) -> Vec<(Id, Result<SaveOutcome>)> {
        ids.iter()
            .map(|id| {
                let outcome = match self.get(id) {
                    None => Err(Error::NotFound),
                    Some(doc) if !change.fits(doc.kind()) => Err(Error::InvalidArgument {
                        arg: "change".into(),
                    }),
                    Some(_) => self.edit(id, changes, |d| change.apply(d, now)),
                };
                (*id, outcome)
            })
            .collect()
    }

    /// Replace the `conversation` block of `id`, which may be any entity;
    /// `None` drops the role ([`Document::set_conversation`]).
    pub fn set_conversation(
        &mut self,
        id: &Id,
        conversation: Option<Conversation>,
        changes: &mut Changes,
    ) -> Result<SaveOutcome> {
        self.check_writable(id)?;
        self.update(id, changes, |d| d.set_conversation(conversation))
    }
}
