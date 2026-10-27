//! The search (`vault.md#search`): an entity's words as a search reads
//! them, kept with its file's text in the catalog, and `search`, which
//! finds the entities holding every word of a text, each with where the
//! words are when its title lacks some.

use std::collections::HashMap;

use av_store::catalog::Words;
use av_utils::cancel::Cancel;
use av_utils::error::Error;
use av_utils::id::Id;
use av_vault::document::Document;
use serde::Serialize;

use crate::app::{App, catalog_key};
use crate::catalog::{CatalogState, Unavailable};
use crate::protocol::Req;

/// The page's channel of the search: a newer search supersedes the one
/// running.
const CHANNEL: &str = "search";

/// An excerpt starts at a word at most this many characters before the
/// first word the title lacks ...
const BEFORE: usize = 40;
/// ... and has at most this many characters.
const LENGTH: usize = 200;

// ----------------------------------------------------------------------------
/// Where the words are, for an entity whose title lacks some of them: the
/// first line of its text holding one, cut around it, or else such a label
/// (`place`).
#[derive(Debug, PartialEq)]
struct Excerpt {
    text: String,
    place: Option<&'static str>,
}

// ----------------------------------------------------------------------------
/// An entity holding every word, as the page gets it.
#[derive(Serialize)]
struct Hit {
    id: Id,
    #[serde(skip_serializing_if = "Option::is_none")]
    excerpt: Option<String>,
    #[serde(rename = "where", skip_serializing_if = "Option::is_none")]
    place: Option<&'static str>,
}

impl Hit {
    fn new(id: Id, excerpt: Option<Excerpt>) -> Hit {
        let (excerpt, place) = excerpt.map_or((None, None), |e| (Some(e.text), e.place));
        Hit { id, excerpt, place }
    }
}

// ----------------------------------------------------------------------------
/// `search`'s answer: the words as it took them, and what holds them.
#[derive(Serialize)]
struct Found {
    terms: Vec<String>,
    hits: Vec<Hit>,
}

// ----------------------------------------------------------------------------
/// `doc`'s words: its title, its labels a line each, and its text
/// (`Document::plain_text`). A change to them is a new format of the
/// catalog (`av_store::catalog::FORMAT`).
pub fn words_of(doc: &Document) -> Words {
    Words {
        title: doc.title().to_string(),
        labels: doc.tags().join("\n"),
        body: doc.plain_text(),
    }
}

// ----------------------------------------------------------------------------
/// The rows holding every word, by path, each with where the words are; or
/// why the catalog couldn't be read, as the error and the page's words.
type Rows = std::result::Result<HashMap<String, Option<Excerpt>>, (Error, &'static str)>;

// ----------------------------------------------------------------------------
/// `search {text}`: the entities holding every word of `text`, in any
/// order and case, also inside a longer word, as `{terms, hits: [{id,
/// excerpt?, where?}]}` in no order. The catalog's rows are read on its
/// thread, after every change asked before. A newer search supersedes the
/// one running, which answers `cancelled`.
pub fn search(app: &mut App, req: Req, text: String) {
    let terms = terms_of(&text);
    if terms.is_empty() {
        let arg = "no words to search for".to_string();
        return req.err(Error::InvalidArgument { arg });
    }
    if app.vault.root().is_none() {
        return req.fail(Error::NotInitialized, "No vault is open.");
    }
    let work = move |state: &CatalogState, cancel: &Cancel| {
        let rows = rows_holding(state, &terms, cancel);
        (terms, rows)
    };
    let op = Id::generate().to_string();
    app.run_superseding_on_catalog(CHANNEL, &op, work, move |app, (terms, rows)| {
        let mut rows = match rows {
            Ok(rows) => rows,
            Err((err, why)) => return req.fail(err, format!("Search isn't available: {why}.")),
        };
        // A row stands for the entry at its path now.
        let hits = app
            .vault
            .index()
            .all()
            .iter()
            .filter_map(|entry| {
                let found = rows.remove(&catalog_key(&entry.local)?)?;
                Some(Hit::new(entry.doc.id(), found))
            })
            .collect();
        req.ok(Found { terms, hits });
    });
}

// ----------------------------------------------------------------------------
/// The catalog's rows holding every one of `terms`, until `cancel` is set.
fn rows_holding(state: &CatalogState, terms: &[String], cancel: &Cancel) -> Rows {
    let unsupported = || Error::Unsupported {
        what: "a search without the catalog".to_string(),
    };
    let catalog = match state.current() {
        Ok(catalog) => catalog,
        Err(Unavailable::Missing) => return Err((unsupported(), "the catalog couldn't be opened")),
        Err(Unavailable::Behind) => {
            return Err((unsupported(), "the catalog is behind; open the vault again"));
        }
    };
    let mut rows = HashMap::new();
    let read = catalog.search(terms, |local, words| {
        if holds(&words, terms) {
            rows.insert(local, excerpt(&words, terms));
        }
        !cancel.is_cancelled()
    });
    match read {
        Ok(()) => Ok(rows),
        Err(err) => {
            log::warn!("Search: the catalog can't be read: {err}");
            Err((err, "the catalog can't be read"))
        }
    }
}

// ----------------------------------------------------------------------------
/// The words of `text` a search looks for: in lower case, each once.
fn terms_of(text: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        let term = word.to_lowercase();
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    terms
}

// ----------------------------------------------------------------------------
/// Whether `words` hold every one of `terms`, each in the title, a label or
/// the text, in any case, also inside a longer word: the search's one rule.
/// The catalog's trigrams only narrow the rows.
fn holds(words: &Words, terms: &[String]) -> bool {
    let fields = [&words.title, &words.labels, &words.body].map(|field| field.to_lowercase());
    terms
        .iter()
        .all(|term| fields.iter().any(|field| field.contains(term.as_str())))
}

// ----------------------------------------------------------------------------
/// Where `terms` are in `words`, if the title lacks some of them.
fn excerpt(words: &Words, terms: &[String]) -> Option<Excerpt> {
    let title = words.title.to_lowercase();
    let missing: Vec<&str> = terms
        .iter()
        .map(String::as_str)
        .filter(|term| !title.contains(term))
        .collect();
    if missing.is_empty() {
        return None;
    }
    if let Some(text) = line_with(&words.body, &missing) {
        return Some(Excerpt { text, place: None });
    }
    let text = line_with(&words.labels, &missing)?;
    Some(Excerpt {
        text,
        place: Some("Label"),
    })
}

// ----------------------------------------------------------------------------
/// The first line of `text` holding one of `terms`, cut around the first
/// of them in it.
fn line_with(text: &str, terms: &[&str]) -> Option<String> {
    text.lines().find_map(|line| {
        let at = terms.iter().filter_map(|term| char_at(line, term)).min()?;
        Some(cut(line, at))
    })
}

// ----------------------------------------------------------------------------
/// Where `term` (lower case) is in `line`, counted in `line`'s characters:
/// a character may lower to several.
fn char_at(line: &str, term: &str) -> Option<usize> {
    let mut lower = String::new();
    // The character of `line` each byte of `lower` comes from.
    let mut of = Vec::new();
    for (i, c) in line.chars().enumerate() {
        for l in c.to_lowercase() {
            of.extend(std::iter::repeat_n(i, l.len_utf8()));
            lower.push(l);
        }
    }
    lower.find(term).map(|byte| of[byte])
}

// ----------------------------------------------------------------------------
/// `line` from a word at most `BEFORE` characters before the character
/// `at`, at most `LENGTH` long, with an ellipsis where it was cut.
fn cut(line: &str, at: usize) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut start = at.saturating_sub(BEFORE);
    if start > 0
        && let Some(space) = chars[start..at].iter().position(|c| c.is_whitespace())
    {
        start += space + 1;
    }
    let end = chars.len().min(start + LENGTH);
    let mut text = String::new();
    if start > 0 {
        text.push('\u{2026}');
    }
    text.extend(&chars[start..end]);
    if end < chars.len() {
        text.push('\u{2026}');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, parse};
    use serde_json::{Value, json};
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    // ------------------------------------------------------------------------
    fn words(title: &str, labels: &str, body: &str) -> Words {
        Words {
            title: title.to_string(),
            labels: labels.to_string(),
            body: body.to_string(),
        }
    }

    // ------------------------------------------------------------------------
    fn terms(text: &str) -> Vec<String> {
        terms_of(text)
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_terms_are_the_words_in_lower_case_each_once() {
        assert_eq!(terms(" Rechnung  rechnung\tHaus "), ["rechnung", "haus"]);
        assert!(terms("  ").is_empty());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn every_term_must_be_somewhere_in_any_case_and_inside_words() {
        let w = words("Stromrechnung", "Haus", "Bitte bis Freitag zahlen");
        assert!(holds(&w, &terms("RECHNUNG haus")));
        assert!(holds(&w, &terms("freitag strom om")));
        assert!(!holds(&w, &terms("rechnung montag")));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_excerpt_is_where_the_title_lacks_a_word() {
        let w = words(
            "Stromrechnung",
            "tax\nreceipt",
            "First line\nBitte bis Freitag zahlen",
        );
        assert_eq!(excerpt(&w, &terms("rechnung")), None);
        let line = Excerpt {
            text: "Bitte bis Freitag zahlen".to_string(),
            place: None,
        };
        assert_eq!(excerpt(&w, &terms("rechnung freitag")), Some(line));
        let label = Excerpt {
            text: "receipt".to_string(),
            place: Some("Label"),
        };
        assert_eq!(excerpt(&w, &terms("receipt")), Some(label));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_long_line_is_cut_from_a_word_before_the_term() {
        let line = format!("{}needle {}", "ab ".repeat(30), "cd ".repeat(100));
        let text = excerpt(&words("", "", &line), &terms("needle"))
            .unwrap()
            .text;
        assert!(text.starts_with("\u{2026}ab ab"), "{text}");
        assert!(text.ends_with('\u{2026}'), "{text}");
        assert!(text.contains("needle"));
        let kept = text.chars().count() - 2;
        assert!(kept <= LENGTH, "{kept}");
        let before = text.find("needle").unwrap() - '\u{2026}'.len_utf8();
        assert!(before <= BEFORE, "{before}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_letter_that_lowers_to_two_doesnt_shift_the_cut() {
        let line = format!("{}zmir and more", "\u{130}".repeat(60));
        let text = excerpt(&words("", "", &line), &terms("zmir")).unwrap().text;
        assert!(text.contains("zmir and more"), "{text}");
    }

    // ------------------------------------------------------------------------
    /// Write the note `title` at `local` in `root` with `body`: settled (an
    /// hour old) so the scan keeps it, or just written.
    fn note(root: &Path, local: &str, title: &str, body: &str, settled: bool) -> Id {
        let (id, path) = (Id::generate(), root.join(local));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = format!("---\nid: {id}\ntitle: {title}\ndocument: {{}}\n---\n{body}\n");
        fs::write(&path, text).unwrap();
        if settled {
            let file = fs::File::options().write(true).open(&path).unwrap();
            let then = SystemTime::now() - Duration::from_secs(3600);
            file.set_modified(then).unwrap();
        }
        id
    }

    // ------------------------------------------------------------------------
    fn open(h: &mut Harness) {
        let root = h.vault();
        crate::api::switch_to(&mut h.app, root);
        h.wait_scanned();
        h.take();
    }

    // ------------------------------------------------------------------------
    /// The reply to a search for `text`.
    fn reply(h: &mut Harness, text: &str) -> Value {
        let sent = h.call_waiting("search", json!({ "text": text }));
        sent.into_iter().find(|m| m["re"] == 1).unwrap()
    }

    // ------------------------------------------------------------------------
    /// The ids a search for `text` finds, sorted, each with its excerpt.
    fn found(h: &mut Harness, text: &str) -> Vec<(String, Value)> {
        let reply = reply(h, text);
        let mut hits: Vec<(String, Value)> = reply["ok"]["hits"]
            .as_array()
            .unwrap_or_else(|| panic!("{reply}"))
            .iter()
            .map(|hit| {
                (
                    hit["id"].as_str().unwrap().to_string(),
                    hit["excerpt"].clone(),
                )
            })
            .collect();
        hits.sort_by(|a, b| a.0.cmp(&b.0));
        hits
    }

    // ------------------------------------------------------------------------
    fn ids(h: &mut Harness, text: &str) -> Vec<String> {
        found(h, text).into_iter().map(|(id, _)| id).collect()
    }

    // ------------------------------------------------------------------------
    fn sorted(ids: &[Id]) -> Vec<String> {
        let mut ids: Vec<String> = ids.iter().map(Id::to_string).collect();
        ids.sort();
        ids
    }

    // ------------------------------------------------------------------------
    #[test]
    fn words_in_the_text_are_found_with_where_they_are() {
        let mut h = Harness::new("search-catalog");
        let root = h.vault();
        let car = note(
            &root,
            "Notes/car.md",
            "Car",
            "The tyres were changed.",
            true,
        );
        let bill = note(&root, "Notes/bill.md", "Stromrechnung", "Paid.", true);
        open(&mut h);

        let reply = reply(&mut h, "Tyres");
        assert_eq!(reply["ok"]["terms"], json!(["tyres"]));
        let hits = &reply["ok"]["hits"];
        let hit = json!({ "id": car.to_string(), "excerpt": "The tyres were changed." });
        assert_eq!(hits, &json!([hit]));
        assert_eq!(ids(&mut h, "rechnung"), [bill.to_string()]);
        assert_eq!(ids(&mut h, "ch"), sorted(&[car, bill]), "a short word");
        assert!(ids(&mut h, "tyres paid").is_empty());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_edit_is_found_by_its_new_words_at_once() {
        let mut h = Harness::new("search-edit");
        let id = note(&h.vault(), "Notes/n.md", "Draft", "old words", true);
        open(&mut h);

        let save = json!({ "id": id, "markdown": "new words" });
        h.call("save_doc_content", save);
        assert_eq!(ids(&mut h, "new"), [id.to_string()]);
        assert!(ids(&mut h, "old").is_empty());

        h.call("set_title", json!({ "id": id, "title": "Zebra" }));
        assert_eq!(ids(&mut h, "zebra new"), [id.to_string()]);
        assert!(ids(&mut h, "draft").is_empty());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_file_read_before_it_settled_is_found() {
        let mut h = Harness::new("search-unsettled");
        let id = note(&h.vault(), "Notes/n.md", "Fresh", "just written", false);
        open(&mut h);
        assert_eq!(ids(&mut h, "written"), [id.to_string()]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_deleted_entity_isnt_found() {
        let mut h = Harness::new("search-deleted");
        let id = note(&h.vault(), "Notes/n.md", "Gone", "soon", true);
        open(&mut h);
        h.call("delete_entity", json!({ "id": id }));
        assert!(ids(&mut h, "gone").is_empty());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn without_a_catalog_a_search_fails() {
        let mut h = Harness::new("search-no-catalog");
        fs::write(h.dir().join("file"), "x").unwrap();
        h.app
            .vault
            .set_catalog_dir(h.dir().join("file").join("catalog"));
        note(&h.vault(), "Notes/n.md", "Kept", "somewhere", true);
        open(&mut h);
        let err = &reply(&mut h, "somewhere")["err"];
        assert_eq!(err["code"], "unsupported");
        assert_eq!(
            err["message"],
            "Search isn't available: the catalog couldn't be opened."
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_newer_search_supersedes_the_one_running() {
        let mut h = Harness::new("search-superseded");
        note(&h.vault(), "Notes/n.md", "Kept", "words", true);
        open(&mut h);
        for (id, text) in [(1, "word"), (2, "words")] {
            let request = json!({ "id": id, "cmd": "search", "args": { "text": text } });
            h.app.on_web_message(request.to_string());
        }
        // The first answers once the catalog's thread comes to it.
        let reply = |sent: &[Value], id: u64| sent.iter().find(|m| m["re"] == id).cloned();
        let both = |sent: &[String]| [1, 2].iter().all(|id| reply(&parse(sent), *id).is_some());
        let sent = parse(&h.wait_for(both));
        assert_eq!(reply(&sent, 1).unwrap()["err"]["code"], "cancelled");
        assert_eq!(reply(&sent, 2).unwrap()["ok"]["terms"], json!(["words"]));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_search_needs_words_and_a_vault() {
        let mut h = Harness::new("search-refused");
        assert_eq!(reply(&mut h, "x")["err"]["code"], "not_initialized");
        assert_eq!(reply(&mut h, " ")["err"]["code"], "invalid_argument");
    }
}
