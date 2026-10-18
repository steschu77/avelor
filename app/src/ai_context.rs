//! What the AI is told about the item a conversation is about
//! (assistant.md#context, the context builder): the linked document,
//! or a thread's members, each as its file -- YAML frontmatter, then its
//! text -- the way the model should see it, within a budget.

use std::borrow::Cow;

use av_utils::id::Id;
use av_vault::document::Document;
use av_vault::envelope::Envelope;

use av_vault::vault::Index;

/// The characters of text (bodies) one context may hold, about 10k tokens.
pub const BUDGET: usize = 40_000;

/// What a body cut at the budget ends with.
const CUT: &str = "[\u{2026} the rest is left out]";

/// What stands for a thread member's body that didn't fit.
const LEFT_OUT: &str = "(left out: the thread is long)";

/// How the files are given, after the intro's first sentence.
const FILES: &str = "(YAML frontmatter, then its text). Answer about it unless they ask \
about something else.";

// ----------------------------------------------------------------------------
/// What the AI is told about the entity `id` a conversation is linked to:
/// a thread's members, or the entity itself (a project its own
/// description); `None` if it is gone.
pub fn describe(index: &Index, id: Id) -> Option<String> {
    let doc = &index.entry(&id)?.doc;
    let titles = |doc: &Document| -> Vec<String> {
        let projects = doc.projects().iter().filter_map(|p| index.entry(p));
        projects.map(|e| e.doc.title().to_string()).collect()
    };
    if doc.roles().thread.is_none() {
        return Some(document_context(doc, &titles(doc), BUDGET));
    }
    let members = index.thread_members(&id);
    let members: Vec<(String, &str)> = members
        .iter()
        .map(|e| (frontmatter(&e.doc, &titles(&e.doc)), e.doc.content()))
        .collect();
    Some(thread_context(doc.title(), &members, BUDGET))
}

// ----------------------------------------------------------------------------
/// The entity's frontmatter as the model sees it: title, date, labels, the
/// projects by title, its role blocks and other keys -- without what only
/// the app reads (`id`, `linked-to`, the source block with its remote ids,
/// the flags, thread ids and a conversation).
fn frontmatter(doc: &Document, projects: &[String]) -> String {
    let mut envelope: Envelope = doc.envelope().cloned().unwrap_or_default();
    if envelope.title.is_none() && !doc.title().is_empty() {
        envelope.title = Some(doc.title().to_string());
    }
    envelope.id = None;
    envelope.linked_to = None;
    envelope.rename = None;
    envelope.starred = None;
    envelope.archived = None;
    envelope.source = None;
    envelope.projects.clear();
    if !projects.is_empty() {
        envelope.set_extra("projects", projects.to_vec());
    }
    let roles = &mut envelope.roles;
    roles.conversation = None;
    if let Some(message) = &mut roles.message {
        message.threads.clear();
        if message.is_empty() {
            roles.message = None;
        }
    }
    envelope.compose().unwrap_or_default()
}

// ----------------------------------------------------------------------------
/// A file: its frontmatter, then its body.
fn file(frontmatter: &str, body: &str) -> String {
    format!("---\n{frontmatter}---\n\n{body}\n")
}

// ----------------------------------------------------------------------------
/// `text` cut to `budget` characters and marked so, or as it is.
fn cut(text: &str, budget: usize) -> Cow<'_, str> {
    match text.char_indices().nth(budget) {
        None => Cow::Borrowed(text),
        Some((end, _)) => Cow::Owned(format!("{}\n\n{CUT}", text[..end].trim_end())),
    }
}

// ----------------------------------------------------------------------------
/// The context of one entity, its body cut at `budget` characters.
fn document_context(doc: &Document, projects: &[String], budget: usize) -> String {
    let kind = doc.kind();
    let body = cut(doc.content().trim(), budget);
    format!(
        "The user is asking about this {kind} from their vault, given as its file {FILES}\n\n{}",
        file(&frontmatter(doc, projects), &body)
    )
}

// ----------------------------------------------------------------------------
/// The context of the thread `title`: its `members` (frontmatter, body) in
/// date order, each under a heading. The bodies go in from the newest back
/// while they fit `budget` characters -- the newest is cut if it alone is
/// too long -- and the older ones are left out; every heading stays.
fn thread_context(title: &str, members: &[(String, &str)], budget: usize) -> String {
    let n = members.len();
    let mut bodies: Vec<Option<Cow<str>>> = vec![None; n];
    let mut left = budget;
    for (i, (_, body)) in members.iter().enumerate().rev() {
        let body = body.trim();
        let len = body.chars().count();
        if len <= left {
            bodies[i] = Some(Cow::Borrowed(body));
            left -= len;
        } else {
            if i + 1 == n {
                bodies[i] = Some(cut(body, left));
            }
            break;
        }
    }
    let left_out = bodies.iter().filter(|b| b.is_none()).count();
    let messages = if n == 1 {
        "message follows"
    } else {
        "messages follow"
    };
    let mut text = format!(
        "The user is asking about the thread \u{201C}{title}\u{201D} from their vault: its {n} \
         {messages}, oldest first, each as its file {FILES}"
    );
    if left_out > 0 {
        text.push_str(&format!(
            " The text of the oldest {left_out} is left out: the thread is long."
        ));
    }
    for (i, ((frontmatter, _), body)) in members.iter().zip(&bodies).enumerate() {
        let body = body.as_deref().unwrap_or(LEFT_OUT);
        text.push_str(&format!(
            "\n\n## {} of {n}\n\n{}",
            i + 1,
            file(frontmatter, body)
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use av_vault::document::FileFormat;

    const MAIL: &str = "---
id: 01a0e1cc-a24c-7490-82fe-e75669985b50
title: Telekom invoice
date: 2026-09-12T08:00:00Z
tags: [bills]
projects: [01a0e1cc-a24c-7490-82fe-e75669985b51]
linked-to: 01a0e1cc-a24c-7490-82fe-e75669985b52
starred: true
remote:
  account: 01a0e1cc-a24c-7490-82fe-e75669985b53
  gmail-id: 18c0ffee
email:
  from: Telekom <rechnung@telekom.de>
  to: jan@example.com
message:
  threads: [01a0e1cc-a24c-7490-82fe-e75669985b54]
custom: kept
---
Please pay 40 EUR by Friday.
";

    fn mail() -> Document {
        Document::load(MAIL.to_string(), FileFormat::Markdown)
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_document_is_given_as_its_file_without_what_only_the_app_reads() {
        let text = document_context(&mail(), &["Flat".to_string()], BUDGET);
        assert!(
            text.starts_with("The user is asking about this email from their vault"),
            "{text}"
        );
        for kept in [
            "title: Telekom invoice",
            "date: 2026-09-12",
            "- bills",
            "- Flat",
            "from: Telekom <rechnung@telekom.de>",
            "custom: kept",
            "---\n\nPlease pay 40 EUR by Friday.\n",
        ] {
            assert!(text.contains(kept), "{kept} in {text}");
        }
        for dropped in [
            "id:",
            "linked-to",
            "starred",
            "gmail-id",
            "remote",
            "threads",
            "01a0e1cc",
        ] {
            assert!(!text.contains(dropped), "{dropped} in {text}");
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_body_past_the_budget_is_cut() {
        let text = document_context(&mail(), &[], 10);
        assert!(
            text.contains("Please pay\n\n[\u{2026} the rest is left out]"),
            "{text}"
        );
        assert_eq!(cut("äöü", 2), "äö\n\n[\u{2026} the rest is left out]");
        assert_eq!(cut("short", 5), "short");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_thread_keeps_every_heading_and_drops_the_oldest_bodies_first() {
        let members = [
            ("title: First\n".to_string(), "Oldest body."),
            ("title: Second\n".to_string(), "Middle body."),
            ("title: Third\n".to_string(), "Newest body."),
        ];
        let all = thread_context("Kitchen", &members, BUDGET);
        assert!(all.contains(
            "the thread \u{201C}Kitchen\u{201D} from their vault: its 3 messages follow"
        ));
        assert!(!all.contains("left out"), "{all}");
        let order = [
            "## 1 of 3",
            "title: First",
            "Oldest body.",
            "## 3 of 3",
            "Newest body.",
        ];
        let positions: Vec<usize> = order.iter().map(|s| all.find(s).unwrap()).collect();
        assert!(positions.windows(2).all(|w| w[0] < w[1]), "{all}");

        // Room for the two newest bodies only.
        let some = thread_context("Kitchen", &members, 24);
        assert!(
            some.contains("The text of the oldest 1 is left out"),
            "{some}"
        );
        assert!(
            some.contains("title: First\n---\n\n(left out: the thread is long)"),
            "{some}"
        );
        assert!(some.contains("Middle body.") && some.contains("Newest body."));

        // The newest alone is too long: cut, the others left out.
        let newest = thread_context("Kitchen", &members, 6);
        assert!(
            newest.contains("The text of the oldest 2 is left out"),
            "{newest}"
        );
        assert!(
            newest.contains("Newest\n\n[\u{2026} the rest is left out]"),
            "{newest}"
        );
        assert_eq!(newest.matches("## ").count(), 3, "every heading stays");
    }
}
