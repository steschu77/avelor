//! The vault's own settings (vault.md#vault-config): `.avelor/vault.yaml`,
//! beside the vault's accounts, read when the vault opens. The model the
//! assistant answers with (accounts.md#model-choice). Keys the app doesn't
//! know are kept.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use av_utils::error::{Error, Result};
use av_vault::schema::layout;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

use crate::ai_accounts::ModelChoice;
use crate::app::App;
use crate::config_files::write_yaml;
use crate::protocol::Req;

// ----------------------------------------------------------------------------
/// The file's content.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct SettingsFile {
    /// The model the assistant answers with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chat_model: Option<ModelChoice>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

// ----------------------------------------------------------------------------
/// The open vault's settings. Without a vault they are the defaults, and
/// can't be changed.
#[derive(Default)]
pub struct VaultSettings {
    /// The open vault's `.avelor/vault.yaml`; empty without a vault.
    path: PathBuf,
    file: SettingsFile,
}

// ----------------------------------------------------------------------------
/// What the page gets (the `vault_settings` event).
#[derive(Serialize)]
struct SettingsMeta<'a> {
    chat_model: Option<&'a ModelChoice>,
}

impl VaultSettings {
    // ------------------------------------------------------------------------
    /// The settings in `path`. A missing file is the defaults; so is one
    /// that doesn't read (logged), which stays as it is until the next save.
    fn load(path: PathBuf) -> Self {
        let file = match fs::read_to_string(&path) {
            Ok(text) => serde_yaml_ng::from_str::<Option<SettingsFile>>(&text)
                .unwrap_or_else(|e| {
                    log::warn!("Could not read {}: {e}", path.display());
                    None
                })
                .unwrap_or_default(),
            Err(e) if e.kind() == ErrorKind::NotFound => SettingsFile::default(),
            Err(e) => {
                log::warn!("Could not read {}: {e}", path.display());
                SettingsFile::default()
            }
        };
        Self { path, file }
    }

    // ------------------------------------------------------------------------
    pub fn chat_model(&self) -> Option<&ModelChoice> {
        self.file.chat_model.as_ref()
    }

    // ------------------------------------------------------------------------
    fn meta(&self) -> SettingsMeta<'_> {
        SettingsMeta {
            chat_model: self.chat_model(),
        }
    }

    // ------------------------------------------------------------------------
    /// Change the settings with `f` and save them; `InvalidPath` without a
    /// vault. A failed save leaves them as they were.
    fn update(&mut self, f: impl FnOnce(&mut SettingsFile)) -> Result<()> {
        if self.path.as_os_str().is_empty() {
            return Err(Error::InvalidPath {
                path: PathBuf::new(),
            });
        }
        let mut file = self.file.clone();
        f(&mut file);
        write_yaml(&self.path, &file)?;
        self.file = file;
        Ok(())
    }
}

// ----------------------------------------------------------------------------
/// The vault `root` opens: its settings replace the ones before, and the page
/// gets them (`vault_settings`).
pub(crate) fn open(app: &mut App, root: &Path) {
    app.settings = VaultSettings::load(root.join(layout::VAULT_SETTINGS));
    post(app);
}

// ----------------------------------------------------------------------------
/// The `vault_settings` event.
fn post(app: &App) {
    app.post_event("vault_settings", app.settings.meta());
}

// ----------------------------------------------------------------------------
/// `set_vault_settings`' arguments; only the fields given change.
#[derive(Deserialize)]
pub(crate) struct SettingsChange {
    #[serde(default)]
    chat_model: Option<ModelChoice>,
}

// ----------------------------------------------------------------------------
/// `set_vault_settings {chat_model?}`: change the open vault's settings.
/// Answered after the `vault_settings` event; nothing changes if saving
/// fails. The model is checked when it is used (`AiAccounts::choose`).
pub fn set_vault_settings(app: &mut App, req: Req, change: SettingsChange) {
    let updated = app.settings.update(|file| {
        if let Some(model) = change.chat_model {
            file.chat_model = Some(model);
        }
    });
    match updated {
        Ok(()) => {
            log::info!("Vault settings saved");
            post(app);
            req.ok(());
        }
        Err(e @ Error::InvalidPath { .. }) => req.err(e),
        Err(e) => {
            log::warn!("Could not save the vault\u{2019}s settings: {e}");
            req.fail(e, "Could not save the vault\u{2019}s settings.");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::{Harness, parse, shape};
    use serde_json::{Value, json};
    use std::fs;

    const OLLAMA: &str = "01a0e1cc-a24c-7490-82fe-e75669985b40";

    // ------------------------------------------------------------------------
    #[test]
    fn a_vault_opens_with_its_settings() {
        let settings = format!("chat-model:\n  account: {OLLAMA}\n  model: qwen\nrename: false\n");
        let mut h = Harness::new("settings-open");
        let root = h.vault();
        fs::create_dir_all(root.join(".avelor")).unwrap();
        fs::write(root.join(".avelor/vault.yaml"), settings).unwrap();
        crate::api::switch_to(&mut h.app, root);
        h.wait_scanned();
        let sent = parse(&h.take());

        let event = sent.iter().find(|m| m["ev"] == "vault_settings").unwrap();
        let model = json!({ "account": OLLAMA, "model": "qwen" });
        assert_eq!(event["data"]["chat_model"], model);
        assert_eq!(h.app.settings.chat_model().unwrap().model, "qwen");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_change_is_saved_and_pushed_and_unknown_keys_are_kept() {
        let mut h =
            Harness::with_vault("settings-set", &[(".avelor/vault.yaml", "rename: false\n")]);
        let model = json!({ "account": OLLAMA, "model": "llama" });
        let sent = h.call("set_vault_settings", json!({ "chat_model": model }));

        assert_eq!(shape(&sent), ["vault_settings", "reply"]);
        assert_eq!(sent[0]["data"]["chat_model"], model);
        assert_eq!(sent[1]["ok"], Value::Null);
        let text = fs::read_to_string(h.vault().join(".avelor/vault.yaml")).unwrap();
        assert!(
            text.contains("chat-model:") && text.contains("model: llama"),
            "{text}"
        );
        assert!(text.contains("rename: false"), "{text}");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn without_a_vault_nothing_is_saved() {
        let mut h = Harness::new("settings-no-vault");
        let model = json!({ "account": OLLAMA, "model": "llama" });
        let sent = h.call("set_vault_settings", json!({ "chat_model": model }));
        assert_eq!(sent[0]["err"]["code"], "invalid_path");
    }
}
