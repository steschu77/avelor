//! The vault domain: envelopes (YAML frontmatter with lossless unknown-key
//! round-trips) and the scanned in-memory index.
//!
//! Pure file-and-data logic — no UI, no network.

pub mod change;
pub mod conversation;
pub mod document;
pub mod envelope;
pub mod filename;
pub mod markdown;
pub mod schema;
pub mod shared;
pub mod vault;
