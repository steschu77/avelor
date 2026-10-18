//! Analysing entities with an AI (email.md#analysis): the email analysis -- Lite
//! gives each new email its kind, a clean title, labels in use and active
//! projects; Medium also points to check, a quick reply and questions, as a
//! conversation linked to the email.
//!
//! An analysis goes plan → run → apply (architecture.md#conventions), one email
//! after another, as a job: the plan picks the model (`AiAccounts::choose`),
//! gathers the vault's labels and projects and each email's text; the run asks
//! the model; the apply reads its answer and writes it through the vault edit
//! (`chat::create` for Medium's conversation). The spend limit is checked before each
//! email.

use std::collections::HashMap;

use crate::texts::Text;
use av_utils::id::Id;
use job::Run;

mod answer;
mod apply;
mod context;
mod job;
mod request;

pub(crate) use job::{analyse, stop};

/// What the email analysis is called in a refused pick's words
/// (`AiAccounts::choose`).
pub fn email_analysis() -> Text {
    Text::new("feature.email_analysis")
}

// ----------------------------------------------------------------------------
/// Whether two names are the same in any case, around spaces.
fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

// ----------------------------------------------------------------------------
/// The analyses running (`App::analysis`), by job.
#[derive(Default)]
pub struct AnalysisState {
    runs: HashMap<Id, Run>,
}

#[cfg(test)]
mod tests;
