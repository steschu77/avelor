//! What reaches a model (vault.md#metadata, assistant.md#context): an
//! entity's classification, and whether the model runs on this PC. The one
//! gate every request to a model asks: the chat's context, the analysis of
//! emails and documents, the reading of attachments and scans.
//!
//! A cloud model gets a `masked` entity's values as placeholders: its
//! frontmatter with each value replaced by a name in braces
//! (`{checking-account-iban}`); the conversation keeps what each names
//! (`Placeholder`, the entity and the path), never the value, and the reply
//! shows the vault's value in its place (`fill`).

use std::collections::BTreeMap;
use std::net::IpAddr;

use av_llm::Provider;
use av_utils::id::Id;
use av_vault::document::Document;
use av_vault::document::item_key;
use av_vault::document::item_position;
use av_vault::envelope::Placeholder;
use av_vault::shared::Classification;
use av_vault::vault::Index;
use serde_yaml_ng::Value;

use crate::texts::Text;

/// How much of an entity a model gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    Whole,
    /// Its frontmatter with placeholders for its values, no body.
    Placeholders,
    Nothing,
}

// ----------------------------------------------------------------------------
/// How much of an entity of `class` a model gets, `local` if it runs on
/// this PC: `open` all of it; `masked` all of it on this PC, else its
/// values as placeholders; `local` all of it on this PC only;
/// `sealed` (and a value the app doesn't know) nothing.
pub fn reach(class: &Classification, local: bool) -> Reach {
    match class.rank() {
        0 => Reach::Whole,
        3 => Reach::Nothing,
        _ if local => Reach::Whole,
        1 => Reach::Placeholders,
        _ => Reach::Nothing,
    }
}

// ----------------------------------------------------------------------------
/// The classification of `doc`: its own, else its kind's, else the
/// vault's `default` (`Envelope::classification`).
pub fn class_of(doc: &Document, default: &Classification) -> Classification {
    doc.envelope().map_or_else(
        || default.clone(),
        |envelope| envelope.classification(default),
    )
}

// ----------------------------------------------------------------------------
/// Whether a model, `local` or not, may read all of `doc` (what has no
/// placeholders: an analysis, a reading).
pub fn reads_whole(doc: &Document, default: &Classification, local: bool) -> bool {
    reach(&class_of(doc, default), local) == Reach::Whole
}

// ----------------------------------------------------------------------------
/// The classification `class` in words.
pub fn class_text(class: &Classification) -> Text {
    match class {
        Classification::Open => Text::new("class.open"),
        Classification::Masked => Text::new("class.masked"),
        Classification::Local => Text::new("class.local"),
        Classification::Sealed => Text::new("class.sealed"),
        Classification::Other(other) => Text::new("class.other").arg("name", other.as_str()),
    }
}

// ----------------------------------------------------------------------------
/// What a job tells of the entities its model wasn't given.
pub fn kept_from(n: usize) -> Text {
    Text::new("ai.kept_from").arg("n", n)
}

// ----------------------------------------------------------------------------
/// Whether the `model` of an account of `provider` at `address` runs on
/// this PC: an Ollama at a loopback address, but not one of its cloud tags
/// (`gpt-oss:120b-cloud`), which ollama.com runs.
pub fn is_local(provider: Provider, address: &str, model: &str) -> bool {
    provider == Provider::Ollama && is_loopback(address) && !is_cloud_tag(model)
}

fn is_loopback(address: &str) -> bool {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or_default();
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn is_cloud_tag(model: &str) -> bool {
    let tag = model.rsplit_once(':').map_or(model, |(_, tag)| tag);
    tag == "cloud" || tag.ends_with("-cloud")
}

// ----------------------------------------------------------------------------
/// The keys of a frontmatter that stay as they are under placeholders:
/// what it is and when, not its personal values.
const KEPT: [&str; 7] = [
    "title",
    "date",
    "tags",
    "projects",
    "classification",
    "kind",
    "type",
];

/// The most values one entity gives as placeholders; more, and it is left
/// out instead (a statement's transactions).
const MOST: usize = 60;

/// The placeholders given in one request, by name.
pub type Placeholders = BTreeMap<String, Placeholder>;

// ----------------------------------------------------------------------------
/// `frontmatter` (the entity `entity` named `title`, as a model sees it)
/// with its values as placeholders, each added to `given`; `None` if it has
/// more than `MOST` of them.
pub fn with_placeholders(
    frontmatter: &mut Value,
    entity: Id,
    title: &str,
    given: &mut Placeholders,
) -> Option<()> {
    let mut found = Vec::new();
    collect(frontmatter, &mut Vec::new(), &mut found);
    if found.len() > MOST {
        return None;
    }
    let stem = slug(title);
    for path in found {
        let key = path.last().map(String::as_str).unwrap_or("value");
        let base = format!("{stem}-{}", slug(key));
        let mut name = base.clone();
        let mut n = 2;
        while given
            .get(&name)
            .is_some_and(|p| p.entity != entity || p.path != path.join("."))
        {
            name = format!("{base}-{n}");
            n += 1;
        }
        *value_mut(frontmatter, &path)? = Value::String(format!("{{{name}}}"));
        given.insert(
            name,
            Placeholder {
                entity,
                path: path.join("."),
            },
        );
    }
    Some(())
}

/// The path of each scalar value under `value` but the kept keys at the
/// top and the ids (a list's item by its `id`, else its index).
fn collect(value: &Value, path: &mut Vec<String>, found: &mut Vec<Vec<String>>) {
    match value {
        Value::Mapping(map) => {
            for (key, inner) in map {
                let Some(key) = key.as_str() else { continue };
                // an item's id names it in a path, and is no personal value
                if key == "id" || path.is_empty() && KEPT.contains(&key) {
                    continue;
                }
                path.push(key.to_string());
                collect(inner, path, found);
                path.pop();
            }
        }
        Value::Sequence(items) => {
            for (i, item) in items.iter().enumerate() {
                path.push(item_key(item, i));
                collect(item, path, found);
                path.pop();
            }
        }
        Value::String(_) | Value::Number(_) => found.push(path.clone()),
        _ => {}
    }
}

/// The value at `path` in `value`.
fn value_mut<'a>(value: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    let mut at = value;
    for key in path {
        at = match at {
            Value::Mapping(map) => map.get_mut(key.as_str())?,
            Value::Sequence(items) => {
                let i = item_position(items, key)?;
                &mut items[i]
            }
            _ => return None,
        };
    }
    Some(at)
}

/// Lowercase letters and digits, words joined by dashes.
fn slug(text: &str) -> String {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        "value".to_string()
    } else {
        words.join("-")
    }
}

// ----------------------------------------------------------------------------
/// `text` with each placeholder of `placeholders` it names in braces
/// replaced by the vault's value now; one whose value is gone stays.
pub fn fill(text: &str, placeholders: &Placeholders, index: &Index) -> String {
    if placeholders.is_empty() || !text.contains('{') {
        return text.to_string();
    }
    let mut out = text.to_string();
    for (name, placeholder) in placeholders {
        let token = format!("{{{name}}}");
        if !out.contains(&token) {
            continue;
        }
        if let Some(value) = value_of(index, placeholder) {
            out = out.replace(&token, &value);
        }
    }
    out
}

/// The value `placeholder` names, as text.
fn value_of(index: &Index, placeholder: &Placeholder) -> Option<String> {
    let envelope = index.entry(&placeholder.entity)?.doc.envelope()?;
    let mut value = serde_yaml_ng::to_value(envelope).ok()?;
    let path: Vec<String> = placeholder.path.split('.').map(str::to_string).collect();
    match value_mut(&mut value, &path)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_classification_reaches_as_far_as_it_says() {
        let all = [
            Classification::Open,
            Classification::Masked,
            Classification::Local,
            Classification::Sealed,
            Classification::Other("secret".into()),
        ];
        let local: Vec<Reach> = all.iter().map(|c| reach(c, true)).collect();
        let cloud: Vec<Reach> = all.iter().map(|c| reach(c, false)).collect();
        use Reach::*;
        assert_eq!(local, [Whole, Whole, Whole, Nothing, Nothing]);
        assert_eq!(cloud, [Whole, Placeholders, Nothing, Nothing, Nothing]);
    }

    #[test]
    fn a_model_is_local_on_a_loopback_ollama_but_never_a_cloud_tag() {
        let ollama = Provider::Ollama;
        assert!(is_local(ollama, "http://localhost:11434", "qwen3:8b"));
        assert!(is_local(ollama, "http://127.0.0.1:11434/", "llama3"));
        assert!(is_local(ollama, "http://[::1]:11434", "llama3"));
        assert!(!is_local(
            ollama,
            "http://localhost:11434",
            "gpt-oss:120b-cloud"
        ));
        assert!(!is_local(ollama, "http://localhost:11434", "kimi-k2:cloud"));
        assert!(!is_local(ollama, "http://192.168.1.20:11434", "llama3"));
        assert!(!is_local(ollama, "https://ollama.com", "llama3"));
        assert!(!is_local(
            Provider::OpenAi,
            "http://localhost:8080",
            "gpt-5"
        ));
    }

    #[test]
    fn values_become_placeholders_and_the_title_stays() {
        let entity = Id::generate();
        let mut fm: Value = serde_yaml_ng::from_str(
            "title: Checking\ndate: 2026-01-01\nbank-account:\n  iban: DE89 3704\n  bank: Volksbank\n  open: true\n",
        )
        .unwrap();
        let mut given = Placeholders::new();
        with_placeholders(&mut fm, entity, "Checking", &mut given).unwrap();
        let text = serde_yaml_ng::to_string(&fm).unwrap();
        assert!(text.contains("title: Checking"), "{text}");
        assert!(text.contains("iban: '{checking-iban}'"), "{text}");
        assert!(text.contains("bank: '{checking-bank}'"), "{text}");
        assert!(text.contains("open: true"), "{text}");
        assert_eq!(
            given["checking-iban"],
            Placeholder {
                entity,
                path: "bank-account.iban".into()
            }
        );
    }

    #[test]
    fn a_list_item_is_named_by_its_id_and_too_many_values_are_none() {
        let entity = Id::generate();
        let mut fm: Value =
            serde_yaml_ng::from_str("items:\n- id: a1\n  number: '1'\n- number: '2'\n").unwrap();
        let mut given = Placeholders::new();
        with_placeholders(&mut fm, entity, "List", &mut given).unwrap();
        assert_eq!(given["list-number"].path, "items.a1.number");
        assert_eq!(given["list-number-2"].path, "items.1.number");
        let many = (0..=MOST)
            .map(|i| format!("k{i}: v"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut fm: Value = serde_yaml_ng::from_str(&many).unwrap();
        assert!(with_placeholders(&mut fm, entity, "Many", &mut Placeholders::new()).is_none());
    }
}
