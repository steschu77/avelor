//! What the AI is told about the item a conversation is about
//! (assistant.md#context, the context builder): the linked document,
//! or a thread's members, each as its file -- YAML frontmatter, then its
//! text, an email's followed by its attachments' -- the way the model should
//! see it, within a budget. Each entity as far as its classification
//! reaches the model (`ai::policy`): whole, its values as placeholders, or
//! left out, which the reply names.

use std::borrow::Cow;
use std::fmt::Display;

use av_utils::id::Id;
use av_utils::text::cut_marked;
use av_vault::document::Document;
use av_vault::envelope::Envelope;
use av_vault::shared::Classification;
use av_vault::vault::Index;

use crate::ai::policy::{self, Placeholders, Reach};
use crate::texts::Text;

/// The characters of text (bodies) one context may hold, about 10k tokens.
pub const BUDGET: usize = 40_000;

/// What a body cut at the budget ends with.
const CUT: &str = "\n\n[\u{2026} the rest is left out]";

/// What stands for a thread member's body that didn't fit.
const LEFT_OUT: &str = "(left out: the thread is long)";

/// How the files are given, after the intro's first sentence.
const FILES: &str = "(YAML frontmatter, then its text). Answer about it unless they ask \
about something else.";

/// What the model is told of placeholders, when it got any.
const PLACEHOLDERS: &str = "Values in braces, like {checking-iban}, stand for personal data \
the user's PC keeps from you: where such a value belongs, write its placeholder as it is, \
and the user sees the value.";

/// What stands for the body of an entity given as placeholders.
const WITHHELD: &str = "(left out: personal, kept on the user's PC)";

/// What stands for a thread member its classification keeps out.
const MEMBER_LEFT_OUT: (&str, &str) = ("left-out: true\n", "(left out: its classification)");

// ----------------------------------------------------------------------------
/// What a request may carry (`ai::policy`): whether its model runs on this
/// PC, and the vault's default classification.
pub struct Gate<'a> {
    pub local: bool,
    pub default: &'a Classification,
}

// ----------------------------------------------------------------------------
/// The context of a conversation: its text, the placeholders it gave, and
/// what it left out (each by its title and why), for the reply to name.
pub struct Described {
    pub text: String,
    pub placeholders: Placeholders,
    pub left_out: Vec<String>,
}

// ----------------------------------------------------------------------------
/// What the AI is told about the entity `id` a conversation is linked to:
/// a thread's members, or the entity itself (a project its own
/// description), each body with its attachments' (`Seen::body_of`), as far
/// as `gate` lets each reach; `None` if it is gone.
pub fn describe(index: &Index, id: Id, gate: &Gate) -> Option<Described> {
    let doc = &index.entry(&id)?.doc;
    let mut seen = Seen {
        gate,
        placeholders: Placeholders::new(),
        left_out: Vec::new(),
    };
    let mut text = if doc.roles().thread.is_none() {
        match seen.file(index, doc) {
            Some((frontmatter, body)) => document_context(doc.kind(), &frontmatter, &body, BUDGET),
            None => left_out_context(doc.kind()),
        }
    } else {
        let members = index.thread_members(&id);
        let files: Vec<(String, Cow<str>)> = members
            .iter()
            .map(|e| {
                seen.file(index, &e.doc).unwrap_or_else(|| {
                    let (frontmatter, body) = MEMBER_LEFT_OUT;
                    (frontmatter.to_string(), Cow::Borrowed(body))
                })
            })
            .collect();
        let members: Vec<(String, &str)> = files
            .iter()
            .map(|(frontmatter, body)| (frontmatter.clone(), body.as_ref()))
            .collect();
        thread_context(doc.title(), &members, BUDGET)
    };
    if !seen.placeholders.is_empty() {
        text.push_str("\n\n");
        text.push_str(PLACEHOLDERS);
    }
    Some(Described {
        text,
        placeholders: seen.placeholders,
        left_out: seen.left_out,
    })
}

// ----------------------------------------------------------------------------
/// The entities of one context as far as the gate lets them reach.
struct Seen<'g> {
    gate: &'g Gate<'g>,
    placeholders: Placeholders,
    left_out: Vec<String>,
}

impl Seen<'_> {
    /// How far `doc` reaches the model.
    fn reach(&self, doc: &Document) -> Reach {
        policy::reach(&self.class(doc), self.gate.local)
    }

    fn class(&self, doc: &Document) -> Classification {
        policy::class_of(doc, self.gate.default)
    }

    /// `doc` named as left out, and why.
    fn leave_out(&mut self, doc: &Document) {
        let class = policy::class_text(&self.class(doc));
        let text = Text::new("ai.left_out")
            .arg("title", doc.title())
            .arg("class", class);
        self.left_out.push(text.render());
    }

    /// `doc` as the model gets it, its frontmatter and its body; `None`
    /// if it is left out.
    fn file<'a>(&mut self, index: &Index, doc: &'a Document) -> Option<(String, Cow<'a, str>)> {
        let projects: Vec<String> = doc
            .projects()
            .iter()
            .filter_map(|p| index.entry(p))
            .map(|e| e.doc.title().to_string())
            .collect();
        match self.reach(doc) {
            Reach::Whole => Some((frontmatter(doc, &projects), self.body_of(index, doc))),
            Reach::Placeholders => {
                let mut value = serde_yaml_ng::to_value(given(doc, &projects)).ok()?;
                let named = policy::with_placeholders(
                    &mut value,
                    doc.id(),
                    doc.title(),
                    &mut self.placeholders,
                );
                if named.is_none() {
                    self.leave_out(doc);
                    return None;
                }
                let text = Text::new("ai.left_out_values").arg("title", doc.title());
                self.left_out.push(text.render());
                let frontmatter = serde_yaml_ng::to_string(&value).unwrap_or_default();
                Some((frontmatter, Cow::Borrowed(WITHHELD)))
            }
            Reach::Nothing => {
                self.leave_out(doc);
                None
            }
        }
    }

    /// The body of `doc` as the model sees it: its text, then its
    /// attachments' (`Index::attachments_of`) that were read and reach the
    /// model, each under a line naming its file and title
    /// (`with_attachments`).
    fn body_of<'a>(&mut self, index: &Index, doc: &'a Document) -> Cow<'a, str> {
        let mut attached: Vec<(String, &str, &str)> = Vec::new();
        for e in index.attachments_of(doc.id()) {
            let text = e.doc.content().trim();
            let Some(original) = e.doc.original() else {
                continue;
            };
            if text.is_empty() {
                continue;
            }
            if self.reach(&e.doc) != Reach::Whole {
                self.leave_out(&e.doc);
                continue;
            }
            let name = original.name.clone();
            let name = name.unwrap_or_else(|| e.doc.title().to_string());
            attached.push((name, e.doc.title(), text));
        }
        with_attachments(doc.content(), &attached)
    }
}

// ----------------------------------------------------------------------------
/// `text`, then each of `attached` -- its file's name, its title, its read
/// text -- under a line naming them. Coming last, the attachments are what
/// a budget cuts first.
fn with_attachments<'a>(text: &'a str, attached: &[(String, &str, &str)]) -> Cow<'a, str> {
    if attached.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut body = text.trim_end().to_string();
    for (name, title, read) in attached {
        body.push_str(&format!(
            "\n\n[Attached file \u{201C}{name}\u{201D} (\u{201C}{title}\u{201D}), as read:]\n\n{read}"
        ));
    }
    Cow::Owned(body)
}

// ----------------------------------------------------------------------------
/// The entity's frontmatter as the model sees it (`given`), as YAML.
fn frontmatter(doc: &Document, projects: &[String]) -> String {
    given(doc, projects).compose().unwrap_or_default()
}

// ----------------------------------------------------------------------------
/// The entity's frontmatter as the model sees it: title, date, labels, the
/// projects by title, its role blocks and other keys -- without what only
/// the app reads (`id`, `linked-to`, the source block with its remote ids,
/// the flags, thread ids and a conversation).
fn given(doc: &Document, projects: &[String]) -> Envelope {
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
    envelope
}

// ----------------------------------------------------------------------------
/// A file: its frontmatter, then its body.
fn file(frontmatter: &str, body: &str) -> String {
    format!("---\n{frontmatter}---\n\n{body}\n")
}

// ----------------------------------------------------------------------------
/// The context of one entity of `kind`, its `body` cut at `budget`
/// characters.
fn document_context(kind: impl Display, frontmatter: &str, body: &str, budget: usize) -> String {
    let body = cut_marked(body.trim(), budget, CUT);
    format!(
        "The user is asking about this {kind} from their vault, given as its file {FILES}\n\n{}",
        file(frontmatter, &body)
    )
}

// ----------------------------------------------------------------------------
/// The context of one entity of `kind` its classification keeps out.
fn left_out_context(kind: impl Display) -> String {
    format!(
        "The user is asking about a {kind} from their vault that its classification keeps \
         from you: say so, and answer what you can without it."
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
                bodies[i] = Some(cut_marked(body, left, CUT));
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

    const EMAIL: &str = "---
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

    fn email() -> Document {
        Document::load(EMAIL.to_string(), FileFormat::Markdown)
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_document_is_given_as_its_file_without_what_only_the_app_reads() {
        let email = email();
        let fm = frontmatter(&email, &["Flat".to_string()]);
        let text = document_context(email.kind(), &fm, email.content(), BUDGET);
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
        let email = email();
        let text = document_context(email.kind(), &frontmatter(&email, &[]), email.content(), 10);
        assert!(
            text.contains("Please pay\n\n[\u{2026} the rest is left out]"),
            "{text}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_mails_attachments_follow_its_text_and_are_cut_first() {
        let attached = [(
            "Rechnung.pdf".to_string(),
            "Rechnung",
            "Betrag: 40 EUR, Rabatt 5 %",
        )];
        let body = with_attachments("Please pay 40 EUR by Friday.\n", &attached);
        assert_eq!(
            body,
            "Please pay 40 EUR by Friday.\n\n[Attached file \u{201C}Rechnung.pdf\u{201D} (\u{201C}Rechnung\u{201D}), as read:]\n\nBetrag: 40 EUR, Rabatt 5 %"
        );
        assert!(matches!(
            with_attachments("Text", &[]),
            Cow::Borrowed("Text")
        ));

        let email = email();
        let room = "Please pay 40 EUR by Friday.".chars().count() + 30;
        let text = document_context(email.kind(), &frontmatter(&email, &[]), &body, room);
        assert!(
            text.contains("Please pay 40 EUR by Friday.\n\n[Attached file"),
            "{text}"
        );
        assert!(text.contains("[\u{2026} the rest is left out]"), "{text}");
        assert!(!text.contains("Rabatt"), "the attachment is cut: {text}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_email_is_described_with_its_attachments_read() {
        let email_id = "01a0e1cc-a24c-7490-82fe-e75669985b55";
        let attachment = |file: &str, name: &str, body: &str| {
            let id = Id::generate();
            let text = format!(
                "---\nid: {id}\ntitle: Rechnung\nlinked-to: {email_id}\ndocument:\n  original:\n    file: {file}\n    name: {name}\n---\n{body}"
            );
            (format!("Attachments/{file}.md"), text)
        };
        let files = [
            (
                "Email/a.md".to_string(),
                format!(
                    "---\nid: {email_id}\ntitle: Abschlag\nemail:\n  from: a@x\n---\nSee the bill.\n"
                ),
            ),
            attachment("r.pdf", "Rechnung.pdf", "Betrag: 40 EUR\n"),
            attachment("s.jpg", "Scan.jpg", ""),
        ];
        let h = crate::testing::Harness::with_vault("context-attachments", &files);
        let gate = Gate {
            local: false,
            default: &Classification::Public,
        };
        let text = describe(h.app.vault.index(), Id::parse(email_id).unwrap(), &gate)
            .unwrap()
            .text;
        assert!(
            text.contains(
                "See the bill.\n\n[Attached file \u{201C}Rechnung.pdf\u{201D} (\u{201C}Rechnung\u{201D}), as read:]\n\nBetrag: 40 EUR\n"
            ),
            "{text}"
        );
        assert!(
            !text.contains("Scan.jpg"),
            "one without text is left out: {text}"
        );
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
