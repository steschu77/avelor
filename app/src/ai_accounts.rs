//! AI accounts (accounts.md#ai-account-file): one YAML file per account in the
//! vault's `.avelor/accounts/`, its block named after the provider (`openai:`,
//! `ollama:`, …), and beside it what the account's calls cost per month
//! (`openai.spend.yaml`). An API key is kept DPAPI-encrypted with the app
//! (on Linux, in a file only the user can read; `config_files::SecretFile`);
//! the key never enters the YAML, the vault or the page. `AiAccounts` holds
//! the open vault's accounts; the rest of the app reaches them only through
//! it.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use av_llm::{Client, PROVIDERS, Provider};
use av_llm::{Ledger, Totals};
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

use crate::config_files::{ConnectionStatus, SecretFile, free_stem, remove_if_exists, write_yaml};

/// The extension of an AI account's secret, its API key.
const KEY: &str = "key";

/// The extension of the file beside an account file that holds what its
/// calls cost (`openai.yaml` → `openai.spend.yaml`).
const SPEND: &str = "spend.yaml";

// ----------------------------------------------------------------------------
/// What an account's calls to one model added up to in one month.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub struct ModelSpend {
    pub calls: u64,
    /// Calls whose price was unknown (not in `cost`).
    pub unpriced_calls: u64,
    /// USD, of the calls whose price was known.
    pub cost: f64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
}

impl ModelSpend {
    // ------------------------------------------------------------------------
    fn add(&mut self, totals: &Totals) {
        self.calls += totals.calls;
        self.unpriced_calls += totals.unpriced_calls;
        self.cost += totals.cost;
        let usage = &totals.usage;
        self.input += usage.input;
        self.output += usage.output;
        self.cache_read += usage.cache_read;
        self.cache_write += usage.cache_write;
        self.reasoning += usage.reasoning;
    }
}

/// Per month (UTC, `2026-10`), per model id.
type Months = BTreeMap<String, BTreeMap<String, ModelSpend>>;

// ----------------------------------------------------------------------------
/// The spend file: the account it belongs to (a file left by a removed
/// account isn't taken over by a new one of the same name), and its months.
#[derive(Serialize, Deserialize)]
struct SpendFile {
    account: Id,
    #[serde(default)]
    months: Months,
}

// ----------------------------------------------------------------------------
/// The month `now` is in, as the spend file names it: `2026-10`, in UTC as
/// the providers bill.
fn month_of(now: DateTime) -> String {
    let (year, month, _) = now.date.to_ymd();
    format!("{year:04}-{month:02}")
}

// ----------------------------------------------------------------------------
/// The model a feature picked (accounts.md#model-choice): one of the models
/// an AI account offers, kept with the feature's settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelChoice {
    pub account: Id,
    pub model: String,
}

// ----------------------------------------------------------------------------
/// The account and model of a feature's pick (`AiAccounts::choose`), with a
/// client for them.
pub struct Resolved {
    pub account: Id,
    pub model: String,
    pub client: Client,
    pub provider: Provider,
    /// Where the API lives, for the words of a failure.
    pub address: String,
}

// ----------------------------------------------------------------------------
/// Why a use can't go to the AI: the error, and the host's words for the
/// page (`Req::fail`).
#[derive(Debug)]
pub struct Refusal {
    pub error: Error,
    pub message: String,
}

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
/// A configured AI account, with its key once decrypted. Clones share the
/// ledger.
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
    /// What the account's clients record, from their workers, until
    /// `take_spend` moves it into `spend`.
    ledger: Arc<Ledger>,
    /// What the account's calls cost, as in its spend file.
    spend: Months,
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
    pub spent: Spent,
}

// ----------------------------------------------------------------------------
/// What the account's calls cost this month.
#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Spent {
    /// USD, of the calls whose price was known.
    pub cost: f64,
    pub calls: u64,
    /// Calls whose price was unknown (not in `cost`).
    pub unpriced: u64,
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
            ledger: Arc::new(Ledger::new()),
            spend: Months::new(),
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
    /// A client for the account's key and address, recording into the
    /// account's ledger.
    pub fn client(&self) -> Client {
        candidate(self.provider, self.key.clone(), self.address())
            .with_ledger(Arc::clone(&self.ledger))
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
            spent: self.spent(&month_of(DateTime::now())),
        }
    }

    // ------------------------------------------------------------------------
    /// What the calls of `month` cost, all models together.
    pub fn spent(&self, month: &str) -> Spent {
        let models = self.spend.get(month).into_iter().flat_map(|m| m.values());
        models.fold(Spent::default(), |mut spent, model| {
            spent.cost += model.cost;
            spent.calls += model.calls;
            spent.unpriced += model.unpriced_calls;
            spent
        })
    }

    // ------------------------------------------------------------------------
    /// Whether the account has spent its monthly limit in `month`.
    pub fn limit_reached(&self, month: &str) -> bool {
        let limit = self.settings.spend_limit;
        limit.is_some_and(|limit| self.spent(month).cost >= limit)
    }

    // ------------------------------------------------------------------------
    /// Refuse a call once the account has spent its limit in `month`.
    fn check_limit(&self, month: &str) -> std::result::Result<(), Refusal> {
        if !self.limit_reached(month) {
            return Ok(());
        }
        let name = self.provider.spec().display;
        let limit = self.settings.spend_limit.unwrap_or_default();
        let message = format!(
            "{name} has reached its spend limit of ${limit:.2} this month; raise it under Accounts."
        );
        Err(Refusal {
            error: Error::SpendLimit,
            message,
        })
    }

    // ------------------------------------------------------------------------
    /// Whether it can be called: Ollama always, a key provider with its key.
    fn usable(&self) -> bool {
        !self.uses_key() || self.has_key()
    }

    // ------------------------------------------------------------------------
    /// Move what the clients recorded since the last time into `month`;
    /// whether there was anything.
    fn take_spend(&mut self, month: &str) -> bool {
        let drained = self.ledger.drain();
        if drained.is_empty() {
            return false;
        }
        let models = self.spend.entry(month.to_string()).or_default();
        for entry in drained {
            models.entry(entry.model).or_default().add(&entry.totals);
        }
        true
    }

    // ------------------------------------------------------------------------
    /// The spend file beside the account file.
    fn spend_path(&self) -> PathBuf {
        self.path.with_extension(SPEND)
    }

    // ------------------------------------------------------------------------
    fn save_spend(&self) -> Result<()> {
        let file = SpendFile {
            account: self.id,
            months: self.spend.clone(),
        };
        write_yaml(&self.spend_path(), &file)
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
    /// Forget the key but keep the account (and its id, and what it spent),
    /// marked `disconnected`. Nothing changes if writing the file fails.
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
    /// Delete the account file, its spend file, and its key if there is
    /// one.
    fn remove(&self, secrets: &Path) -> Result<()> {
        remove_if_exists(&self.path)?;
        remove_if_exists(&self.spend_path())?;
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
/// Whether `path` is an AI account's spend file (`openai.spend.yaml`), not
/// an account file.
pub(crate) fn is_spend_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    name.is_some_and(|name| name.to_ascii_lowercase().ends_with(&format!(".{SPEND}")))
}

// ----------------------------------------------------------------------------
/// The months of the spend file `path`, if it belongs to the account `id`.
/// A file that doesn't parse or names another account is logged, and
/// replaced on the next save.
fn load_spend(path: &Path, id: Id) -> Months {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Months::new(),
        Err(e) => {
            log::warn!("Could not read {}: {e}", path.display());
            return Months::new();
        }
    };
    match serde_yaml_ng::from_str::<SpendFile>(&text) {
        Ok(file) if file.account == id => file.months,
        Ok(file) => {
            let other = file.account;
            log::warn!("{} is account {other}'s, not {id}'s", path.display());
            Months::new()
        }
        Err(e) => {
            log::warn!("Could not read {}: {e}", path.display());
            Months::new()
        }
    }
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
        ledger: Arc::new(Ledger::new()),
        spend: Months::new(),
    };
    account.spend = load_spend(&account.spend_path(), id);
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

    // ------------------------------------------------------------------------
    /// The account, model and client of `choice`, the model a feature picked
    /// for `what` ("chats"; accounts.md#model-choice), as the page's
    /// `aiModelOf` shows it. Refused in words if nothing is picked, the
    /// account is gone or has no key, the model isn't offered any more, or
    /// the account has reached its spend limit in the month of `now` (UTC).
    /// No other model is ever tried: it could send the text elsewhere.
    pub fn choose(
        &self,
        choice: Option<&ModelChoice>,
        what: &str,
        now: DateTime,
    ) -> std::result::Result<Resolved, Refusal> {
        let refuse = |error, message: String| Err(Refusal { error, message });
        let no_model = || Error::InvalidArgument {
            arg: "model".to_string(),
        };
        let Some(choice) = choice else {
            if self.list.is_empty() {
                let message = format!("For {what}, add an AI provider under Accounts.");
                return refuse(Error::NotConnected, message);
            }
            return refuse(no_model(), format!("For {what}, pick a model."));
        };
        let Ok(account) = self.get(choice.account) else {
            let message = format!(
                "For {what}, the picked model\u{2019}s account is gone; pick another model."
            );
            return refuse(no_model(), message);
        };
        if !account.usable() {
            let name = account.provider().spec().display;
            let message = format!("For {what}, enter {name}\u{2019}s key again under Accounts.");
            return refuse(Error::NotConnected, message);
        }
        let model = &choice.model;
        if !account.settings.offered.contains(model) {
            let message = format!(
                "For {what}, {model} is no longer offered; tick it under Accounts or pick another model."
            );
            return refuse(no_model(), message);
        }
        account.check_limit(&month_of(now))?;
        Ok(Resolved {
            account: account.id(),
            model: model.clone(),
            client: account.client(),
            provider: account.provider(),
            address: account.address().to_string(),
        })
    }

    // ------------------------------------------------------------------------
    /// Whether the account `id` may still be called in the month of `now`:
    /// refused once it has reached its spend limit (`SpendLimit`), or if it
    /// is gone (`NotFound`).
    pub fn check_limit(&self, id: Id, now: DateTime) -> std::result::Result<(), Refusal> {
        let account = self.get(id).map_err(|error| Refusal {
            error,
            message: "The AI account is gone.".to_string(),
        })?;
        account.check_limit(&month_of(now))
    }

    // ------------------------------------------------------------------------
    /// Add what the clients of the account `id` recorded since the last time
    /// to this month, and save its spend file. A failed save keeps it in
    /// memory, for the next one.
    pub fn record_spend(&mut self, id: Id) -> Result<()> {
        let index = self.index(id)?;
        let account = &mut self.list[index];
        if !account.take_spend(&month_of(DateTime::now())) {
            return Ok(());
        }
        account.save_spend()
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
        record(&account, "gpt-5-mini", 0.5);
        account.take_spend("2026-10");
        account.save_spend().unwrap();

        account.disconnect(&dirs.secrets).unwrap();
        assert!(!dirs.key(id).exists(), "the key file is gone");
        assert!(dirs.folder.join("openai.spend.yaml").exists(), "spend kept");
        let text = fs::read_to_string(dirs.folder.join("openai.yaml")).unwrap();
        assert!(text.contains("disconnected:"), "{text}");
        let reloaded = dirs.load("openai.yaml");
        assert_eq!((reloaded.id(), reloaded.status()), (id, "disconnected"));

        reloaded.remove(&dirs.secrets).unwrap();
        let left: Vec<_> = fs::read_dir(&dirs.folder).unwrap().collect();
        assert!(left.is_empty(), "the account and spend files are gone");
    }

    // ------------------------------------------------------------------------
    /// Record a call to `model` costing `cost` (`None`: unpriced) through
    /// `account`'s client, as a worker does.
    fn record(account: &AiAccount, model: &str, cost: impl Into<Option<f64>>) {
        let usage = av_llm::Usage {
            input: 100,
            output: 20,
            ..Default::default()
        };
        let client = account.client();
        client
            .ledger()
            .record(account.provider(), model, &usage, cost.into());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn spending_is_kept_per_month_and_read_back() {
        let dirs = Dirs::new();
        fs::write(dirs.folder.join("openai.yaml"), OPENAI).unwrap();
        let mut account = dirs.load("openai.yaml");
        assert!(!account.take_spend("2026-09"), "nothing recorded yet");
        record(&account, "gpt-5-mini", 0.25);
        account.take_spend("2026-09");
        record(&account, "gpt-5-mini", 0.5);
        record(&account, "gpt-5-mini", 0.25);
        record(&account, "gpt-5-nano", None);
        assert!(account.take_spend("2026-10"));
        account.save_spend().unwrap();

        let text = fs::read_to_string(dirs.folder.join("openai.spend.yaml")).unwrap();
        let again = dirs.load("openai.yaml");
        let october = Spent {
            cost: 0.75,
            calls: 3,
            unpriced: 1,
        };
        assert!(
            text.starts_with(&format!("account: {}", account.id())),
            "{text}"
        );
        assert!(text.contains("cache-read: 0"), "{text}");
        assert_eq!(again.spent("2026-10"), october);
        assert_eq!(again.spent("2026-09").calls, 1);
        assert_eq!(again.spent("2026-08"), Spent::default());
        assert_eq!(again.spend["2026-10"]["gpt-5-mini"].input, 200);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_spend_file_of_another_account_is_not_taken_over() {
        let dirs = Dirs::new();
        fs::write(dirs.folder.join("openai.yaml"), OPENAI).unwrap();
        let other = Id::generate();
        let spend = format!("account: {other}\nmonths:\n  2026-10:\n    gpt-5:\n      calls: 7\n");
        fs::write(dirs.folder.join("openai.spend.yaml"), spend).unwrap();
        let account = dirs.load("openai.yaml");
        assert_eq!(account.spent("2026-10"), Spent::default());
        assert!(is_spend_file(&dirs.folder.join("openai.Spend.yaml")));
        assert!(!is_spend_file(&dirs.folder.join("openai.yaml")));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_use_is_recorded_into_this_month() {
        let dirs = Dirs::new();
        fs::write(dirs.folder.join("openai.yaml"), OPENAI).unwrap();
        let account = dirs.load("openai.yaml");
        let id = account.id();
        record(&account, "gpt-5-mini", 0.5);
        let mut accounts =
            AiAccounts::new(dirs.folder.clone(), dirs.secrets.clone(), vec![account]);

        accounts.record_spend(id).unwrap();
        let spent = &accounts.get(id).unwrap().meta().spent;
        assert_eq!((spent.calls, spent.cost), (1, 0.5));
        let reloaded = dirs.load("openai.yaml");
        assert_eq!(reloaded.meta().spent.calls, 1, "survives a restart");
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

    // ------------------------------------------------------------------------
    /// An unsaved account of `provider`, with a key if `key`, offering
    /// `offered`.
    fn account(provider: Provider, key: bool, offered: &[&str]) -> AiAccount {
        let mut account = AiAccount::new(Path::new("accounts"), provider);
        if key {
            account.key = Some("sk-0123456789abcdef".into());
        }
        account.settings.offered = offered.iter().map(|m| m.to_string()).collect();
        account
    }

    fn list(accounts: Vec<AiAccount>) -> AiAccounts {
        AiAccounts::new(PathBuf::new(), PathBuf::new(), accounts)
    }

    fn pick(account: &AiAccount, model: &str) -> ModelChoice {
        ModelChoice {
            account: account.id(),
            model: model.to_string(),
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_pick_goes_to_its_account_and_model() {
        let now = DateTime::now();
        let ollama = account(Provider::Ollama, false, &["qwen", "llama"]);
        let anthropic = account(Provider::Anthropic, true, &["claude"]);
        let (llama, claude) = (pick(&ollama, "llama"), pick(&anthropic, "claude"));
        let accounts = list(vec![ollama, anthropic]);

        let chat = accounts.choose(Some(&llama), "chats", now).unwrap();
        assert_eq!(
            (chat.account, chat.model.as_str(), chat.provider),
            (llama.account, "llama", Provider::Ollama),
            "not the first offered model: the one picked"
        );
        let ocr = accounts.choose(Some(&claude), "OCR", now).unwrap();
        assert_eq!(
            (ocr.account, ocr.model.as_str(), ocr.provider),
            (claude.account, "claude", Provider::Anthropic)
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_pick_that_does_not_work_is_refused_in_words_and_nothing_else_tried() {
        let refusal = |accounts: &AiAccounts, choice: Option<&ModelChoice>| {
            let refusal = accounts
                .choose(choice, "OCR", DateTime::now())
                .err()
                .unwrap();
            (refusal.error.code(), refusal.message)
        };
        let none = list(vec![]);
        assert_eq!(
            refusal(&none, None),
            (
                "not_connected",
                "For OCR, add an AI provider under Accounts.".to_string()
            )
        );
        let keyless = account(Provider::OpenAi, false, &["gpt-5"]);
        let ollama = account(Provider::Ollama, false, &["qwen"]);
        let (gpt, unticked) = (pick(&keyless, "gpt-5"), pick(&ollama, "llama"));
        let gone = ModelChoice {
            account: Id::generate(),
            model: "qwen".to_string(),
        };
        let accounts = list(vec![keyless, ollama]);
        assert_eq!(
            refusal(&accounts, None),
            ("invalid_argument", "For OCR, pick a model.".to_string()),
            "never the first offered model"
        );
        assert_eq!(
            refusal(&accounts, Some(&gpt)),
            (
                "not_connected",
                "For OCR, enter OpenAI\u{2019}s key again under Accounts.".to_string()
            )
        );
        assert_eq!(
            refusal(&accounts, Some(&gone)),
            (
                "invalid_argument",
                "For OCR, the picked model\u{2019}s account is gone; pick another model."
                    .to_string()
            )
        );
        assert_eq!(
            refusal(&accounts, Some(&unticked)),
            (
                "invalid_argument",
                "For OCR, llama is no longer offered; tick it under Accounts or pick another model."
                    .to_string()
            )
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_account_at_its_limit_this_month_is_refused_and_no_other_tried() {
        let mut openai = account(Provider::OpenAi, true, &["gpt-5"]);
        openai.settings.spend_limit = Some(10.0);
        let spend = ModelSpend {
            calls: 40,
            cost: 12.5,
            ..ModelSpend::default()
        };
        openai
            .spend
            .insert("2026-09".into(), [("gpt-5".to_string(), spend)].into());
        let (id, gpt) = (openai.id(), pick(&openai, "gpt-5"));
        let ollama = account(Provider::Ollama, false, &["qwen"]);
        let accounts = list(vec![openai, ollama]);
        let september = DateTime::from_iso8601("2026-09-30T23:00:00Z").unwrap();
        let october = DateTime::from_iso8601("2026-10-01T01:00:00Z").unwrap();

        let refusal = accounts
            .choose(Some(&gpt), "chats", september)
            .err()
            .unwrap();
        assert_eq!(refusal.error.code(), "spend_limit");
        assert_eq!(
            refusal.message,
            "OpenAI has reached its spend limit of $10.00 this month; raise it under Accounts."
        );
        let chat = accounts.choose(Some(&gpt), "chats", october).unwrap();
        assert_eq!(chat.account, id);
        assert!(accounts.check_limit(id, september).is_err());
        assert!(accounts.check_limit(id, october).is_ok());
        let gone = accounts.check_limit(Id::generate(), october).err().unwrap();
        assert_eq!(gone.error.code(), "not_found");
    }
}
