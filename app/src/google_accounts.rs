//! Google accounts (accounts.md#google-file): one YAML file per account in
//! the vault's `.avelor/accounts/`, its refresh token DPAPI-encrypted with the
//! app (on Linux, in a file only the user can read;
//! `config_files::SecretFile`); the token never enters the YAML or the
//! vault. `GoogleAccounts` holds the open vault's accounts; the rest of the
//! app reaches them only through it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use av_google::auth::{GoogleScope, ScopeAccess};
use av_google::calendar::CalendarListEntry;
use av_google::client::Client;
use av_google::gmail::MailFilter;
use av_utils::datetime::DateTime;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

use crate::ai_accounts::ModelChoice;
use crate::config::is_false;
use crate::config_files::{ConnectionStatus, SecretFile, remove_if_exists, write_yaml};

/// The extension of a Google account's secret, its refresh token.
const TOKEN: &str = "token";

// ----------------------------------------------------------------------------
/// A service's access level, as written in account files and UI messages;
/// the file lists only services that aren't `off`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Off,
    Read,
    Full,
}

impl Level {
    // ------------------------------------------------------------------------
    /// The name used in files and messages.
    pub fn name(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Read => "read",
            Level::Full => "full",
        }
    }
}

impl From<ScopeAccess> for Level {
    fn from(access: ScopeAccess) -> Self {
        match access {
            ScopeAccess::None => Level::Off,
            ScopeAccess::ReadOnly => Level::Read,
            ScopeAccess::Full => Level::Full,
        }
    }
}

impl From<Level> for ScopeAccess {
    fn from(level: Level) -> Self {
        match level {
            Level::Off => ScopeAccess::None,
            Level::Read => ScopeAccess::ReadOnly,
            Level::Full => ScopeAccess::Full,
        }
    }
}

// ----------------------------------------------------------------------------
/// Each service by its file/message name, with its access in `scope`.
fn services(scope: &GoogleScope) -> [(&'static str, ScopeAccess); 4] {
    [
        ("email", scope.gmail),
        ("calendar", scope.calendar),
        ("contacts", scope.contacts),
        ("tasks", scope.tasks),
    ]
}

// ----------------------------------------------------------------------------
fn service_mut<'a>(scope: &'a mut GoogleScope, service: &str) -> Option<&'a mut ScopeAccess> {
    Some(match service {
        "email" => &mut scope.gmail,
        "calendar" => &mut scope.calendar,
        "contacts" => &mut scope.contacts,
        "tasks" => &mut scope.tasks,
        _ => return None,
    })
}

// ----------------------------------------------------------------------------
/// `service: level` pairs (a file's `scopes`, or a UI message) as a scope;
/// unknown services are logged and ignored.
pub fn scope_from_levels<'a>(
    levels: impl IntoIterator<Item = (&'a String, &'a Level)>,
) -> GoogleScope {
    let mut scope = GoogleScope::default();
    for (service, level) in levels {
        match service_mut(&mut scope, service) {
            Some(access) => *access = (*level).into(),
            None => log::warn!("Ignoring unknown Google service `{service}`"),
        }
    }
    scope
}

// ----------------------------------------------------------------------------
/// Every service's level, for the UI.
pub fn levels_of(scope: &GoogleScope) -> BTreeMap<&'static str, Level> {
    services(scope)
        .into_iter()
        .map(|(service, access)| (service, access.into()))
        .collect()
}

// ----------------------------------------------------------------------------
/// A scope in account-file terms, for the log: `email: read, calendar: full`.
pub fn describe_scope(scope: &GoogleScope) -> String {
    let granted: Vec<String> = services(scope)
        .into_iter()
        .filter(|(_, access)| *access != ScopeAccess::None)
        .map(|(service, access)| format!("{service}: {}", Level::from(access).name()))
        .collect();
    if granted.is_empty() {
        "no access".to_string()
    } else {
        granted.join(", ")
    }
}

// ----------------------------------------------------------------------------
/// The account file's content. Unknown keys are kept, so hand-added
/// settings survive the app rewriting the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AccountFile {
    pub id: Id,
    pub google: GoogleAccount,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<DateTime>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

// ----------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct GoogleAccount {
    /// Unknown until the first (re)connect for hand-written files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    /// One `service: level` entry per granted service, e.g. `- email: read`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scopes: Vec<BTreeMap<String, Level>>,
    /// The calendars chosen as sources, in the order they were checked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calendars: Vec<CalendarSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected: Option<DateTime>,
    /// When the account was disconnected: it is meant to have no token.
    /// Without it, a missing token means a reconnect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disconnected: Option<DateTime>,
    /// The mail import's remembered settings; absent until the first fetch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail: Option<MailSettings>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

// ----------------------------------------------------------------------------
/// What the mail import remembers per account: the filter of the last
/// fetch, the preview's update mode, and when mail was last fetched (the
/// "Since last time" preset); and the account's auto-sync.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct MailSettings {
    #[serde(default)]
    pub filter: MailFilter,
    #[serde(default = "auto_update_default")]
    pub auto_update: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fetch: Option<DateTime>,
    /// Absent until auto-sync is first switched on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_sync: Option<AutoSync>,
    /// Fetch mail's analysis, remembered with the filter; absent: off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<Analysis>,
    /// Fetch mail's attachments, remembered with the filter; absent: off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<AttachmentSettings>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

impl Default for MailSettings {
    fn default() -> Self {
        Self {
            filter: MailFilter::default(),
            auto_update: auto_update_default(),
            last_fetch: None,
            auto_sync: None,
            analysis: None,
            attachments: None,
            rest: BTreeMap::new(),
        }
    }
}

// ----------------------------------------------------------------------------
/// How closely new mail is analysed (mail.md#analysis): Lite sorts it,
/// Medium also drafts a reply and what to check, as a chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnalysisLevel {
    Off,
    Lite,
    Medium,
}

// ----------------------------------------------------------------------------
/// A filter's mail analysis: its level, and the model it runs with
/// (accounts.md#model-choice), kept while the level is off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    pub level: AnalysisLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelChoice>,
}

impl Analysis {
    // ------------------------------------------------------------------------
    /// Whether new mail is analysed at all.
    pub fn is_on(&self) -> bool {
        self.level != AnalysisLevel::Off
    }
}

fn auto_update_default() -> bool {
    true
}

// ----------------------------------------------------------------------------
/// What a filter does with a mail's attached files (mail.md#attachments):
/// whether they are saved, which ones (`patterns`, as a watch has them),
/// up to what size, in MB, and whether they are read, with the model the
/// reading step picked (`models`, by step, as a watch has them), kept while
/// reading is off.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AttachmentSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default = "max_size_default")]
    pub max_size: u32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub read: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, ModelChoice>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

impl AttachmentSettings {
    // ------------------------------------------------------------------------
    /// The model `step` picked, if it has one.
    pub fn model(&self, step: &str) -> Option<&ModelChoice> {
        self.models.get(step)
    }
}

fn max_size_default() -> u32 {
    10
}

// ----------------------------------------------------------------------------
/// The account's mail auto-sync (`mail::auto_sync`): every 15 minutes, the
/// mail that matches `filter` and arrived since `since` is fetched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AutoSync {
    pub enabled: bool,
    /// Fetch mail's filter without its dates: the host sets `after` itself.
    #[serde(default)]
    pub filter: MailFilter,
    /// Mail from then on is fetched: when auto-sync was switched on, then
    /// the start of the last complete run.
    pub since: DateTime,
    /// The analysis of the mail it fetches; absent: off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis: Option<Analysis>,
    /// The attachments of the mail it fetches; absent: off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<AttachmentSettings>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

impl AutoSync {
    // ------------------------------------------------------------------------
    /// Off, fetching the mail from `since` on once it is switched on.
    pub fn new(since: DateTime) -> Self {
        Self {
            enabled: false,
            filter: MailFilter::default(),
            since,
            analysis: None,
            attachments: None,
            rest: BTreeMap::new(),
        }
    }
}

// ----------------------------------------------------------------------------
/// A Google calendar chosen as a source; name and color are kept for
/// showing it without asking Google.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CalendarSource {
    /// Google's calendarId.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Synced every 15 minutes (`calendar::auto_sync`); written only when
    /// on.
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto_sync: bool,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

impl GoogleAccount {
    // ------------------------------------------------------------------------
    pub fn scope(&self) -> GoogleScope {
        scope_from_levels(self.scopes.iter().flatten())
    }

    // ------------------------------------------------------------------------
    fn set_scope(&mut self, scope: &GoogleScope) {
        self.scopes = services(scope)
            .into_iter()
            .filter(|(_, access)| *access != ScopeAccess::None)
            .map(|(service, access)| BTreeMap::from([(service.to_string(), access.into())]))
            .collect();
    }
}

// ----------------------------------------------------------------------------
/// A configured account, and its client once its token has been decrypted.
pub struct Account {
    /// The account file.
    pub path: PathBuf,
    pub file: AccountFile,
    /// `None`: disconnected, or the token is missing or can't be decrypted
    /// (e.g. the vault was opened on another computer) — see `status`.
    pub client: Option<Arc<Client>>,
}

// ----------------------------------------------------------------------------
/// What the page gets per account (the `account` event).
#[derive(Serialize)]
pub struct AccountMeta<'a> {
    pub id: Id,
    pub kind: &'static str,
    pub email: Option<&'a str>,
    pub scope: BTreeMap<&'static str, Level>,
    pub connected: Option<DateTime>,
    pub last_sync: Option<DateTime>,
    pub status: ConnectionStatus,
    pub calendars: Vec<CalendarMeta<'a>>,
    pub mail: Option<MailMeta<'a>>,
}

// ----------------------------------------------------------------------------
/// A calendar source as the page gets it (see `CalendarSource`).
#[derive(Debug, PartialEq, Serialize)]
pub struct CalendarMeta<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub color: Option<&'a str>,
    pub auto_sync: bool,
}

impl<'a> From<&'a CalendarSource> for CalendarMeta<'a> {
    fn from(source: &'a CalendarSource) -> Self {
        Self {
            id: &source.id,
            name: &source.name,
            color: source.color.as_deref(),
            auto_sync: source.auto_sync,
        }
    }
}

// ----------------------------------------------------------------------------
/// The mail settings as the page gets them (see `MailSettings`).
#[derive(Serialize)]
pub struct MailMeta<'a> {
    pub filter: &'a MailFilter,
    pub auto_update: bool,
    pub last_fetch: Option<DateTime>,
    pub auto_sync: Option<&'a AutoSync>,
    pub analysis: Option<&'a Analysis>,
    pub attachments: Option<&'a AttachmentSettings>,
}

impl Account {
    // ------------------------------------------------------------------------
    /// A newly connected Google account in `folder`, named after its email.
    fn new(folder: &Path, email: &str, client: Client) -> Self {
        let mut google = GoogleAccount {
            email: Some(email.to_string()),
            scopes: Vec::new(),
            calendars: Vec::new(),
            connected: Some(DateTime::now()),
            disconnected: None,
            mail: None,
            rest: BTreeMap::new(),
        };
        google.set_scope(client.scope());
        Self {
            path: folder.join(format!("google_{}.yaml", file_safe(email))),
            file: AccountFile {
                id: Id::generate(),
                google,
                last_sync: None,
                rest: BTreeMap::new(),
            },
            client: Some(Arc::new(client)),
        }
    }

    // ------------------------------------------------------------------------
    pub fn id(&self) -> Id {
        self.file.id
    }

    // ------------------------------------------------------------------------
    pub fn email(&self) -> Option<&str> {
        self.file.google.email.as_deref()
    }

    // ------------------------------------------------------------------------
    /// Take a fresh consent's client (reconnect, or connecting a
    /// disconnected account again): its email and granted scope replace
    /// the recorded ones; id and `connected` stay.
    fn reconnect(&mut self, email: &str, client: Client) {
        let google = &mut self.file.google;
        google.email = Some(email.to_string());
        google.set_scope(client.scope());
        google.connected.get_or_insert_with(DateTime::now);
        google.disconnected = None;
        self.client = Some(Arc::new(client));
    }

    // ------------------------------------------------------------------------
    pub fn status(&self) -> ConnectionStatus {
        let disconnected = self.file.google.disconnected.is_some();
        ConnectionStatus::of(self.client.is_some(), disconnected)
    }

    // ------------------------------------------------------------------------
    pub fn meta(&self) -> AccountMeta<'_> {
        // A live client knows what was granted; the file is the fallback.
        let scope = match &self.client {
            Some(client) => client.scope().clone(),
            None => self.file.google.scope(),
        };
        AccountMeta {
            id: self.id(),
            kind: "google",
            email: self.email(),
            scope: levels_of(&scope),
            connected: self.file.google.connected,
            last_sync: self.file.last_sync,
            status: self.status(),
            calendars: self.file.google.calendars.iter().map(Into::into).collect(),
            mail: self.file.google.mail.as_ref().map(|mail| MailMeta {
                filter: &mail.filter,
                auto_update: mail.auto_update,
                last_fetch: mail.last_fetch,
                auto_sync: mail.auto_sync.as_ref(),
                analysis: mail.analysis.as_ref(),
                attachments: mail.attachments.as_ref(),
            }),
        }
    }

    // ------------------------------------------------------------------------
    /// Check (`selected`) or uncheck a calendar as a source; whether that
    /// changed anything. Unchecking only stops future syncs: what was
    /// synced stays in the vault.
    pub fn set_calendar(&mut self, source: CalendarSource, selected: bool) -> bool {
        let calendars = &mut self.file.google.calendars;
        let index = calendars.iter().position(|c| c.id == source.id);
        match (index, selected) {
            (None, true) => calendars.push(source),
            (Some(i), false) => {
                calendars.remove(i);
            }
            _ => return false,
        }
        true
    }

    // ------------------------------------------------------------------------
    /// Switch the auto-sync of the checked calendar `id` on or off; whether
    /// that changed anything (false for a calendar that isn't checked).
    pub fn set_calendar_sync(&mut self, id: &str, enabled: bool) -> bool {
        let calendars = &mut self.file.google.calendars;
        match calendars.iter_mut().find(|c| c.id == id) {
            Some(source) if source.auto_sync != enabled => {
                source.auto_sync = enabled;
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------------------
    /// Take names and colors of checked calendars from a fresh calendar
    /// list; whether anything changed. Calendars missing from the list stay
    /// checked (a hiccup on Google's side mustn't drop the selection).
    pub fn refresh_calendars(&mut self, list: &[CalendarListEntry]) -> bool {
        let mut changed = false;
        for source in &mut self.file.google.calendars {
            let Some(entry) = list.iter().find(|e| e.id == source.id) else {
                continue;
            };
            let color = Some(entry.background_color.clone()).filter(|c| !c.is_empty());
            if source.name != entry.summary || source.color != color {
                source.name = entry.summary.clone();
                source.color = color;
                changed = true;
            }
        }
        changed
    }

    // ------------------------------------------------------------------------
    fn secret(&self, secrets: &Path) -> SecretFile {
        SecretFile::of(secrets, self.id(), TOKEN)
    }

    // ------------------------------------------------------------------------
    /// Write the token (encrypted) first, then the account file, so a
    /// crash in between never leaves a connected file without its token.
    fn save(&self, secrets: &Path) -> Result<()> {
        if let Some(client) = &self.client {
            self.secret(secrets).write(client.refresh_token())?;
        }
        write_yaml(&self.path, &self.file)?;
        log::info!("Saved account {}", self.path.display());
        Ok(())
    }

    // ------------------------------------------------------------------------
    /// Forget the (already revoked) token but keep the account, marked
    /// `disconnected`, so connecting the same Google account again keeps
    /// its id. Nothing changes if writing the file fails.
    fn disconnect(&mut self, secrets: &Path) -> Result<()> {
        let mut file = self.file.clone();
        file.google.disconnected = Some(DateTime::now());
        write_yaml(&self.path, &file)?;
        self.file = file;
        self.client = None;
        self.secret(secrets).remove()?;
        log::info!("Disconnected account {}", self.path.display());
        Ok(())
    }

    // ------------------------------------------------------------------------
    /// Delete the account file, and its token if there is one.
    fn remove(&self, secrets: &Path) -> Result<()> {
        remove_if_exists(&self.path)?;
        self.secret(secrets).remove()?;
        log::info!("Removed account {}", self.path.display());
        Ok(())
    }
}

// ----------------------------------------------------------------------------
/// `s` usable in a file name (Windows forbids `<>:"/\|?*`).
fn file_safe(s: &str) -> String {
    s.chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

// ----------------------------------------------------------------------------
/// The Google account in `path` (`value`, its parsed content), its token in
/// `secrets`.
pub(crate) fn load(path: &Path, value: Value, secrets: &Path) -> Result<Account> {
    let file: AccountFile = serde_yaml_ng::from_value(value)?;
    let mut account = Account {
        path: path.to_path_buf(),
        file,
        client: None,
    };
    let secret = account.secret(secrets);
    match secret.read() {
        Ok(token) => {
            let client = Client::new(token, account.file.google.scope());
            account.client = Some(Arc::new(client));
        }
        Err(e) if account.file.google.disconnected.is_none() => {
            log::warn!("Account {} needs reconnecting: {e}", path.display());
        }
        Err(_) => {}
    }
    log::info!(
        "Loaded account {} ({}, {}): {}",
        path.display(),
        account.email().unwrap_or("no email"),
        account.status().name(),
        describe_scope(&account.file.google.scope())
    );
    Ok(account)
}

// ----------------------------------------------------------------------------
/// The open vault's Google accounts: their files in `folder` (the vault's
/// `.avelor/accounts`), their tokens in `secrets` (the config folder). Without
/// a vault it is empty, and nothing can be added.
#[derive(Default)]
pub struct GoogleAccounts {
    folder: PathBuf,
    secrets: PathBuf,
    list: Vec<Account>,
}

impl GoogleAccounts {
    // ------------------------------------------------------------------------
    pub fn new(folder: PathBuf, secrets: PathBuf, list: Vec<Account>) -> Self {
        Self {
            folder,
            secrets,
            list,
        }
    }

    // ------------------------------------------------------------------------
    pub fn iter(&self) -> std::slice::Iter<'_, Account> {
        self.list.iter()
    }

    // ------------------------------------------------------------------------
    /// The account `id`; `NotFound` if there is none.
    pub fn get(&self, id: Id) -> Result<&Account> {
        self.list
            .iter()
            .find(|a| a.id() == id)
            .ok_or(Error::NotFound)
    }

    // ------------------------------------------------------------------------
    /// The client of the account `id`; `NotFound` if there is no such
    /// account, `NotConnected` if it has no token.
    pub fn connected(&self, id: Id) -> Result<Arc<Client>> {
        self.get(id)?.client.clone().ok_or(Error::NotConnected)
    }

    // ------------------------------------------------------------------------
    fn index(&self, id: Id) -> Result<usize> {
        self.list
            .iter()
            .position(|a| a.id() == id)
            .ok_or(Error::NotFound)
    }

    // ------------------------------------------------------------------------
    /// Change the account `id` with `f`, and save it if `f` says it changed
    /// anything; a failed save leaves the account as it was. Whether it
    /// changed.
    pub fn update(&mut self, id: Id, f: impl FnOnce(&mut Account) -> bool) -> Result<bool> {
        let index = self.index(id)?;
        let account = &mut self.list[index];
        let before = account.file.clone();
        let changed = f(account);
        if changed {
            if let Err(e) = account.save(&self.secrets) {
                account.file = before;
                return Err(e);
            }
        }
        Ok(changed)
    }

    // ------------------------------------------------------------------------
    /// Store a fresh consent of `email`: for the account `target` (a
    /// reconnect, which must sign in as the same email), else for the
    /// account of that email, else as a new account; its id. A failed save
    /// leaves the accounts as they were.
    pub fn connect(&mut self, target: Option<Id>, email: &str, client: Client) -> Result<Id> {
        let same_email = |account: &Account| {
            account
                .email()
                .is_some_and(|known| known.eq_ignore_ascii_case(email))
        };
        let index = match target {
            Some(id) => Some(self.index(id)?),
            None => self.list.iter().position(same_email),
        };
        let Some(index) = index else {
            if self.folder.as_os_str().is_empty() {
                return Err(Error::InvalidPath {
                    path: PathBuf::new(),
                }); // no vault is open
            }
            let account = Account::new(&self.folder, email, client);
            account.save(&self.secrets)?;
            let id = account.id();
            self.list.push(account);
            return Ok(id);
        };

        let account = &mut self.list[index];
        if account.email().is_some() && !same_email(account) {
            return Err(Error::WrongAccount {
                email: email.to_string(),
            });
        }
        let before = (account.file.clone(), account.client.take());
        account.reconnect(email, client);
        if let Err(e) = account.save(&self.secrets) {
            (account.file, account.client) = before;
            return Err(e);
        }
        Ok(account.id())
    }

    // ------------------------------------------------------------------------
    /// Disconnect the account `id` (its token already revoked): marked
    /// `disconnected`, its token deleted.
    pub fn disconnect(&mut self, id: Id) -> Result<()> {
        let index = self.index(id)?;
        self.list[index].disconnect(&self.secrets)
    }

    // ------------------------------------------------------------------------
    /// Forget the account `id`, which must not be connected: its file and
    /// token are deleted.
    pub fn remove(&mut self, id: Id) -> Result<()> {
        let index = self.index(id)?;
        let account = &self.list[index];
        if account.client.is_some() {
            let what = "removing a connected account (disconnect it first)".to_string();
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
    use av_utils::secret;
    use std::fs;

    const GOOGLE: &str = "id: 01a0e1cc-a24c-7490-82fe-e75669985b24
google:
  email: someone@example.com
  scopes:
    - email: read
    - calendar: full
  connected: 2026-09-27T10:14:00Z
  custom: kept
last-sync: 2026-09-27T10:30:00Z
";

    struct Dirs {
        root: PathBuf,
        folder: PathBuf,
        secrets: PathBuf,
    }

    impl Dirs {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("av-google-test-{}", Id::generate()));
            let (folder, secrets) = (root.join("accounts"), root.join("config"));
            fs::create_dir_all(&folder).unwrap();
            fs::create_dir_all(&secrets).unwrap();
            Self {
                root,
                folder,
                secrets,
            }
        }

        /// The account file `name` with `text`, and its token if given.
        fn account(&self, name: &str, text: &str, token: Option<&str>) {
            fs::write(self.folder.join(name), text).unwrap();
            if let Some(token) = token {
                let id = text.lines().next().unwrap().trim_start_matches("id: ");
                let path = self.secrets.join(format!("{id}.token"));
                fs::write(path, secret::protect(token.as_bytes()).unwrap()).unwrap();
            }
        }

        fn load(&self, name: &str) -> Account {
            let path = self.folder.join(name);
            let value = serde_yaml_ng::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            load(&path, value, &self.secrets).unwrap()
        }

        fn accounts(&self, names: &[&str]) -> GoogleAccounts {
            let list = names.iter().map(|name| self.load(name)).collect();
            GoogleAccounts::new(self.folder.clone(), self.secrets.clone(), list)
        }

        fn token(&self, id: Id) -> PathBuf {
            self.secrets.join(format!("{id}.token"))
        }
    }

    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn account_file_maps_scope_levels() {
        let file: AccountFile = serde_yaml_ng::from_str(GOOGLE).unwrap();
        let scope = file.google.scope();
        assert_eq!(scope.gmail, ScopeAccess::ReadOnly);
        assert_eq!(scope.calendar, ScopeAccess::Full);
        assert_eq!(scope.contacts, ScopeAccess::None);
        assert_eq!(file.google.email.as_deref(), Some("someone@example.com"));
        assert!(file.last_sync.is_some());
        assert_eq!(file.google.mail, None);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn mail_settings_round_trip_in_the_account_file() {
        let mail = "  mail:\n    filter:\n      labels: [Umzug]\n      since: week\n    last-fetch: 2026-09-22T12:10:00Z\n    attachments:\n      enabled: true\n      patterns: ['*.pdf']\n      read: true\n      models:\n        read:\n          account: 01a0e1cc-a24c-7490-82fe-e75669985b42\n          model: gemini-3.8-flash\n      custom: kept\n";
        let yaml = GOOGLE.replacen("  custom: kept\n", &format!("  custom: kept\n{mail}"), 1);
        let file: AccountFile = serde_yaml_ng::from_str(&yaml).unwrap();
        let mail = file.google.mail.as_ref().unwrap();
        assert_eq!(mail.filter.labels, ["Umzug"]);
        assert!(mail.auto_update, "auto-update defaults to on");
        assert_eq!(
            mail.last_fetch.map(|d| d.as_iso8601()).as_deref(),
            Some("2026-09-22T12:10:00Z")
        );
        let attachments = mail.attachments.as_ref().unwrap();
        assert!(attachments.enabled && attachments.patterns == ["*.pdf"]);
        assert_eq!(attachments.max_size, 10, "10 MB unless set");
        let read = attachments.model(crate::watches::READ_STEP).unwrap();
        assert!(attachments.read && read.model == "gemini-3.8-flash");
        let text = serde_yaml_ng::to_string(&file).unwrap();
        assert!(
            text.contains("max-size: 10\n      read: true\n      models:\n        read:\n"),
            "{text}"
        );
        assert!(
            text.contains("gemini-3.8-flash\n      custom: kept\n"),
            "{text}"
        );
        let again: AccountFile =
            serde_yaml_ng::from_str(&serde_yaml_ng::to_string(&file).unwrap()).unwrap();
        assert_eq!(again.google.mail, file.google.mail);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_settings_keep_the_keys_the_app_doesnt_know() {
        let calendars = "  calendars:\n    - id: c\n      name: C\n      custom: kept\n";
        let sync = "    auto-sync:\n      enabled: true\n      since: 2026-09-22T12:10:00Z\n      custom: kept\n";
        let mail = format!("  mail:\n    custom: kept\n{sync}");
        let yaml = GOOGLE.replacen(
            "  custom: kept\n",
            &format!("  custom: kept\n{calendars}{mail}"),
            1,
        );
        let file: AccountFile = serde_yaml_ng::from_str(&yaml).unwrap();
        let text = serde_yaml_ng::to_string(&file).unwrap();
        assert_eq!(text.matches("custom: kept").count(), 4, "{text}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_status_follows_the_token_and_the_disconnected_mark() {
        let dirs = Dirs::new();
        dirs.account("a.yaml", GOOGLE, Some("1//a"));
        let b = GOOGLE.replace("5b24", "5b25");
        dirs.account("b.yaml", &b, None);
        let c = GOOGLE
            .replace("5b24", "5b26")
            .replace("  custom", "  disconnected: 2026-10-01T08:00:00Z\n  custom");
        dirs.account("c.yaml", &c, None);

        let (a, b, c) = (
            dirs.load("a.yaml"),
            dirs.load("b.yaml"),
            dirs.load("c.yaml"),
        );
        assert_eq!(a.status(), ConnectionStatus::Connected);
        assert_eq!(a.client.as_ref().unwrap().refresh_token(), "1//a");
        assert_eq!(b.status(), ConnectionStatus::Reconnect, "no token, no mark");
        assert_eq!(c.status(), ConnectionStatus::Disconnected);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn disconnect_marks_the_account_and_connecting_again_keeps_its_id() {
        let dirs = Dirs::new();
        dirs.account("google_a.yaml", GOOGLE, Some("1//old"));
        let mut accounts = dirs.accounts(&["google_a.yaml"]);
        let id = accounts.iter().next().unwrap().id();

        accounts.disconnect(id).unwrap();
        let status = accounts.get(id).unwrap().status();
        assert_eq!(status, ConnectionStatus::Disconnected);
        assert!(!dirs.token(id).exists(), "the token is gone");
        let text = fs::read_to_string(dirs.folder.join("google_a.yaml")).unwrap();
        assert!(text.contains("disconnected:") && text.contains("custom: kept"));
        let mut accounts = dirs.accounts(&["google_a.yaml"]);
        assert_eq!(
            accounts.get(id).unwrap().status(),
            ConnectionStatus::Disconnected
        );

        let client = Client::new("1//new".into(), GoogleScope::default());
        let connected = accounts
            .connect(None, "Someone@Example.com", client)
            .unwrap();
        assert_eq!(connected, id, "the same email continues the account");
        let again = dirs.load("google_a.yaml");
        assert_eq!(again.status(), ConnectionStatus::Connected);
        assert_eq!(again.client.as_ref().unwrap().refresh_token(), "1//new");
        assert!(again.file.google.disconnected.is_none());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_reconnect_must_sign_in_as_the_same_email() {
        let dirs = Dirs::new();
        dirs.account("google_a.yaml", GOOGLE, Some("1//a"));
        let mut accounts = dirs.accounts(&["google_a.yaml"]);
        let id = accounts.iter().next().unwrap().id();

        let client = Client::new("1//b".into(), GoogleScope::default());
        let err = accounts.connect(Some(id), "other@example.com", client);
        assert!(matches!(err, Err(Error::WrongAccount { email }) if email == "other@example.com"));
        let token = accounts.connected(id).unwrap().refresh_token().to_string();
        assert_eq!(token, "1//a", "the account is as it was");
        let client = Client::new("1//c".into(), GoogleScope::default());
        let unknown = accounts.connect(Some(Id::generate()), "x@example.com", client);
        assert!(matches!(unknown, Err(Error::NotFound)));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn new_accounts_are_named_after_their_email_and_need_a_vault() {
        let dirs = Dirs::new();
        let mut accounts = dirs.accounts(&[]);
        let client = Client::new("1//t".into(), GoogleScope::default());
        let id = accounts.connect(None, "a/b@example.com", client).unwrap();
        let account = accounts.get(id).unwrap();
        let path = dirs.folder.join("google_a_b@example.com.yaml");
        assert_eq!(account.path, path);
        assert!(path.exists() && dirs.token(id).exists());
        assert!(account.file.google.connected.is_some());

        let client = Client::new("1//t".into(), GoogleScope::default());
        let none = GoogleAccounts::default().connect(None, "c@example.com", client);
        assert!(matches!(none, Err(Error::InvalidPath { .. })));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn lookups_tell_unknown_from_not_connected() {
        let dirs = Dirs::new();
        dirs.account("a.yaml", GOOGLE, None);
        let accounts = dirs.accounts(&["a.yaml"]);
        let id = accounts.iter().next().unwrap().id();
        assert!(matches!(accounts.connected(id), Err(Error::NotConnected)));
        assert!(matches!(accounts.get(Id::generate()), Err(Error::NotFound)));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn remove_refuses_a_connected_account_and_deletes_both_files() {
        let dirs = Dirs::new();
        dirs.account("a.yaml", GOOGLE, Some("1//a"));
        let mut accounts = dirs.accounts(&["a.yaml"]);
        let id = accounts.iter().next().unwrap().id();
        assert!(matches!(
            accounts.remove(id),
            Err(Error::Unsupported { .. })
        ));

        accounts.disconnect(id).unwrap();
        accounts.remove(id).unwrap();
        assert!(accounts.iter().next().is_none());
        let left: Vec<_> = fs::read_dir(&dirs.folder).unwrap().collect();
        assert!(left.is_empty() && !dirs.token(id).exists());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn scopes_are_described_in_file_terms() {
        let scope = GoogleScope {
            gmail: ScopeAccess::ReadOnly,
            calendar: ScopeAccess::Full,
            ..GoogleScope::default()
        };
        assert_eq!(describe_scope(&scope), "email: read, calendar: full");
        assert_eq!(describe_scope(&GoogleScope::default()), "no access");
    }

    // ------------------------------------------------------------------------
    fn calendar(id: &str, name: &str) -> CalendarSource {
        CalendarSource {
            id: id.into(),
            name: name.into(),
            color: Some("#9fe1e7".into()),
            auto_sync: false,
            rest: BTreeMap::new(),
        }
    }

    // ------------------------------------------------------------------------
    #[test]
    fn calendar_sources_are_saved_and_unchecked() {
        let dirs = Dirs::new();
        dirs.account("google_a.yaml", GOOGLE, Some("1//t"));
        let mut accounts = dirs.accounts(&["google_a.yaml"]);
        let id = accounts.iter().next().unwrap().id();

        let check = |accounts: &mut GoogleAccounts, source: CalendarSource, on: bool| {
            accounts.update(id, |a| a.set_calendar(source, on)).unwrap()
        };
        assert!(check(&mut accounts, calendar("me@example.com", "Me"), true));
        assert!(check(&mut accounts, calendar("fam@group", "Family"), true));
        assert!(
            !check(&mut accounts, calendar("fam@group", "Family"), true),
            "already checked"
        );
        assert!(
            !check(&mut accounts, calendar("gone@group", "Gone"), false),
            "not checked"
        );

        let text = fs::read_to_string(dirs.folder.join("google_a.yaml")).unwrap();
        let saved = dirs.load("google_a.yaml").file.google.calendars;
        assert!(check(
            &mut accounts,
            calendar("me@example.com", "Me"),
            false
        ));

        assert!(
            text.contains("calendars:") && text.contains("custom: kept"),
            "{text}"
        );
        let both = [
            calendar("me@example.com", "Me"),
            calendar("fam@group", "Family"),
        ];
        assert_eq!(saved, both);
        let account = accounts.get(id).unwrap();
        let family = calendar("fam@group", "Family");
        assert_eq!(account.meta().calendars, [CalendarMeta::from(&family)]);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_calendars_auto_sync_is_written_only_while_on() {
        let dirs = Dirs::new();
        dirs.account("google_a.yaml", GOOGLE, Some("1//t"));
        let mut accounts = dirs.accounts(&["google_a.yaml"]);
        let id = accounts.iter().next().unwrap().id();
        let mut set = |f: &dyn Fn(&mut Account) -> bool| accounts.update(id, f).unwrap();
        assert!(set(
            &|a| a.set_calendar(calendar("fam@group", "Family"), true)
        ));
        assert!(
            !set(&|a| a.set_calendar_sync("other@group", true)),
            "not checked"
        );
        assert!(set(&|a| a.set_calendar_sync("fam@group", true)));
        assert!(
            !set(&|a| a.set_calendar_sync("fam@group", true)),
            "on already"
        );

        let text = fs::read_to_string(dirs.folder.join("google_a.yaml")).unwrap();
        assert!(text.contains("auto-sync: true"), "{text}");
        let saved = dirs.load("google_a.yaml").file.google.calendars;
        assert!(saved[0].auto_sync);
        assert!(accounts.get(id).unwrap().meta().calendars[0].auto_sync);

        let off = accounts.update(id, |a| a.set_calendar_sync("fam@group", false));
        assert!(off.unwrap());
        let text = fs::read_to_string(dirs.folder.join("google_a.yaml")).unwrap();
        assert!(!text.contains("auto-sync"), "{text}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_failed_save_leaves_the_account_as_it_was() {
        let dirs = Dirs::new();
        dirs.account("a.yaml", GOOGLE, Some("1//t"));
        let mut accounts = dirs.accounts(&["a.yaml"]);
        let id = accounts.iter().next().unwrap().id();
        // A folder in the way of the temporary file makes the write fail.
        let path = dirs.folder.join("a.yaml");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();

        let result = accounts.update(id, |a| a.set_calendar(calendar("x", "X"), true));
        assert!(result.is_err());
        assert!(accounts.get(id).unwrap().file.google.calendars.is_empty());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn refreshing_calendars_renames_and_keeps_missing_ones() {
        let client = Client::new("1//t".into(), GoogleScope::default());
        let mut account = Account::new(Path::new("cfg"), "a@example.com", client);
        account.set_calendar(calendar("me@example.com", "Me"), true);
        account.set_calendar(calendar("gone@group", "Gone"), true);

        let entry = |id: &str, summary: &str, color: &str| CalendarListEntry {
            id: id.into(),
            summary: summary.into(),
            background_color: color.into(),
            ..CalendarListEntry::default()
        };
        let list = [
            entry("me@example.com", "Me, renamed", ""),
            entry("other@group", "Other", "#fff"),
        ];
        assert!(account.refresh_calendars(&list));
        assert!(
            !account.refresh_calendars(&list),
            "nothing new the second time"
        );

        let calendars = &account.file.google.calendars;
        assert_eq!(
            calendars.len(),
            2,
            "the missing calendar stays, the unchecked one isn't added"
        );
        assert_eq!(
            (calendars[0].name.as_str(), calendars[0].color.as_deref()),
            ("Me, renamed", None)
        );
        assert_eq!(calendars[1], calendar("gone@group", "Gone"));
    }
}
