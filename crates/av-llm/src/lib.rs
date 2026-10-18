//! LLMs for Avelor: one `Client` per AI account, the same calls for
//! every provider — Anthropic, OpenAI (Responses API), Mistral, Gemini, and
//! Ollama (local or ollama.com).
//!
//! - `Client::models` lists what the account can use,
//! - `Client::chat_stream` runs a conversation turn (streamed text
//!   arrives through a callback; the `Cancel` passed in stops the request),
//! - every answer carries its normalized `Usage` and, where the model's
//!   price is known, its cost.
//!
//! Everything blocks: requests go through av-http, each under the caller's
//! `av_utils::cancel::Cancel`; the caller runs them on a worker thread. Failures are
//! `av_utils::error::Error`: `Http` for an error status (with the provider's
//! error type as `reason`), `InvalidResponse` for an answer that broke off or
//! can't be read.

mod anthropic;
mod client;
mod gemini;
mod mistral;
mod ollama;
mod openai;
mod stream;
mod types;
mod usage;

pub use client::Client;
pub use types::{ChatRequest, ChatResponse, Message, Part, Role, StopReason};
pub use types::{ModelInfo, PROVIDERS, Provider, ProviderSpec, base_url};
pub use usage::Price;
