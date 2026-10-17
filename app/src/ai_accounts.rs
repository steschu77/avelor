//! AI accounts (accounts.md#ai-account-file): one YAML file per account in the
//! vault's `.avelor/accounts/`, its block named after the provider (`openai:`,
//! `ollama:`, …). An API key is kept DPAPI-encrypted with the app (on Linux,
//! in a file only the user can read; `config_files::SecretFile`); the key
//! never enters the YAML, the vault or the page. `AiAccounts` holds the open
//! vault's accounts; the rest of the app reaches them only through it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use av_llm::{Client, PROVIDERS, Provider};
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

use crate::config_files::{ConnectionStatus, SecretFile, free_stem, remove_if_exists, write_yaml};

/// The extension of an AI account's secret, its API key.
const KEY: &str = "key";

// ----------------------------------------------------------------------------
/// An AI account's block. Unknown keys are kept, so hand-added settings
/// survive the app rewriting the file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AiSettings {
    /// Where the API lives, if not at the provider's default address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// The models offered wherever a model is picked, in the order ticked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offered: Vec<String>,
    /// USD a month.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_limit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected: Option<DateTime>,
    /// When the key was forgotten: the account is meant to have none.
    /// Without it, a missing key means a new key is needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disconnected: Option<DateTime>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

// ----------------------------------------------------------------------------
/// A configured AI account, with its key once decrypted.
#[derive(Clone)]
pub struct AiAccount {
    /// The account file.
    path: PathBuf,
    id: Id,
    provider: Provider,
    /// The block's key as found (`claude` stays `claude`).
    kind_key: String,
    settings: AiSettings,
    /// The file's other top-level keys, kept as they are.
    rest: Mapping,
    /// `None`: no key (see `status`), or the key is missing or can't be
    /// decrypted (e.g. the vault was opened on another computer).
    key: Option<String>,
    /// Ollama: whether the last model listing reached the server; `None`
    /// until one ran.
    reachable: Option<bool>,
}

// ----------------------------------------------------------------------------
/// What the page gets per AI account (the `account` event).
#[derive(Serialize)]
pub struct AiAccountMeta<'a> {
    pub id: Id,
    pub kind: &'static str,
    /// The key's first and last characters (`sk-…4f2a`), if there is one.
    pub key: Option<String>,
    pub address: &'a str,
    /// See `AiAccount::status`.
    pub status: &'static str,
    pub connected: Option<DateTime>,
    pub offered: &'a [String],
    pub spend_limit: Option<f64>,
}

impl AiAccount {
    // ------------------------------------------------------------------------
    /// A new, unsaved account of `provider`, in the first free file of
    /// `folder`: `openai.yaml`, then `openai_2.yaml`, …
    pub fn new(folder: &Path, provider: Provider) -> Self {
        let kind = provider.as_str();
        let stem = free_stem(folder, kind, &["yaml"]);
        Self {
            path: folder.join(format!("{stem}.yaml")),
            id: Id::generate(),
            provider,
            kind_key: kind.to_string(),
            settings: AiSettings {
                connected: Some(DateTime::now()),
                ..AiSettings::default()
            },
            rest: Mapping::new(),
            key: None,
            reachable: None,
        }
    }

    // ------------------------------------------------------------------------
    pub fn id(&self) -> Id {
        self.id
    }

    // ------------------------------------------------------------------------
    pub fn provider(&self) -> Provider {
        self.provider
    }

    // ------------------------------------------------------------------------
    /// Whether the account works with an API key; Ollama is addressed
    /// instead.
    pub fn uses_key(&self) -> bool {
        self.provider.spec().needs_key
    }

    // ------------------------------------------------------------------------
    pub fn has_key(&self) -> bool {
        self.key.is_some()
    }

    // ------------------------------------------------------------------------
    /// Where the API lives.
    pub fn address(&self) -> &str {
        self.settings
            .address
            .as_deref()
            .unwrap_or(self.provider.spec().default_url)
    }

    // ------------------------------------------------------------------------
    /// A client for the account's key and address.
    pub fn client(&self) -> Client {
        candidate(self.provider, self.key.clone(), self.address())
    }

    // ------------------------------------------------------------------------
    /// Take a checked key (`None` keeps the current one) and address
    /// (`None` keeps it; the provider's default address is not written).
    fn set_credentials(&mut self, key: Option<String>, address: Option<String>) {
        if let Some(key) = key {
            self.key = Some(key);
            self.settings.disconnected = None;
        }
        if let Some(address) = address {
            let default = address == self.provider.spec().default_url;
            self.settings.address = (!default).then_some(address);
        }
    }

    // ------------------------------------------------------------------------
    /// Key providers: `connected`, `disconnected` or `reconnect`
    /// (`ConnectionStatus`). Ollama: `running` or `not running` after a
    /// model listing, `unknown` before.
    pub fn status(&self) -> &'static str {
        if !self.uses_key() {
            return match self.reachable {
                Some(true) => "running",
                Some(false) => "not running",
                None => "unknown",
            };
        }
        let disconnected = self.settings.disconnected.is_some();
        ConnectionStatus::of(self.key.is_some(), disconnected).name()
    }

    // ------------------------------------------------------------------------
    pub fn meta(&self) -> AiAccountMeta<'_> {
        AiAccountMeta {
            id: self.id,
            kind: self.provider.as_str(),
            key: self.key.as_deref().map(key_hint),
            address: self.address(),
            status: self.status(),
            connected: self.settings.connected,
            offered: &self.settings.offered,
            spend_limit: self.settings.spend_limit,
        }
    }

    // ------------------------------------------------------------------------
    fn secret(&self, secrets: &Path) -> SecretFile {
        SecretFile::of(secrets, self.id, KEY)
    }

    // ------------------------------------------------------------------------
    /// The file's content: `id`, the provider block, then the other keys.
    fn to_mapping(&self, settings: &AiSettings) -> Result<Mapping> {
        let mut map = Mapping::new();
        map.insert("id".into(), serde_yaml_ng::to_value(self.id)?);
        map.insert(
            self.kind_key.as_str().into(),
            serde_yaml_ng::to_value(settings)?,
        );
        for (k, v) in &self.rest {
            map.insert(k.clone(), v.clone());
        }
        Ok(map)
    }

    // ------------------------------------------------------------------------
    /// Write the key (encrypted, to `secrets`) first, then the account
    /// file, so a crash in between never leaves a connected file without
    /// its key.
    fn save(&self, secrets: &Path) -> Result<()> {
        if let Some(key) = &self.key {
            self.secret(secrets).write(key)?;
        }
        write_yaml(&self.path, &self.to_mapping(&self.settings)?)?;
        log::info!("Saved AI account {}", self.path.display());
        Ok(())
    }

    // ------------------------------------------------------------------------
    /// Forget the key but keep the account (and its id), marked
    /// `disconnected`. Nothing changes if writing the file fails.
    fn disconnect(&mut self, secrets: &Path) -> Result<()> {
        let mut settings = self.settings.clone();
        settings.disconnected = Some(DateTime::now());
        write_yaml(&self.path, &self.to_mapping(&settings)?)?;
        self.settings = settings;
        self.key = None;
        self.secret(secrets).remove()?;
        log::info!("Disconnected AI account {}", self.path.display());
        Ok(())
    }

    // ------------------------------------------------------------------------
    /// Delete the account file and its key if there is one.
    fn remove(&self, secrets: &Path) -> Result<()> {
        remove_if_exists(&self.path)?;
        self.secret(secrets).remove()?;
        log::info!("Removed AI account {}", self.path.display());
        Ok(())
    }
}

// ----------------------------------------------------------------------------
/// A client for `provider` at `address`, e.g. to check a key before it is
/// stored.
pub fn candidate(provider: Provider, key: Option<String>, address: &str) -> Client {
    Client::new(provider, key).with_base_url(address)
}

// ----------------------------------------------------------------------------
/// `sk-…4f2a`: enough to tell keys apart, too little to use one. Short
/// keys show nothing.
fn key_hint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() < 12 {
        return "…".to_string();
    }
    let head: String = chars[..3].iter().collect();
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{head}…{tail}")
}

// ----------------------------------------------------------------------------
/// The provider block's key of an account file's top level, if it is an AI
/// account's: a provider's key or alias (`claude` is Anthropic).
pub(crate) fn kind_of(map: &Mapping) -> Option<&'static str> {
    PROVIDERS
        .iter()
        .flat_map(|spec| std::iter::once(&spec.key).chain(spec.aliases))
        .copied()
        .find(|kind| map.contains_key(*kind))
}

// ----------------------------------------------------------------------------
/// The AI account in `path` (`map`, its parsed top level, with the block
/// `kind_key`), its key in `secrets`.
pub(crate) fn load(
    path: &Path,
    mut map: Mapping,
    kind_key: &str,
    secrets: &Path,
) -> Result<AiAccount> {
    let provider: Provider = kind_key.parse()?;
    let block = map.shift_remove(kind_key).unwrap_or_default();
    let id = map.shift_remove("id").ok_or(Error::InvalidArgument {
        arg: "id".to_string(),
    })?;
    let id: Id = serde_yaml_ng::from_value(id)?;
    let settings: AiSettings = match block {
        Value::Null => AiSettings::default(),
        block => serde_yaml_ng::from_value(block)?,
    };

    let mut account = AiAccount {
        path: path.to_path_buf(),
        id,
        provider,
        kind_key: kind_key.to_string(),
        settings,
        rest: map,
        key: None,
        reachable: None,
    };
    let secret = account.secret(secrets);
    if account.uses_key() {
        match secret.read() {
            Ok(key) => account.key = Some(key),
            Err(e) if account.settings.disconnected.is_none() => {
                log::warn!("AI account {} needs a new key: {e}", path.display());
            }
            Err(_) => {}
        }
    }
    log::info!(
        "Loaded AI account {} ({provider}, {})",
        path.display(),
        account.status()
    );
    Ok(account)
}

// ----------------------------------------------------------------------------
/// The open vault's AI accounts: their files in `folder` (the vault's
/// `.avelor/accounts`), their keys in `secrets` (the config folder). Without a
/// vault it is empty, and nothing can be added.
#[derive(Default)]
pub struct AiAccounts {
    folder: PathBuf,
    secrets: PathBuf,
    list: Vec<AiAccount>,
}

impl AiAccounts {
    // ------------------------------------------------------------------------
    pub fn new(folder: PathBuf, secrets: PathBuf, list: Vec<AiAccount>) -> Self {
        Self {
            folder,
            secrets,
            list,
        }
    }

    // ------------------------------------------------------------------------
    pub fn iter(&self) -> std::slice::Iter<'_, AiAccount> {
        self.list.iter()
    }

    // ------------------------------------------------------------------------
    /// The account `id`; `NotFound` if there is none.
    pub fn get(&self, id: Id) -> Result<&AiAccount> {
        self.list
            .iter()
            .find(|a| a.id() == id)
            .ok_or(Error::NotFound)
    }

    // ------------------------------------------------------------------------
    pub fn contains(&self, id: Id) -> bool {
        self.get(id).is_ok()
    }

    // ------------------------------------------------------------------------
    /// Where the accounts are kept: the open vault's `.avelor/accounts`; empty
    /// without a vault.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    // ------------------------------------------------------------------------
    fn index(&self, id: Id) -> Result<usize> {
        self.list
            .iter()
            .position(|a| a.id() == id)
            .ok_or(Error::NotFound)
    }

    // ------------------------------------------------------------------------
    /// Store a checked key and/or address (`None` keeps the account's) for
    /// the account `target`, or for a new account of `provider`; its id. A
    /// failed save leaves the accounts as they were.
    pub fn connect(
        &mut self,
        target: Option<Id>,
        provider: Provider,
        key: Option<String>,
        address: Option<String>,
    ) -> Result<Id> {
        let index = target.map(|id| self.index(id)).transpose()?;
        let mut account = match index {
            Some(index) => self.list[index].clone(),
            None if self.folder.as_os_str().is_empty() => {
                return Err(Error::InvalidPath {
                    path: PathBuf::new(),
                });
            }
            None => AiAccount::new(&self.folder, provider),
        };
        account.set_credentials(key, address);
        account.reachable = Some(true);
        account.save(&self.secrets)?;
        let id = account.id();
        match index {
            Some(index) => self.list[index] = account,
            None => self.list.push(account),
        }
        Ok(id)
    }

    // ------------------------------------------------------------------------
    /// Change the settings of the account `id` with `f`, and save them; a
    /// failed save leaves them as they were.
    pub fn update(&mut self, id: Id, f: impl FnOnce(&mut AiSettings)) -> Result<()> {
        let index = self.index(id)?;
        let account = &mut self.list[index];
        let before = account.settings.clone();
        f(&mut account.settings);
        if let Err(e) = account.save(&self.secrets) {
            account.settings = before;
            return Err(e);
        }
        Ok(())
    }

    // ------------------------------------------------------------------------
    /// Whether the last model listing of the account `id` reached its
    /// server (an Ollama account's status).
    pub fn set_reachable(&mut self, id: Id, reachable: bool) {
        if let Ok(index) = self.index(id) {
            self.list[index].reachable = Some(reachable);
        }
    }

    // ------------------------------------------------------------------------
    /// Forget the key of the account `id`, keep the account: marked
    /// `disconnected`.
    pub fn disconnect(&mut self, id: Id) -> Result<()> {
        let index = self.index(id)?;
        self.list[index].disconnect(&self.secrets)
    }

    // ------------------------------------------------------------------------
    /// Forget the account `id`, which must have no key (Ollama has none):
    /// its files are deleted.
    pub fn remove(&mut self, id: Id) -> Result<()> {
        let index = self.index(id)?;
        let account = &self.list[index];
        if account.uses_key() && account.has_key() {
            let what = "removing an account with a key (disconnect it first)".to_string();
            return Err(Error::Unsupported { what });
        }
        account.remove(&self.secrets)?;
        self.list.remove(index);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const OPENAI: &str = "id: 01a0e1cc-a24c-7490-82fe-e75669985b30
openai:
  offered:
    - gpt-5-mini
    - gpt-5-nano
  spend-limit: 10.0
  connected: 2026-09-30T10:00:00Z
  custom: kept
note: top-level kept
";

    struct Dirs {
        root: PathBuf,
        folder: PathBuf,
        secrets: PathBuf,
    }

    impl Dirs {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("avelor-ai-test-{}", Id::generate()));
            let (folder, secrets) = (root.join("accounts"), root.join("config"));
            fs::create_dir_all(&folder).unwrap();
            fs::create_dir_all(&secrets).unwrap();
            Self {
                root,
                folder,
                secrets,
            }
        }

        fn load(&self, name: &str) -> AiAccount {
            let path = self.folder.join(name);
            let text = fs::read_to_string(&path).unwrap();
            let Value::Mapping(map) = serde_yaml_ng::from_str(&text).unwrap() else {
                panic!("not a mapping");
            };
            let kind = kind_of(&map).unwrap();
            load(&path, map, kind, &self.secrets).unwrap()
        }

        fn key(&self, id: Id) -> PathBuf {
            self.secrets.join(format!("{id}.key"))
        }
    }

    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn save_keeps_unknown_keys_and_the_key_stays_out_of_the_yaml() {
        let dirs = Dirs::new();
        fs::write(dirs.folder.join("openai.yaml"), OPENAI).unwrap();
        let mut account = dirs.load("openai.yaml");
        assert_eq!(account.status(), "reconnect", "no key, no mark");
        account.set_credentials(Some("sk-new-key-0123456789".into()), None);
        account.settings.offered.push("gpt-5".into());
        account.save(&dirs.secrets).unwrap();

        let text = fs::read_to_string(dirs.folder.join("openai.yaml")).unwrap();
        let again = dirs.load("openai.yaml");

        assert!(text.starts_with("id: "), "{text}");
        assert!(text.contains("custom: kept") && text.contains("note: top-level kept"));
        assert!(!text.contains("sk-new"), "the key never enters the YAML");
        assert!(
            dirs.key(account.id()).exists(),
            "the key is named by the id"
        );
        assert_eq!(again.id(), account.id());
        assert_eq!(again.key.as_deref(), Some("sk-new-key-0123456789"));
        assert_eq!(again.status(), "connected");
        assert_eq!(
            again.settings.offered.last().map(String::as_str),
            Some("gpt-5")
        );
        assert_eq!(again.settings.spend_limit, Some(10.0));
        assert_eq!(again.meta().key.as_deref(), Some("sk-…6789"));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn disconnect_marks_the_account_and_remove_cleans_up() {
        let dirs = Dirs::new();
        fs::write(dirs.folder.join("openai.yaml"), OPENAI).unwrap();
        let mut account = dirs.load("openai.yaml");
        let id = account.id();
        account.set_credentials(Some("sk-0123456789ab".into()), None);
        account.save(&dirs.secrets).unwrap();

        account.disconnect(&dirs.secrets).unwrap();
        assert!(!dirs.key(id).exists(), "the key file is gone");
        let text = fs::read_to_string(dirs.folder.join("openai.yaml")).unwrap();
        assert!(text.contains("disconnected:"), "{text}");
        let reloaded = dirs.load("openai.yaml");
        assert_eq!((reloaded.id(), reloaded.status()), (id, "disconnected"));

        reloaded.remove(&dirs.secrets).unwrap();
        let left: Vec<_> = fs::read_dir(&dirs.folder).unwrap().collect();
        assert!(left.is_empty(), "the account file is gone");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn new_accounts_take_the_first_free_file() {
        let dirs = Dirs::new();
        let mut first = AiAccount::new(&dirs.folder, Provider::OpenAi);
        assert_eq!(first.path, dirs.folder.join("openai.yaml"));
        first.set_credentials(Some("sk-0123456789abcdef".into()), None);
        first.save(&dirs.secrets).unwrap();
        let second = AiAccount::new(&dirs.folder, Provider::OpenAi);
        let ollama = AiAccount::new(&dirs.folder, Provider::Ollama);

        assert_eq!(second.path, dirs.folder.join("openai_2.yaml"));
        assert_eq!(ollama.path, dirs.folder.join("ollama.yaml"));
        assert!(first.settings.connected.is_some());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_default_address_is_not_written() {
        let mut ollama = AiAccount::new(Path::new("cfg"), Provider::Ollama);
        ollama.set_credentials(None, Some("http://localhost:11434".into()));
        assert_eq!(ollama.settings.address, None);
        ollama.set_credentials(None, Some("http://nas:11434".into()));
        assert_eq!(ollama.address(), "http://nas:11434");
        assert!(!ollama.uses_key() && ollama.status() == "unknown");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn key_hints_show_only_the_ends() {
        assert_eq!(key_hint("sk-proj-abcdefghijklmnop4f2a"), "sk-…4f2a");
        assert_eq!(key_hint("short"), "…");
    }
}
