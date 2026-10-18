//! Token usage, prices, and the ledger summing both.
//!
//! `Usage` is normalized across providers so that a call's cost is a plain
//! sum of tokens times prices: `input` is only the input billed at the full
//! rate (providers that count cache reads into their prompt tokens have them
//! subtracted), and `output` includes reasoning (providers that count it
//! apart have it added).

use std::collections::BTreeMap;
use std::ops::AddAssign;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};

use crate::types::Provider;

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Input tokens at the full rate (neither read from nor written to a
    /// prompt cache).
    pub input: u64,
    /// Output tokens, reasoning included.
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// The part of `output` spent on reasoning, where the provider tells.
    pub reasoning: u64,
}

impl AddAssign for Usage {
    fn add_assign(&mut self, other: Self) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.reasoning += other.reasoning;
    }
}

// ----------------------------------------------------------------------------
/// USD per million tokens.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl Price {
    /// A model running on the user's own machine.
    pub const FREE: Price = Price::new(0.0, 0.0, 0.0, 0.0);

    // ------------------------------------------------------------------------
    pub const fn new(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Self {
        Self {
            input,
            output,
            cache_read,
            cache_write,
        }
    }

    // ------------------------------------------------------------------------
    /// USD.
    pub fn cost(&self, usage: &Usage) -> f64 {
        (usage.input as f64 * self.input
            + usage.output as f64 * self.output
            + usage.cache_read as f64 * self.cache_read
            + usage.cache_write as f64 * self.cache_write)
            / 1_000_000.0
    }
}

// ----------------------------------------------------------------------------
/// Anthropic: input, output, cache hit, 5-minute cache write.
const fn anthropic(input: f64, output: f64, cache_read: f64) -> Price {
    Price::new(input, output, cache_read, input * 1.25)
}

/// No cache surcharge: writes cost the input price.
const fn cached(input: f64, output: f64, cache_read: f64) -> Price {
    Price::new(input, output, cache_read, input)
}

/// No cache discount either.
const fn flat(input: f64, output: f64) -> Price {
    Price::new(input, output, input, input)
}

/// List prices, standard tier, from the providers' pricing pages. A model
/// id matches its entry exactly or with a `-` suffix (a dated version); the
/// longest entry wins. Gemini: prompts up to 200k tokens.
const PRICES: &[(Provider, &str, Price)] = &[
    (
        Provider::Anthropic,
        "claude-fable-5-1",
        anthropic(10.0, 50.0, 0.25),
    ),
    (
        Provider::Anthropic,
        "claude-fable-5",
        anthropic(10.0, 50.0, 1.0),
    ),
    (
        Provider::Anthropic,
        "claude-opus-5-5",
        anthropic(4.0, 20.0, 0.2),
    ),
    (
        Provider::Anthropic,
        "claude-opus-5",
        anthropic(5.0, 25.0, 0.5),
    ),
    (
        Provider::Anthropic,
        "claude-opus-4-8",
        anthropic(5.0, 25.0, 0.5),
    ),
    (
        Provider::Anthropic,
        "claude-opus-4-7",
        anthropic(5.0, 25.0, 0.5),
    ),
    (
        Provider::Anthropic,
        "claude-opus-4-6",
        anthropic(5.0, 25.0, 0.5),
    ),
    (
        Provider::Anthropic,
        "claude-opus-4-5",
        anthropic(5.0, 25.0, 0.5),
    ),
    (
        Provider::Anthropic,
        "claude-sonnet-5-5",
        anthropic(2.0, 10.0, 0.2),
    ),
    (
        Provider::Anthropic,
        "claude-sonnet-5",
        anthropic(2.0, 10.0, 0.2),
    ),
    (
        Provider::Anthropic,
        "claude-sonnet-4-6",
        anthropic(3.0, 15.0, 0.3),
    ),
    (
        Provider::Anthropic,
        "claude-sonnet-4-5",
        anthropic(3.0, 15.0, 0.3),
    ),
    (
        Provider::Anthropic,
        "claude-haiku-4-5",
        anthropic(1.0, 5.0, 0.1),
    ),
    (Provider::OpenAi, "gpt-6-astra", cached(10.0, 50.0, 1.0)),
    (Provider::OpenAi, "gpt-6-sol", cached(2.0, 10.0, 0.2)),
    (Provider::OpenAi, "gpt-6-luna", cached(0.1, 0.5, 0.01)),
    (Provider::OpenAi, "gpt-5.6-sol", cached(4.0, 20.0, 0.4)),
    (Provider::OpenAi, "gpt-5.6-terra", cached(2.0, 12.0, 0.2)),
    (Provider::OpenAi, "gpt-5.6-luna", cached(0.2, 1.2, 0.02)),
    (Provider::OpenAi, "gpt-5.5", cached(5.0, 30.0, 0.5)),
    (Provider::OpenAi, "gpt-5.5-pro", flat(30.0, 180.0)),
    (Provider::OpenAi, "gpt-5.4", cached(2.5, 15.0, 0.25)),
    (Provider::OpenAi, "gpt-5.4-mini", cached(0.75, 4.5, 0.075)),
    (Provider::OpenAi, "gpt-5.4-nano", cached(0.2, 1.25, 0.02)),
    (Provider::OpenAi, "gpt-5.4-pro", flat(30.0, 180.0)),
    (Provider::OpenAi, "gpt-5.3-codex", cached(1.75, 14.0, 0.175)),
    (Provider::OpenAi, "gpt-5.2", cached(1.75, 14.0, 0.175)),
    (Provider::OpenAi, "gpt-5.2-pro", flat(21.0, 168.0)),
    (Provider::OpenAi, "gpt-5.1", cached(1.25, 10.0, 0.125)),
    (Provider::OpenAi, "gpt-5", cached(1.25, 10.0, 0.125)),
    (Provider::OpenAi, "gpt-5-mini", cached(0.25, 2.0, 0.025)),
    (Provider::OpenAi, "gpt-5-nano", cached(0.05, 0.4, 0.005)),
    (Provider::OpenAi, "gpt-5-pro", flat(15.0, 120.0)),
    (Provider::OpenAi, "gpt-4.1", cached(2.0, 8.0, 0.5)),
    (Provider::OpenAi, "gpt-4.1-mini", cached(0.4, 1.6, 0.1)),
    (Provider::OpenAi, "gpt-4.1-nano", cached(0.1, 0.4, 0.025)),
    (Provider::OpenAi, "gpt-4o", cached(2.5, 10.0, 1.25)),
    (Provider::OpenAi, "gpt-4o-2024-05-13", flat(5.0, 15.0)),
    (Provider::OpenAi, "gpt-4o-mini", cached(0.15, 0.6, 0.075)),
    (Provider::OpenAi, "o1", cached(15.0, 60.0, 7.5)),
    (Provider::OpenAi, "o1-pro", flat(150.0, 600.0)),
    (Provider::OpenAi, "o3", cached(2.0, 8.0, 0.5)),
    (Provider::OpenAi, "o3-pro", flat(20.0, 80.0)),
    (Provider::OpenAi, "o3-mini", cached(1.1, 4.4, 0.55)),
    (Provider::OpenAi, "o4-mini", cached(1.1, 4.4, 0.275)),
    (Provider::Mistral, "mistral-medium-latest", flat(1.5, 7.5)),
    (Provider::Mistral, "mistral-medium-2604", flat(1.5, 7.5)),
    (Provider::Mistral, "mistral-small-latest", flat(0.15, 0.6)),
    (Provider::Mistral, "mistral-small-2603", flat(0.15, 0.6)),
    (Provider::Mistral, "mistral-large-latest", flat(0.5, 1.5)),
    (Provider::Mistral, "mistral-large-2512", flat(0.5, 1.5)),
    (Provider::Mistral, "ministral-14b-latest", flat(0.2, 0.2)),
    (Provider::Mistral, "ministral-14b-2512", flat(0.2, 0.2)),
    (Provider::Mistral, "ministral-8b-latest", flat(0.15, 0.15)),
    (Provider::Mistral, "ministral-8b-2512", flat(0.15, 0.15)),
    (Provider::Mistral, "ministral-3b-latest", flat(0.1, 0.1)),
    (Provider::Mistral, "ministral-3b-2512", flat(0.1, 0.1)),
    (
        Provider::Gemini,
        "gemini-3.8-flash",
        cached(0.75, 3.75, 0.075),
    ),
    (
        Provider::Gemini,
        "gemini-3.7-flash",
        cached(0.75, 3.75, 0.075),
    ),
    (
        Provider::Gemini,
        "gemini-3.6-flash",
        cached(0.75, 3.75, 0.075),
    ),
    (Provider::Gemini, "gemini-3.5-flash", cached(1.5, 9.0, 0.15)),
    (
        Provider::Gemini,
        "gemini-3.5-flash-lite",
        cached(0.3, 2.5, 0.03),
    ),
    (
        Provider::Gemini,
        "gemini-3.1-flash-lite",
        cached(0.25, 1.5, 0.025),
    ),
    (
        Provider::Gemini,
        "gemini-3.1-pro-preview",
        cached(2.0, 12.0, 0.2),
    ),
    (
        Provider::Gemini,
        "gemini-2.5-pro",
        cached(1.25, 10.0, 0.125),
    ),
    (Provider::Gemini, "gemini-2.5-flash", cached(0.3, 2.5, 0.03)),
    (
        Provider::Gemini,
        "gemini-2.5-flash-lite",
        cached(0.1, 0.4, 0.01),
    ),
];

// ----------------------------------------------------------------------------
/// The list price of `model`, if the table knows it.
pub fn list_price(provider: Provider, model: &str) -> Option<Price> {
    let model = model.strip_prefix("models/").unwrap_or(model);
    PRICES
        .iter()
        .filter(|(p, id, _)| *p == provider && matches_id(model, id))
        .max_by_key(|(_, id, _)| id.len())
        .map(|(_, _, price)| *price)
}

// ----------------------------------------------------------------------------
/// `model` is `id` or a version of it (`id-…`).
fn matches_id(model: &str, id: &str) -> bool {
    model
        .strip_prefix(id)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
}

// ----------------------------------------------------------------------------
/// What the calls to one model added up to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Totals {
    pub calls: u64,
    pub usage: Usage,
    /// USD, of the calls whose price was known.
    pub cost: f64,
    /// Calls whose price was unknown (not in `cost`).
    pub unpriced_calls: u64,
}

// ----------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LedgerEntry {
    pub provider: Provider,
    pub model: String,
    #[serde(flatten)]
    pub totals: Totals,
}

// ----------------------------------------------------------------------------
/// Usage and cost per provider and model, summed over the calls of every
/// `Client` sharing it. In memory only.
#[derive(Default)]
pub struct Ledger {
    totals: Mutex<BTreeMap<(Provider, String), Totals>>,
}

impl Ledger {
    // ------------------------------------------------------------------------
    pub fn new() -> Self {
        Self::default()
    }

    // ------------------------------------------------------------------------
    pub fn record(&self, provider: Provider, model: &str, usage: &Usage, cost: Option<f64>) {
        let mut totals = lock(&self.totals);
        let entry = totals.entry((provider, model.to_string())).or_default();
        entry.calls += 1;
        entry.usage += *usage;
        match cost {
            Some(cost) => entry.cost += cost,
            None => entry.unpriced_calls += 1,
        }
    }

    // ------------------------------------------------------------------------
    /// Every model called since the last `drain`, taken out: the ledger
    /// starts over. For whoever keeps the totals elsewhere, e.g. per month
    /// on disk.
    pub fn drain(&self) -> Vec<LedgerEntry> {
        std::mem::take(&mut *lock(&self.totals))
            .into_iter()
            .map(|((provider, model), totals)| LedgerEntry {
                provider,
                model,
                totals,
            })
            .collect()
    }
}

// ----------------------------------------------------------------------------
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Sums stay consistent per update; a poisoned lock is usable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn list_price_matches_versions_and_prefers_the_longest_id() {
        let price = |p, m| list_price(p, m).map(|p: Price| (p.input, p.output));
        assert_eq!(
            price(Provider::Anthropic, "claude-opus-5-5"),
            Some((4.0, 20.0))
        );
        assert_eq!(
            price(Provider::Anthropic, "claude-opus-5"),
            Some((5.0, 25.0))
        );
        assert_eq!(
            price(Provider::Anthropic, "claude-haiku-4-5-20251001"),
            Some((1.0, 5.0)),
            "a dated version"
        );
        assert_eq!(price(Provider::OpenAi, "gpt-5-mini"), Some((0.25, 2.0)));
        assert_eq!(
            price(Provider::OpenAi, "gpt-5-2025-08-07"),
            Some((1.25, 10.0))
        );
        assert_eq!(
            price(Provider::OpenAi, "gpt-5.9"),
            None,
            "not a version of gpt-5"
        );
        assert_eq!(
            price(Provider::Gemini, "models/gemini-2.5-flash-lite"),
            Some((0.1, 0.4))
        );
        assert_eq!(price(Provider::Anthropic, "gpt-5"), None, "per provider");
        assert_eq!(price(Provider::Ollama, "llama3.2"), None);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn cost_sums_every_kind_of_token() {
        let usage = Usage {
            input: 1_000_000,
            output: 500_000,
            cache_read: 2_000_000,
            cache_write: 100_000,
            reasoning: 200_000,
        };
        let price = anthropic(3.0, 15.0, 0.3);
        // 3 + 7.5 + 0.6 + 0.375; reasoning is part of output.
        assert!((price.cost(&usage) - 11.475).abs() < 1e-9);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn drain_takes_the_totals() {
        let ledger = Ledger::new();
        let usage = Usage {
            input: 10,
            ..Usage::default()
        };
        ledger.record(Provider::OpenAi, "gpt-5", &usage, Some(0.5));

        let drained = ledger.drain();
        assert_eq!(drained.len(), 1);
        assert_eq!(
            (drained[0].model.as_str(), drained[0].totals.calls),
            ("gpt-5", 1)
        );
        assert!(ledger.drain().is_empty());
    }
}
