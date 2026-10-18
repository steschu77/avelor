//! The app's texts (`localization.md`): a catalog read and checked against
//! English, a language's texts over English's, and the keys the page and the
//! host use. The app's build is its user (`app/build.rs`); nothing of it is
//! in the exe.

use oxc::{
    allocator::Allocator,
    ast::ast::{Argument, CallExpression, Expression},
    ast_visit::{Visit, walk},
    parser::Parser,
    span::SourceType,
};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The categories of `Intl.PluralRules` a count's forms are keyed by.
const CATEGORIES: &[&str] = &["zero", "one", "two", "few", "many", "other"];

/// The page's functions whose first argument is a key (`core/text.js`).
const PAGE_CALLS: &[&str] = &["t", "parts"];

/// The host's call for a text the page shows (`texts.rs`, `Text`).
const HOST_PAGE_CALL: &str = "Text::new(\"";

/// The host's lookups of a text it shows or writes itself (`texts.rs`).
const HOST_CALLS: &[&str] = &["texts::text(\"", "texts::filled(\""];

// ----------------------------------------------------------------------------
/// A catalog's text: plain, or a count's forms by their category.
#[derive(Debug, Clone, PartialEq)]
pub enum Text {
    Plain(String),
    Forms(BTreeMap<String, String>),
}

// ----------------------------------------------------------------------------
/// A language's catalog: its texts by key.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub language: String,
    pub texts: BTreeMap<String, Text>,
}

// ----------------------------------------------------------------------------
/// What the sources use: the keys the page shows, and those the host shows
/// or writes itself, which are plain texts.
#[derive(Debug, Default)]
pub struct Used {
    pub page: BTreeSet<String>,
    pub host: BTreeSet<String>,
}

// ----------------------------------------------------------------------------
impl Catalog {
    // ------------------------------------------------------------------------
    /// The catalog of `language` in `json`: an object of texts, each a string
    /// or a count's forms with `other`. Anything else is an error that names
    /// the key.
    pub fn parse(language: &str, json: &str) -> Result<Catalog, String> {
        let value: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let Value::Object(entries) = value else {
            return Err("not an object of texts".to_string());
        };
        let mut texts = BTreeMap::new();
        for (key, value) in entries {
            let text = match value {
                Value::String(text) => Text::Plain(text),
                Value::Object(forms) => Text::Forms(forms_of(&key, forms)?),
                _ => return Err(format!("\"{key}\": neither a text nor a count's forms")),
            };
            texts.insert(key, text);
        }
        Ok(Catalog {
            language: language.to_string(),
            texts,
        })
    }

    // ------------------------------------------------------------------------
    /// The catalog as the page reads it (`core/text.js`): one JSON object.
    pub fn to_json(&self) -> String {
        let texts: Map<String, Value> = self
            .texts
            .iter()
            .map(|(key, text)| {
                let value = match text {
                    Text::Plain(text) => Value::String(text.clone()),
                    Text::Forms(forms) => Value::Object(
                        forms
                            .iter()
                            .map(|(c, text)| (c.clone(), Value::String(text.clone())))
                            .collect(),
                    ),
                };
                (key.clone(), value)
            })
            .collect();
        Value::Object(texts).to_string()
    }

    // ------------------------------------------------------------------------
    /// The catalog with English's texts where it has none: what a language
    /// that came from outside shows.
    pub fn over(&self, english: &Catalog) -> Catalog {
        let mut texts = english.texts.clone();
        texts.extend(self.texts.clone());
        Catalog {
            language: self.language.clone(),
            texts,
        }
    }

    // ------------------------------------------------------------------------
    /// The plain text of `key`; none for a count's forms.
    pub fn plain(&self, key: &str) -> Option<&str> {
        match self.texts.get(key)? {
            Text::Plain(text) => Some(text),
            Text::Forms(_) => None,
        }
    }
}

// ----------------------------------------------------------------------------
/// A count's forms: each a category of `Intl.PluralRules` with its text,
/// `other` among them.
fn forms_of(key: &str, forms: Map<String, Value>) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (category, text) in forms {
        if !CATEGORIES.contains(&category.as_str()) {
            return Err(format!("\"{key}\": \"{category}\" isn't a plural category"));
        }
        let Value::String(text) = text else {
            return Err(format!("\"{key}\": its form \"{category}\" isn't a text"));
        };
        out.insert(category, text);
    }
    if !out.contains_key("other") {
        return Err(format!("\"{key}\": its forms have no \"other\""));
    }
    Ok(out)
}

// ----------------------------------------------------------------------------
/// The placeholders, `{name}`, of `text` in all its forms.
fn placeholders(text: &Text) -> BTreeSet<String> {
    let texts: Vec<&String> = match text {
        Text::Plain(text) => vec![text],
        Text::Forms(forms) => forms.values().collect(),
    };
    let mut names = BTreeSet::new();
    for text in texts {
        let mut rest = text.as_str();
        while let Some((_, after)) = rest.split_once('{') {
            match after.split_once('}') {
                Some((name, tail)) if is_name(name) => {
                    names.insert(name.to_string());
                    rest = tail;
                }
                _ => rest = after,
            }
        }
    }
    names
}

fn is_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ----------------------------------------------------------------------------
/// The keys the page's script `js` uses: the first argument of each `t`
/// and `parts`.
/// A key that isn't written out is an error, since nothing could check it.
pub fn page_keys(js: &str) -> Result<BTreeSet<String>, Vec<String>> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, js, SourceType::script()).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(vec!["the page's script doesn't parse".to_string()]);
    }
    let mut calls = KeyCalls::default();
    calls.visit_program(&parsed.program);
    if calls.problems.is_empty() {
        Ok(calls.keys)
    } else {
        Err(calls.problems)
    }
}

#[derive(Default)]
struct KeyCalls {
    keys: BTreeSet<String>,
    problems: Vec<String>,
}

impl<'a> Visit<'a> for KeyCalls {
    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Expression::Identifier(callee) = &it.callee {
            if PAGE_CALLS.contains(&callee.name.as_str()) {
                let key = match it.arguments.first() {
                    Some(Argument::StringLiteral(key)) => Some(key.value.as_str()),
                    _ => None,
                };
                match key {
                    Some(key) => {
                        self.keys.insert(key.to_string());
                    }
                    None => self.problems.push(format!(
                        "{}(...) at {}: its key isn't written out",
                        callee.name, it.span.start
                    )),
                }
            }
        }
        walk::walk_call_expression(self, it);
    }
}

// ----------------------------------------------------------------------------
/// The keys the host source `rust` uses, outside the line comments: those
/// of the texts it gives the page (`Text::new`), and those it shows or
/// writes itself (`texts::text`, `texts::filled`).
pub fn host_keys(rust: &str) -> Used {
    let mut used = Used::default();
    for line in rust.lines() {
        let code = line.split_once("//").map_or(line, |(code, _)| code);
        used.page.extend(keys_after(code, HOST_PAGE_CALL));
        for call in HOST_CALLS {
            used.host.extend(keys_after(code, call));
        }
    }
    used
}

/// The keys written out after each `call` in `code`.
fn keys_after<'a>(code: &'a str, call: &'a str) -> impl Iterator<Item = String> + 'a {
    code.match_indices(call).filter_map(move |(at, _)| {
        let rest = &code[at + call.len()..];
        rest.split_once('"').map(|(key, _)| key.to_string())
    })
}

// ----------------------------------------------------------------------------
/// The keys the page's own markup (`index.html`) names: `data-t="key"`, its
/// element's text, which the element doesn't have in the markup, and
/// `data-t-<attribute>="key"`, that attribute's. An element with a text
/// in the markup besides its key is an error.
pub fn html_keys(html: &str) -> Result<BTreeSet<String>, Vec<String>> {
    let mut keys = BTreeSet::new();
    let mut problems = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(" data-t") {
        let after = &rest[at + " data-t".len()..];
        let (text, after) = if let Some(after) = after.strip_prefix("=\"") {
            (true, after)
        } else if let Some(after) = after.strip_prefix('-') {
            match after.split_once("=\"") {
                Some((_, after)) => (false, after),
                None => (false, after),
            }
        } else {
            rest = after;
            continue;
        };
        let Some((key, after)) = after.split_once('"') else {
            problems.push("index.html: a data-t without its key's end".to_string());
            break;
        };
        if text && !after.starts_with("></") {
            problems.push(format!(
                "index.html: the element of \"{key}\" has more than its text"
            ));
        }
        keys.insert(key.to_string());
        rest = after;
    }
    if problems.is_empty() {
        Ok(keys)
    } else {
        Err(problems)
    }
}

// ----------------------------------------------------------------------------
/// The catalogs' problems, each naming the catalog's file and the key:
/// English against what `used` names (a key used that it lacks, one used
/// nowhere, a count's forms where the host needs a plain text); each of
/// `others` against English (a key English lacks, a placeholder English's
/// text doesn't have, a count's forms for a host's text); and a key English
/// has that one of the app's `own` lacks.
pub fn check(english: &Catalog, others: &[Catalog], own: &[&str], used: &Used) -> Vec<String> {
    let mut problems = Vec::new();
    let in_use: BTreeSet<&String> = used.page.iter().chain(&used.host).collect();
    for key in &in_use {
        if !english.texts.contains_key(*key) {
            problems.push(format!("en.json: no \"{key}\", which is used"));
        }
    }
    for key in english.texts.keys() {
        if !in_use.contains(key) {
            problems.push(format!("en.json: \"{key}\" is used nowhere"));
        }
    }
    for catalog in std::iter::once(english).chain(others) {
        let file = format!("{}.json", catalog.language);
        for key in &used.host {
            if let Some(Text::Forms(_)) = catalog.texts.get(key) {
                problems.push(format!(
                    "{file}: \"{key}\" has a count's forms, but the host shows it as it is"
                ));
            }
        }
    }
    for catalog in others {
        let file = format!("{}.json", catalog.language);
        for (key, text) in &catalog.texts {
            let Some(theirs) = english.texts.get(key) else {
                problems.push(format!("{file}: \"{key}\" isn't in en.json"));
                continue;
            };
            let extra: Vec<String> = placeholders(text)
                .difference(&placeholders(theirs))
                .map(|name| format!("{{{name}}}"))
                .collect();
            if !extra.is_empty() {
                problems.push(format!(
                    "{file}: \"{key}\" has {}, which en.json's hasn't",
                    extra.join(", ")
                ));
            }
        }
        if own.contains(&catalog.language.as_str()) {
            for key in english.texts.keys() {
                if !catalog.texts.contains_key(key) {
                    problems.push(format!("{file}: no \"{key}\", which en.json has"));
                }
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests;
