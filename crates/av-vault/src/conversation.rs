//! Conversation trees (vault.md#conversation): the messages of a
//! `conversation` block form a tree through `parent`, the first message has
//! none, and `active` is the leaf of the path shown. The rules for walking
//! and changing the tree; writing it is the index's
//! (`Index::set_conversation`).

use std::collections::HashSet;

use av_utils::error::{Error, Result};
use av_utils::id::Id;

use crate::envelope::{Conversation, ConversationMessage};

impl ConversationMessage {
    // ------------------------------------------------------------------------
    /// Whether an AI model wrote the message (it names the model); a
    /// person's message names none.
    pub fn is_reply(&self) -> bool {
        self.model.is_some()
    }
}

impl Conversation {
    // ------------------------------------------------------------------------
    /// The message `id`, if it is in the tree.
    pub fn message(&self, id: Id) -> Option<&ConversationMessage> {
        self.messages.iter().find(|m| m.id == Some(id))
    }

    // ------------------------------------------------------------------------
    /// The messages from the first one down to `leaf`, following `parent`
    /// (cycles and missing ids end the walk).
    pub fn path_to(&self, leaf: Id) -> Vec<&ConversationMessage> {
        let mut path = Vec::new();
        let mut seen = HashSet::new();
        let mut next = Some(leaf);
        while let Some(id) = next {
            if !seen.insert(id) {
                break;
            }
            let Some(message) = self.message(id) else {
                break;
            };
            path.push(message);
            next = message.parent;
        }
        path.reverse();
        path
    }

    // ------------------------------------------------------------------------
    /// The leaf below `id`, following the newest child (the greatest id: ids
    /// are time-ordered) at each branch; `None` if `id` isn't a message.
    pub fn leaf_from(&self, id: Id) -> Option<Id> {
        self.message(id)?;
        let mut current = id;
        let mut seen = HashSet::from([id]);
        loop {
            let newest = self
                .messages
                .iter()
                .filter(|m| m.parent == Some(current))
                .filter_map(|m| m.id)
                .max();
            match newest {
                Some(child) if seen.insert(child) => current = child,
                _ => return Some(current),
            }
        }
    }

    // ------------------------------------------------------------------------
    /// Add `message`. It becomes `active` if `activate`, or if the shown
    /// path ends at its parent (a reply the user didn't navigate away
    /// from). `Error::NotFound` if its parent isn't in the tree.
    pub fn add(&mut self, message: ConversationMessage, activate: bool) -> Result<()> {
        if let Some(parent) = message.parent {
            self.message(parent).ok_or(Error::NotFound)?;
        }
        if activate || (message.parent.is_some() && self.active == message.parent) {
            self.active = message.id;
        }
        self.messages.push(message);
        Ok(())
    }

    // ------------------------------------------------------------------------
    /// Make `id` the end of the shown path; with `user_only`, it must be a
    /// person's message. Whether `active` changed; `Error::NotFound` if
    /// `id` isn't a message, `Error::WrongKind` if it is a reply and
    /// `user_only`.
    pub fn activate(&mut self, id: Id, user_only: bool) -> Result<bool> {
        let message = self.message(id).ok_or(Error::NotFound)?;
        if user_only && message.is_reply() {
            return Err(Error::WrongKind);
        }
        if self.active == Some(id) {
            return Ok(false);
        }
        self.active = Some(id);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: Id, parent: Option<Id>, model: Option<&str>) -> ConversationMessage {
        ConversationMessage {
            id: Some(id),
            parent,
            model: model.map(str::to_string),
            content: Some(format!("{id}")),
            ..Default::default()
        }
    }

    /// Ids in creation order.
    fn ids<const N: usize>() -> [Id; N] {
        let mut ids = [(); N].map(|()| Id::generate());
        ids.sort();
        ids
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_branch_leads_to_its_newest_leaf() {
        let [q1, a1, q2, a2, a1b, missing] = ids();
        let conversation = Conversation {
            messages: vec![
                message(q1, None, None),
                message(a1, Some(q1), Some("m")),
                message(q2, Some(a1), None),
                message(a2, Some(q2), Some("m")),
                message(a1b, Some(q1), Some("m")),
            ],
            active: Some(a2),
        };
        assert_eq!(conversation.leaf_from(a1), Some(a2));
        assert_eq!(conversation.leaf_from(q1), Some(a1b), "the newest child");
        assert_eq!(conversation.leaf_from(a1b), Some(a1b));
        assert_eq!(conversation.leaf_from(missing), None);
        let path: Vec<_> = conversation.path_to(a2).iter().map(|m| m.id).collect();
        assert_eq!(path, [Some(q1), Some(a1), Some(q2), Some(a2)]);
        assert!(conversation.path_to(missing).is_empty());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_cycle_ends_the_walk() {
        let [a, b] = ids();
        let conversation = Conversation {
            messages: vec![message(a, Some(b), None), message(b, Some(a), None)],
            active: Some(a),
        };
        assert_eq!(conversation.path_to(a).len(), 2);
        assert!(conversation.leaf_from(a).is_some());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_message_needs_its_parent_and_follows_the_shown_path() {
        let [q1, a1, q2, a2, missing] = ids();
        let mut conversation = Conversation::default();
        conversation.add(message(q1, None, None), true).unwrap();
        assert_eq!(conversation.active, Some(q1));

        // A reply to the end of the shown path is shown.
        conversation
            .add(message(a1, Some(q1), Some("m")), false)
            .unwrap();
        assert_eq!(conversation.active, Some(a1));

        // One to a message the user navigated away from is not.
        conversation.add(message(q2, Some(a1), None), true).unwrap();
        conversation.activate(q1, true).unwrap();
        conversation
            .add(message(a2, Some(q2), Some("m")), false)
            .unwrap();
        assert_eq!(conversation.active, Some(q1));

        let orphan = message(missing, Some(Id::generate()), None);
        assert!(matches!(
            conversation.add(orphan, true),
            Err(Error::NotFound)
        ));
        assert_eq!(conversation.messages.len(), 4, "nothing added");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn activating_needs_the_message_and_says_whether_it_changed() {
        let [q1, a1, missing] = ids();
        let mut conversation = Conversation {
            messages: vec![message(q1, None, None), message(a1, Some(q1), Some("m"))],
            active: Some(a1),
        };
        assert!(conversation.activate(q1, true).unwrap());
        assert!(!conversation.activate(q1, true).unwrap(), "unchanged");
        assert!(matches!(
            conversation.activate(a1, true),
            Err(Error::WrongKind)
        ));
        assert!(conversation.activate(a1, false).unwrap());
        assert!(matches!(
            conversation.activate(missing, false),
            Err(Error::NotFound)
        ));
        assert_eq!(conversation.active, Some(a1));
        assert!(conversation.messages[1].is_reply() && !conversation.messages[0].is_reply());
    }
}
