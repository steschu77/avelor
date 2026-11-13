//! The shipped blueprints (`vault.md#project`): YAML in the app
//! (`app/blueprints`), their words keys of the catalogs, handed to the page
//! as JSON in its data block (`page::html`) in the run's language. A project names the one it came from and keeps in its
//! file what it took (its checks, its labels), so a release changes no
//! project.

use serde::{Deserialize, Serialize};

use crate::texts;

/// The shipped blueprints' files, by the name a project's `blueprint` gives.
const SHIPPED: [(&str, &str); 1] = [("travel", include_str!("../blueprints/travel.yaml"))];

// ----------------------------------------------------------------------------
/// A shipped blueprint: what New project offers, the facts a project of it
/// has, what it checks, its labels.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blueprint {
    #[serde(default)]
    pub name: String,
    /// Its name, a key of the catalogs, as all its words are.
    pub title: String,
    /// What New project says beside the title.
    pub note: String,
    /// The name a new project of it starts with ("New trip").
    pub untitled: String,
    /// The page's icon (`#i-…`); `build.rs` checks it is there.
    pub icon: String,
    #[serde(default)]
    pub facts: Vec<Fact>,
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

// ----------------------------------------------------------------------------
/// A fact a blueprint names: its key in `project.facts`, its label in the
/// header, and what it is, which picks its editor.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fact {
    pub name: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: FactType,
    /// A `choice`'s values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FactType {
    Text,
    Date,
    /// A trip's days: from and to.
    Days,
    Amount,
    Contacts,
    Calendar,
    Choice,
}

// ----------------------------------------------------------------------------
/// The shipped blueprints, in the order New project offers them. One that
/// doesn't parse is left out and logged; the tests read every one.
pub fn shipped() -> Vec<Blueprint> {
    SHIPPED
        .into_iter()
        .filter_map(
            |(name, yaml)| match serde_yaml_ng::from_str::<Blueprint>(yaml) {
                Ok(blueprint) => Some(Blueprint {
                    name: name.to_string(),
                    ..blueprint
                }),
                Err(err) => {
                    log::error!("The blueprint {name} can't be read: {err}");
                    None
                }
            },
        )
        .collect()
}

// ----------------------------------------------------------------------------
/// The shipped blueprints as the page's data block holds them, their words
/// in the run's language.
pub fn json() -> String {
    let worded: Vec<Blueprint> = shipped()
        .into_iter()
        .map(|blueprint| Blueprint {
            title: texts::of(&blueprint.title),
            note: texts::of(&blueprint.note),
            untitled: texts::of(&blueprint.untitled),
            facts: blueprint
                .facts
                .into_iter()
                .map(|fact| Fact {
                    label: texts::of(&fact.label),
                    ..fact
                })
                .collect(),
            ..blueprint
        })
        .collect();
    serde_json::to_string(&worded).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn every_shipped_blueprint_reads() {
        for (name, yaml) in SHIPPED {
            let blueprint: Blueprint = serde_yaml_ng::from_str(yaml)
                .unwrap_or_else(|err| panic!("blueprints/{name}.yaml: {err}"));
            assert!(blueprint.icon.starts_with("#i-"), "{name}");
            assert!(!blueprint.facts.is_empty(), "{name}");
        }
        assert_eq!(shipped().len(), SHIPPED.len());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_trip_names_its_facts() {
        let trip = shipped().into_iter().find(|b| b.name == "travel").unwrap();
        let facts: Vec<_> = trip
            .facts
            .iter()
            .map(|f| (f.name.as_str(), f.kind))
            .collect();
        assert_eq!(
            facts,
            [
                ("days", FactType::Days),
                ("home", FactType::Text),
                ("travelers", FactType::Contacts),
                ("calendar", FactType::Calendar),
                ("budget", FactType::Amount),
            ]
        );
        assert_eq!(
            trip.checks,
            ["budget", "deadlines", "nights", "journeys", "tickets"]
        );
        assert_eq!(trip.labels, ["travel"]);
        let json: serde_json::Value = serde_json::from_str(&json()).unwrap();
        assert_eq!(json[0]["name"], "travel");
        assert_eq!(json[0]["facts"][0]["type"], "days");
    }
}
