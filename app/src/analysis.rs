//! Analysing entities with an AI (email.md#analysis): the email analysis -- Lite
//! gives each new email its kind, a clean title, labels in use and active
//! projects; Medium also points to check, a quick reply and questions, as a
//! conversation linked to the email. The attachments read go along with their email,
//! and are analysed after the emails as Lite does an email: a clean title,
//! their own date, a genre, labels and projects. A watch's analysis takes
//! the scans it imported the same way (import.md#import-scans), at Medium
//! with points to check and questions as a conversation linked to the document.
//!
//! An analysis goes plan → run → apply (architecture.md#conventions), one email
//! after another, as a job: the plan picks the model (`AiAccounts::choose`),
//! gathers the vault's labels and projects and each email's text; the run asks
//! the model; the apply reads its answer and writes it through the vault edit
//! (`chat::create` for Medium's conversation). The spend limit is checked before each
//! email.

use std::collections::HashMap;

use av_utils::datetime::DateTime;
use av_utils::id::Id;
use serde::{Deserialize, Serialize};

use crate::ai::accounts::{AiAccounts, ModelChoice, Refusal};
use crate::texts::Text;
use crate::watches::WatchKind;
use job::Run;

mod answer;
mod apply;
mod context;
mod job;
mod request;

pub(crate) use job::{Batch, analyse, stop};

/// What the email analysis is called in a refused pick's words
/// (`AiAccounts::choose`).
pub fn email_analysis() -> Text {
    Text::new("feature.email_analysis")
}

// ----------------------------------------------------------------------------
/// How closely what comes in is analysed (email.md#analysis): Lite sorts
/// it, Medium also drafts what to check, an email's reply too, as a conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnalysisLevel {
    Off,
    Lite,
    Medium,
}

// ----------------------------------------------------------------------------
/// An email filter's or a watch's analysis: its level, and the model it runs
/// with (accounts.md#model-choice), kept while the level is off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    pub level: AnalysisLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelChoice>,
}

impl Analysis {
    // ------------------------------------------------------------------------
    /// Whether anything is analysed at all.
    pub fn is_on(&self) -> bool {
        self.level != AnalysisLevel::Off
    }

    // ------------------------------------------------------------------------
    /// Whether it can run as `what` ("the email analysis"): nothing to check
    /// while it is off, else its pick (`AiAccounts::choose`).
    pub fn check(&self, ai: &AiAccounts, what: Text) -> std::result::Result<(), Refusal> {
        if !self.is_on() {
            return Ok(());
        }
        ai.choose(self.model.as_ref(), what, DateTime::now())
            .map(|_| ())
    }
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

// ----------------------------------------------------------------------------
/// What the analysis of a watch of the type `kind` is called in a refused
/// pick's words (`AiAccounts::choose`).
pub fn scan_analysis(kind: WatchKind) -> Text {
    match kind {
        WatchKind::Images | WatchKind::Statements => Text::new("feature.scan_analysis"),
        WatchKind::Pdf => Text::new("feature.pdf_analysis"),
    }
}

#[cfg(test)]
mod tests;
