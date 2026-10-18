//! The names the vault's files use beyond the envelope's typed fields: the
//! folders entities are written into (vault.md#vault-layout).

/// The folders the app writes entities into, relative to the vault's root
/// (vault.md#vault-layout). A convention: entities are found by
/// their frontmatter, wherever they are.
pub mod layout {
    /// Projects, made with `Index::create_project`.
    pub const PROJECTS: &str = "Projects";
    /// Documents (notes) made in the app (`Index::create_new`).
    pub const DOCUMENTS: &str = "Documents";
    /// Tasks made in the app.
    pub const TASKS: &str = "Tasks";
    /// Events made in the app, in no calendar.
    pub const CREATED_EVENTS: &str = "Calendar/Created Here";
    /// Contacts made in the app.
    pub const CONTACTS: &str = "Contacts";
    /// Lists made in the app.
    pub const LISTS: &str = "Lists";
    /// Records made in the app.
    pub const RECORDS: &str = "Records";
    /// Conversations with the assistant.
    pub const CONVERSATIONS: &str = "Conversations";
    /// Deleted entities, kept at their vault-relative path; the scan skips
    /// it like every dot-folder.
    pub const TRASH: &str = ".trash";
    /// The vault's accounts (AI providers), one YAML file each; no
    /// entities, and skipped by the scan like every dot-folder. Their
    /// secrets stay with the app.
    pub const ACCOUNTS: &str = ".avelor/accounts";
    /// The vault's own settings (`vault.md#vault-config`); skipped by the
    /// scan like every dot-folder.
    pub const VAULT_SETTINGS: &str = ".avelor/vault.yaml";
}
