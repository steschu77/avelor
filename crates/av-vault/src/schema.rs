//! The names the vault's files use beyond the envelope's typed fields: the
//! folders entities are written into (vault.md#vault-layout).

/// The folders the app writes entities into, relative to the vault's root
/// (vault.md#vault-layout). A convention: entities are found by
/// their frontmatter, wherever they are.
pub mod layout {
    /// Projects, made with `Index::create_project`.
    pub const PROJECTS: &str = "Projects";
}
