//! The vault domain: envelopes (YAML frontmatter with lossless unknown-key
//! round-trips), the `remote:` sync-bookkeeping block, the scanned
//! in-memory index, and the typed models built on top (calendar events,
//! conversation trees).
//!
//! Pure file-and-data logic — no UI, no network.

pub mod change;
pub mod conversation;
pub mod document;
pub mod email;
pub mod envelope;
pub mod filename;
pub mod html;
pub mod markdown;
pub mod md_writer;
pub mod schema;
pub mod shared;
pub mod vault;
