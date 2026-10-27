//! The search (`vault.md#search`): an entity's words as a search reads
//! them, kept with its file's text in the catalog.

use av_store::catalog::Words;
use av_vault::document::Document;

// ----------------------------------------------------------------------------
/// `doc`'s words: its title, its labels a line each, and its text
/// (`Document::plain_text`). A change to them is a new format of the
/// catalog (`av_store::catalog::FORMAT`).
pub fn words_of(doc: &Document) -> Words {
    Words {
        title: doc.title().to_string(),
        labels: doc.tags().join("\n"),
        body: doc.plain_text(),
    }
}
