//! Accounts of both kinds (accounts.md): their files in the
//! open vault's `.avelor/accounts/` (and an AI account's spend file beside its
//! own), read in one pass when a vault opens; what
//! the page gets about them (`accounts`, `account`, `account_removed`); and
//! what both kinds do alike, disconnect and remove (api.md#google).
//! Google accounts: `google_accounts`, `google`; AI accounts: `ai_accounts`,
//! `ai`.

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
use crate::google;
use crate::google_accounts::{self, Account, AccountMeta, GoogleAccounts};
use crate::protocol::Req;

// ----------------------------------------------------------------------------
/// An account file, read.
enum Loaded {
    Google(Account),
    Ai(AiAccount),
}

// ----------------------------------------------------------------------------
/// The accounts in `folder`, their secrets in `secrets`: every `*.yaml`,
/// handed to the loader of its top-level key (`google`, or a provider's);
/// AI accounts read their spend files themselves. Other and broken files
/// are logged and skipped.
pub(crate) fn load(folder: &Path, secrets: &Path) -> (Vec<Account>, Vec<AiAccount>) {
    let loaded = load_config_files(folder, "account", |path| {
        if ai_accounts::is_spend_file(path) {
            return Ok(None);
        }
        let Value::Mapping(map) = serde_yaml_ng::from_str(&fs::read_to_string(path)?)? else {
            log::warn!("Skipping {}: not an account", path.display());
            return Ok(None);
        };
        if map.contains_key("google") {
            let account = google_accounts::load(path, Value::Mapping(map), secrets)?;
            return Ok(Some(Loaded::Google(account)));
        }
        match ai_accounts::kind_of(&map) {
            Some(kind) => {
                let account = ai_accounts::load(path, map, kind, secrets)?;
                Ok(Some(Loaded::Ai(account)))
            }
            None => {
                log::warn!("Skipping {}: not an account", path.display());
                Ok(None)
            }
        }
    });
    let (mut google, mut ai) = (Vec::new(), Vec::new());
    for account in loaded {
        match account {
            Loaded::Google(account) => google.push(account),
            Loaded::Ai(account) => ai.push(account),
        }
    }
    (google, ai)
}

// ----------------------------------------------------------------------------
/// The vault `root` opens: its accounts replace the ones before, a pending
/// sign-in is cancelled, and the page gets them all (`accounts`).
pub(crate) fn open(app: &mut App, root: &Path) {
    let folder = root.join(layout::ACCOUNTS);
    let secrets = app.config.dir().to_path_buf();
    let (mut google, ai) = load(&folder, &secrets);
    if app.hides_google {
        // Their files stay as they are.
        google.clear();
    }
    log::info!(
        "{} Google and {} AI account(s) in {}",
        google.len(),
        ai.len(),
        folder.display()
    );
    app.cancel_connect();
    app.google = GoogleAccounts::new(folder.clone(), secrets.clone(), google);
    app.ai = AiAccounts::new(folder, secrets, ai);
    post_accounts(app);
}

// ----------------------------------------------------------------------------
/// What the page gets of an account of either kind.
#[derive(Serialize)]
#[serde(untagged)]
enum Meta<'a> {
    Google(AccountMeta<'a>),
    Ai(AiAccountMeta<'a>),
}

/// The `accounts` event.
#[derive(Serialize)]
struct Accounts<'a> {
    items: Vec<Meta<'a>>,
}

// ----------------------------------------------------------------------------
/// Send every account of the open vault: the `accounts` event, which
/// replaces the ones the page has.
pub(crate) fn post_accounts(app: &App) {
    let google = app.google.iter().map(|a| Meta::Google(a.meta()));
    let ai = app.ai.iter().map(|a| Meta::Ai(a.meta()));
    let items = google.chain(ai).collect();
    app.post_event("accounts", Accounts { items });
}

// ----------------------------------------------------------------------------
/// `disconnect_account {account}`: a Google account signs out
/// (`google::disconnect`), an AI account forgets its key; either keeps its
/// file and id, marked `disconnected`. Answered after the `account` event.
pub fn disconnect_account(app: &mut App, req: Req, id: Id) {
    if app.ai.contains(id) {
        req.answer(ai::disconnect(app, id));
    } else {
        google::disconnect(app, req, id);
    }
}

// ----------------------------------------------------------------------------
/// `remove_account {account}`: forget an account that isn't connected (an
/// Ollama account in any state) by deleting its files. Documents keep
/// naming its id; connecting the same Google account later creates a new
/// one. Answered after `account_removed`.
pub fn remove_account(app: &mut App, req: Req, id: Id) {
    log::info!("Removing account {id}");
    let removed = match app.ai.contains(id) {
        true => app.ai.remove(id),
        false => app.google.remove(id),
    };
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

    const GOOGLE_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b24";
    const OPENAI_ID: &str = "01a0e1cc-a24c-7490-82fe-e75669985b30";

    fn google(id: &str, extra: &str) -> String {
        format!(
            "id: {id}\ngoogle:\n  email: someone@example.com\n  scopes:\n    - calendar: full\n{extra}  custom: kept\n"
        )
    }

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
        write(&folder.join("google_a.yaml"), google(GOOGLE_ID, ""));
        secret_file(&secrets, &format!("{GOOGLE_ID}.token"), "1//a");
        write(&folder.join("openai.yaml"), openai(OPENAI_ID));
        secret_file(&secrets, &format!("{OPENAI_ID}.key"), "sk-0123456789abcd");
        let ollama = "id: 01a0e1cc-a24c-7490-82fe-e75669985b32\nollama:\n  model: qwen\n";
        write(&folder.join("ollama.yaml"), ollama);
        let spend = format!("account: {OPENAI_ID}\nmonths: {{}}\n");
        write(&folder.join("openai.spend.yaml"), spend);
        let watch = "id: 01a0e1cc-a24c-7490-82fe-e75669985b34\nfolder-watch:\n  type: pdf\n";
        write(&folder.join("watch_pdf.yaml"), watch);
        write(
            &folder.join("openai_bad_id.yaml"),
            "id: E:/e-office\nopenai: {}\n",
        );
        write(&folder.join("broken.yaml"), "google: [unclosed\n");

        let (google, ai) = load(&folder, &secrets);
        fs::remove_dir_all(&dir).unwrap();

        assert_eq!(google.len(), 1);
        assert_eq!(google[0].status().name(), "connected");
        let kinds: Vec<_> = ai
            .iter()
            .map(|a| (a.provider().as_str(), a.status()))
            .collect();
        assert_eq!(kinds, [("ollama", "unknown"), ("openai", "connected")]);
    }

    // ------------------------------------------------------------------------
    /// A harness whose vault has a Google account (with a token, unless
    /// `disconnected`) and an OpenAI account with a key.
    fn with_accounts(name: &str, disconnected: bool) -> Harness {
        let extra = match disconnected {
            true => "  disconnected: 2026-10-01T08:00:00Z\n",
            false => "",
        };
        let note =
            "---\nid: 01a0e1cc-a24c-7490-82fe-e75669985b40\ndocument: {}\ntitle: Note\n---\n";
        let files = [
            (
                ".avelor/accounts/google_a.yaml".to_string(),
                google(GOOGLE_ID, extra),
            ),
            (
                ".avelor/accounts/openai.yaml".to_string(),
                openai(OPENAI_ID),
            ),
            ("Notes/n.md".to_string(), note.to_string()),
        ];
        let mut h = Harness::new(name);
        let secrets = h.app.config.dir().to_path_buf();
        if !disconnected {
            secret_file(&secrets, &format!("{GOOGLE_ID}.token"), "1//a");
        }
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
        assert_eq!(
            kinds,
            [
                (Some("google"), Some("connected")),
                (Some("openai"), Some("connected"))
            ]
        );
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
        assert!(h.app.google.iter().next().is_none());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_build_that_hides_google_leaves_the_google_accounts_out() {
        let mut h = with_accounts("hidden-google", false);
        h.take();
        h.app.hides_google = true;
        let root = h.vault();
        crate::api::switch_to(&mut h.app, root.clone());
        h.wait_scanned();
        let sent = parse(&h.take());
        let accounts = sent.iter().find(|m| m["ev"] == "accounts").unwrap();
        let items = accounts["data"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0]["kind"], "openai");
        assert!(root.join(".avelor/accounts/google_a.yaml").exists());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn lookups_answer_unknown_and_not_connected() {
        let mut h = with_accounts("calendars-errors", true);
        h.take();
        let unknown = Id::generate().to_string();
        let replies = h.call("list_calendars", serde_json::json!({ "account": unknown }));
        assert_eq!(replies[0]["err"]["code"], "not_found");
        let replies = h.call(
            "list_calendars",
            serde_json::json!({ "account": GOOGLE_ID }),
        );
        assert_eq!(replies[0]["err"]["code"], "not_connected");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_calendar_is_checked_as_a_source() {
        let mut h = with_accounts("calendar-source", false);
        h.take();
        let calendar = serde_json::json!({ "id": "fam@group", "name": "Family", "color": null });
        let args =
            serde_json::json!({ "account": GOOGLE_ID, "calendar": calendar, "selected": true });
        let sent = h.call("set_calendar_source", args);
        assert_eq!(shape(&sent), ["account", "reply"]);
        assert_eq!(sent[0]["data"]["calendars"][0]["name"], "Family");
        assert_eq!(sent[1]["ok"], Json::Null);
        let file = h.vault().join(".avelor/accounts/google_a.yaml");
        let text = fs::read_to_string(file).unwrap();
        assert!(
            text.contains("fam@group") && text.contains("custom: kept"),
            "{text}"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_disconnected_google_account_is_removed_with_its_file() {
        let mut h = with_accounts("remove-google", true);
        h.take();
        let sent = h.call(
            "remove_account",
            serde_json::json!({ "account": GOOGLE_ID }),
        );
        assert_eq!(shape(&sent), ["account_removed", "reply"]);
        assert_eq!(sent[0]["data"]["id"], GOOGLE_ID);
        assert!(!h.vault().join(".avelor/accounts/google_a.yaml").exists());
        assert!(h.app.google.iter().next().is_none());
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

    // ------------------------------------------------------------------------
    #[test]
    fn cancelling_without_a_pending_sign_in_is_fine() {
        let mut h = Harness::new("cancel-connect");
        let sent = h.call("cancel_connect", serde_json::json!({}));
        assert_eq!(sent, [serde_json::json!({ "re": 1, "ok": null })]);
        let sent = h.call(
            "connect_google",
            serde_json::json!({ "levels": { "email": "read" } }),
        );
        assert_eq!(
            sent[0]["err"]["code"], "invalid_path",
            "no vault to keep it in"
        );
    }
}
