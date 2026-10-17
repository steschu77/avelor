//! LLMs for Avelor: one `Client` per AI account, the same calls for
//! every provider — Anthropic, OpenAI, Mistral, Gemini, and Ollama (local
//! or ollama.com).
//!
//! - `Client::models` lists what the account can use.
//!
//! Everything blocks: requests go through av-http, each under the caller's
//! `av_utils::cancel::Cancel`; the caller runs them on a worker thread. Failures are
//! `av_utils::error::Error`: `Http` for an error status (with the provider's
//! error type as `reason`).

mod anthropic;
mod client;
mod gemini;
mod mistral;
mod ollama;
mod openai;
mod types;
mod usage;

pub use client::Client;
pub use types::{ModelInfo, PROVIDERS, Provider, ProviderSpec, base_url};
pub use usage::Price;
