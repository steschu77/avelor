//! Accounts (accounts.md): their files in the open vault's
//! `.avelor/accounts/`, read in one pass when a vault opens; what the page gets
//! about them (`accounts`, `account`, `account_removed`); and disconnect and
//! remove. AI accounts: `ai_accounts`, `ai`.

use std::fs;
use std::path::Path;

use av_utils::id::Id;
use av_vault::schema::layout;
use serde::Serialize;
use serde_json::json;
use serde_yaml_ng::Value;

use crate::ai;
use crate::ai_accounts::{self, AiAccount, AiAccountMeta, AiAccounts};
use crate::app::App;
use crate::config_files::load_config_files;
use crate::protocol::Req;

// ----------------------------------------------------------------------------
/// The accounts in `folder`, their secrets in `secrets`: every `*.yaml`,
/// handed to the loader of its top-level key (a provider's). Other and
/// broken files are logged and skipped.
pub(crate) fn load(folder: &Path, secrets: &Path) -> Vec<AiAccount> {
    load_config_files(folder, "account", |path| {
        let Value::Mapping(map) = serde_yaml_ng::from_str(&fs::read_to_string(path)?)? else {
            log::warn!("Skipping {}: not an account", path.display());
            return Ok(None);
        };
        match ai_accounts::kind_of(&map) {
            Some(kind) => {
                let account = ai_accounts::load(path, map, kind, secrets)?;
                Ok(Some(account))
            }
            None => {
                log::warn!("Skipping {}: not an account", path.display());
                Ok(None)
            }
        }
    })
}

// ----------------------------------------------------------------------------
/// The vault `root` opens: its accounts replace the ones before, and the
/// page gets them all (`accounts`).
pub(crate) fn open(app: &mut App, root: &Path) {
    let folder = root.join(layout::ACCOUNTS);
    let secrets = app.config.dir().to_path_buf();
    let ai = load(&folder, &secrets);
    log::info!("{} AI account(s) in {}", ai.len(), folder.display());
    app.ai = AiAccounts::new(folder, secrets, ai);
    post_accounts(app);
}

/// The `accounts` event.
#[derive(Serialize)]
struct Accounts<'a> {
    items: Vec<AiAccountMeta<'a>>,
}

// ----------------------------------------------------------------------------
/// Send every account of the open vault: the `accounts` event, which
/// replaces the ones the page has.
pub(crate) fn post_accounts(app: &App) {
    let items = app.ai.iter().map(|a| a.meta()).collect();
    app.post_event("accounts", Accounts { items });
}

// ----------------------------------------------------------------------------
/// `disconnect_account {account}`: an AI account forgets its key; it keeps
/// its file and id, marked `disconnected`. Answered after the `account`
/// event.
pub fn disconnect_account(app: &mut App, req: Req, id: Id) {
    req.answer(ai::disconnect(app, id));
}

// ----------------------------------------------------------------------------
/// `remove_account {account}`: forget an account that isn't connected (an
/// Ollama account in any state) by deleting its files. Documents keep
/// naming its id. Answered after `account_removed`.
pub fn remove_account(app: &mut App, req: Req, id: Id) {
    log::info!("Removing account {id}");
    let removed = app.ai.remove(id);
    match &removed {
        Ok(()) => app.post_event("account_removed", json!({ "id": id })),
        Err(e) => log::warn!("Could not remove account {id}: {e}"),
    }
    req.answer(removed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Harness, parse, shape};
    use av_utils::secret;
    use serde_json::Value as Json;

    const OPENAI_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b30";

    fn openai(id: &str) -> String {
        format!("id: {id}\nopenai:\n  model: gpt-5-mini\n")
    }

    fn write(path: &Path, text: impl AsRef<[u8]>) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn secret_file(secrets: &Path, name: &str, value: &str) {
        write(
            &secrets.join(name),
            secret::protect(value.as_bytes()).unwrap(),
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn one_pass_hands_each_file_to_its_kind() {
        let dir = std::env::temp_dir().join(format!("avelor-accounts-{}", Id::generate()));
        let (folder, secrets) = (dir.join("accounts"), dir.join("config"));
        write(&folder.join("openai.yaml"), openai(OPENAI_ID));
        secret_file(&secrets, &format!("{OPENAI_ID}.key"), "sk-0123456789abcd");
        let ollama = "id: 01a0e1cc-a24c-7490-82fe-e75669985b32\nollama:\n  model: qwen\n";
        write(&folder.join("ollama.yaml"), ollama);
        let watch = "id: 01a0e1cc-a24c-7490-82fe-e75669985b34\nfolder-watch:\n  type: pdf\n";
        write(&folder.join("watch_pdf.yaml"), watch);
        write(
            &folder.join("openai_bad_id.yaml"),
            "id: E:/e-office\nopenai: {}\n",
        );
        write(&folder.join("broken.yaml"), "google: [unclosed\n");

        let ai = load(&folder, &secrets);
        fs::remove_dir_all(&dir).unwrap();

        let kinds: Vec<_> = ai
            .iter()
            .map(|a| (a.provider().as_str(), a.status()))
            .collect();
        assert_eq!(kinds, [("ollama", "unknown"), ("openai", "connected")]);
    }

    // ------------------------------------------------------------------------
    /// A harness whose vault has an OpenAI account with a key.
    fn with_accounts(name: &str, _disconnected: bool) -> Harness {
        let note =
            "---\nid: 01a0e1cc-a24c-7490-82fe-e75669985b40\ndocument: {}\ntitle: Note\n---\n";
        let files = [
            (
                ".avelor/accounts/openai.yaml".to_string(),
                openai(OPENAI_ID),
            ),
            ("Notes/n.md".to_string(), note.to_string()),
        ];
        let mut h = Harness::new(name);
        let secrets = h.app.config.dir().to_path_buf();
        secret_file(&secrets, &format!("{OPENAI_ID}.key"), "sk-0123456789abcd");
        let root = h.vault();
        for (path, text) in &files {
            write(&root.join(path), text);
        }
        crate::api::switch_to(&mut h.app, root.clone());
        h.wait_scanned();
        h
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_vault_brings_its_accounts_and_another_vault_replaces_them() {
        let h = with_accounts("vault-accounts", false);
        let sent = parse(&h.take());
        let accounts: Vec<&Json> = sent.iter().filter(|m| m["ev"] == "accounts").collect();
        assert_eq!(accounts.len(), 1, "{sent:?}");
        let items = accounts[0]["data"]["items"].as_array().unwrap();
        let kinds: Vec<_> = items
            .iter()
            .map(|a| (a["kind"].as_str(), a["status"].as_str()))
            .collect();
        assert_eq!(kinds, [(Some("openai"), Some("connected"))]);
        // The account files are no entities.
        let entities: Vec<_> = sent.iter().filter(|m| m["ev"] == "entity").collect();
        assert_eq!(entities.len(), 1, "only the note");

        let mut h = h;
        let other = h.vault().with_file_name("other");
        fs::create_dir_all(&other).unwrap();
        crate::api::switch_to(&mut h.app, other.clone());
        h.wait_scanned();
        let sent = parse(&h.take());
        let accounts = sent.iter().find(|m| m["ev"] == "accounts").unwrap();
        assert_eq!(accounts["data"]["items"], serde_json::json!([]));
        assert!(h.app.ai.iter().next().is_none());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_ai_account_is_disconnected_then_removed() {
        let mut h = with_accounts("ai-disconnect", false);
        h.take();
        let key = h.app.config.dir().join(format!("{OPENAI_ID}.key"));
        let args = serde_json::json!({ "account": OPENAI_ID });
        let sent = h.call("remove_account", args.clone());
        assert_eq!(sent[0]["err"]["code"], "unsupported", "it has a key");

        let sent = h.call("disconnect_account", args.clone());
        assert_eq!(shape(&sent), ["account", "reply"]);
        assert_eq!(sent[0]["data"]["status"], "disconnected");
        assert!(!key.exists());
        let sent = h.call("remove_account", args);
        assert_eq!(shape(&sent), ["account_removed", "reply"]);
        assert!(!h.vault().join(".avelor/accounts/openai.yaml").exists());
    }
}
