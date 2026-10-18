//! AI accounts' commands (api.md#ai): check and store a key or an
//! address, list the models, keep the offered models and settings. The
//! accounts themselves: `ai_accounts`.

use std::path::PathBuf;

use av_llm::{Client, Provider};
use av_utils::cancel::Cancel;
use av_utils::error::{Error, Result};
use av_utils::id::Id;
use serde::{Deserialize, Deserializer, Serialize};

use crate::ai_accounts;
use crate::app::App;
use crate::config_files::unique_names;
use crate::protocol::Req;

// ----------------------------------------------------------------------------
/// Send the account `id` to the page, if there is one: the `account` event.
pub fn post(app: &App, id: Id) {
    if let Ok(account) = app.ai.get(id) {
        app.post_event("account", account.meta());
    }
}

// ----------------------------------------------------------------------------
/// One model of the answers to `connect_ai` and `list_ai_models`.
#[derive(Serialize)]
struct ModelEntry {
    id: String,
    name: String,
    context_window: Option<u64>,
    /// USD per million tokens; `None` if unknown.
    price: Option<PriceEntry>,
}

#[derive(Serialize)]
struct PriceEntry {
    input: f64,
    output: f64,
}

/// `connect_ai`'s answer.
#[derive(Serialize)]
struct Connected {
    id: Id,
    models: Vec<ModelEntry>,
}

/// `list_ai_models`' answer.
#[derive(Serialize)]
struct Models {
    models: Vec<ModelEntry>,
}

// ----------------------------------------------------------------------------
/// The models `client` can chat with, priced for the page.
fn list_models_of(client: &Client, cancel: &Cancel) -> Result<Vec<ModelEntry>> {
    let models = client.models(cancel)?;
    Ok(models
        .into_iter()
        .map(|model| ModelEntry {
            price: client.price(&model.id).map(|price| PriceEntry {
                input: price.input,
                output: price.output,
            }),
            id: model.id,
            name: model.name,
            context_window: model.context_window,
        })
        .collect())
}

// ----------------------------------------------------------------------------
/// What went wrong checking an account, for the page.
fn check_error(provider: Provider, address: &str, err: &Error) -> String {
    let name = provider.spec().display;
    error_text(provider, address, err).unwrap_or_else(|| format!("Checking {name} failed: {err}"))
}

// ----------------------------------------------------------------------------
/// What went wrong in a chat, for the page.
pub fn chat_error(provider: Provider, address: &str, err: &Error) -> String {
    let name = provider.spec().display;
    error_text(provider, address, err).unwrap_or_else(|| format!("{name} failed: {err}"))
}

// ----------------------------------------------------------------------------
/// A rejected key, an HTTP error, or an unreachable server, in words;
/// `None` for anything else.
fn error_text(provider: Provider, address: &str, err: &Error) -> Option<String> {
    let spec = provider.spec();
    let name = spec.display;
    let text = match err {
        Error::Http {
            status: 401 | 403,
            msg,
            ..
        } if spec.needs_key => {
            if msg.is_empty() {
                "The key was rejected.".to_string()
            } else {
                format!("The key was rejected: {msg}")
            }
        }
        Error::Http { status, msg, .. } => format!("{name} answered {status}: {msg}"),
        // A server of the user's own.
        Error::Network { .. } | Error::Timeout if !spec.needs_key => {
            format!("{name} isn't reachable at {address}. Is it running?")
        }
        Error::Network { msg } => format!("{name} isn't reachable: {msg}"),
        Error::Timeout => format!("{name} didn't answer in time."),
        _ => return None,
    };
    Some(text)
}

// ----------------------------------------------------------------------------
/// Answer a failed save of an account with a sentence for the page.
fn save_failed(req: Req, error: Error) {
    log::warn!("Could not save the account: {error}");
    req.fail(error, "Could not save the account.");
}

// ----------------------------------------------------------------------------
/// `connect_ai {account?, provider?, key?, address?}`: check a key or an
/// address by listing the models with it, and only then store it -- in the
/// account `target`, or in a new account of `provider`; what is left out
/// stays as it is. Answers the account's id and its models, after the
/// `account` event; nothing is stored if the check fails. A newer
/// `connect_ai` supersedes a running one, which answers `cancelled`, and so
/// does a vault switch.
pub fn connect_ai(
    app: &mut App,
    req: Req,
    target: Option<Id>,
    provider: Option<Provider>,
    key: Option<String>,
    address: Option<String>,
) {
    // The vault the account is kept in; a switch meanwhile drops the result.
    let folder = app.ai.folder().to_path_buf();
    if folder.as_os_str().is_empty() {
        return req.err(Error::InvalidPath {
            path: PathBuf::new(),
        });
    }
    let account = match target.map(|id| app.ai.get(id)).transpose() {
        Ok(account) => account,
        Err(e) => return req.err(e),
    };
    let provider = match (account, provider) {
        (Some(account), _) => account.provider(),
        (None, Some(provider)) => provider,
        (None, None) => {
            let arg = "provider".to_string();
            return req.fail(Error::InvalidArgument { arg }, "Choose an AI provider.");
        }
    };
    let key = key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());
    let address = address
        .filter(|address| !address.trim().is_empty())
        .map(|address| av_llm::base_url(&address));

    // Checked with what is new, and the account's own for the rest.
    let check_address = match (&address, account) {
        (Some(address), _) => address.clone(),
        (None, Some(account)) => account.address().to_string(),
        (None, None) => provider.spec().default_url.to_string(),
    };
    let client = match (account, &key) {
        (Some(account), None) => account.client().with_base_url(&check_address),
        (_, key) => ai_accounts::candidate(provider, key.clone(), &check_address),
    };
    let has_key = key.is_some() || account.is_some_and(|account| account.has_key());
    if provider.spec().needs_key && !has_key {
        let arg = "key".to_string();
        return req.fail(Error::InvalidArgument { arg }, "Enter the API key.");
    }

    let op = Id::generate();
    log::info!(
        "Connect AI {op}: checking {provider} at {check_address}{}",
        if key.is_some() { " with a new key" } else { "" }
    );
    let work = move |cancel: &Cancel| list_models_of(&client, cancel);
    let latest = op.to_string();
    app.run_superseding("connect_ai", &latest, work, move |app, result| {
        let models = match result {
            Ok(models) => models,
            Err(e) => {
                let message = check_error(provider, &check_address, &e);
                log::info!("Connect AI {op}: {message}");
                return req.fail(e, message);
            }
        };
        if app.ai.folder() != folder {
            log::info!("Connect AI {op}: the vault was switched meanwhile");
            return req.err(Error::Cancelled);
        }
        match app.ai.connect(target, provider, key, address) {
            Ok(id) => {
                let count = models.len();
                log::info!("Connect AI {op}: {provider} account {id} stored, {count} models");
                post(app, id);
                req.ok(Connected { id, models });
            }
            Err(Error::NotFound) => req.err(Error::NotFound), // removed meanwhile
            Err(e) => save_failed(req, e),
        }
    });
}

// ----------------------------------------------------------------------------
/// `list_ai_models {account}`: the account's models, after the `account`
/// event (an Ollama account's status follows the listing).
pub fn list_ai_models(app: &mut App, req: Req, id: Id) {
    let account = match app.ai.get(id) {
        Ok(account) => account,
        Err(e) => return req.err(e),
    };
    if account.uses_key() && !account.has_key() {
        return req.fail(Error::NotConnected, "The account has no key.");
    }
    let (provider, address) = (account.provider(), account.address().to_string());
    let client = account.client();
    log::info!("Listing the models of {id}");
    app.run(
        move |cancel| list_models_of(&client, cancel),
        move |app, result| {
            if !app.ai.contains(id) {
                return req.err(Error::NotFound);
            }
            // An error status still means the server is there.
            let reached = !matches!(&result, Err(Error::Network { .. } | Error::Timeout));
            app.ai.set_reachable(id, reached);
            post(app, id);
            match result {
                Ok(models) => {
                    log::info!("{} models of {id}", models.len());
                    req.ok(Models { models });
                }
                Err(e) => {
                    let message = check_error(provider, &address, &e);
                    log::info!("Listing the models of {id}: {message}");
                    req.fail(e, message);
                }
            }
        },
    );
}

// ----------------------------------------------------------------------------
/// A field that may be `null`: absent is `None`, `null` is `Some(None)`.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

// ----------------------------------------------------------------------------
/// `set_ai_settings`' arguments; only the fields given change.
#[derive(Deserialize)]
pub struct SettingsChange {
    account: Id,
    #[serde(default)]
    offered: Option<Vec<String>>,
    /// USD a month; `null` for no limit.
    #[serde(default, deserialize_with = "present")]
    spend_limit: Option<Option<f64>>,
}

// ----------------------------------------------------------------------------
/// `set_ai_settings {account, offered?, spend_limit?}`: change the offered
/// models or the spend limit. Answered after the `account` event; nothing
/// changes if saving fails.
pub fn set_ai_settings(app: &mut App, req: Req, change: SettingsChange) {
    let id = change.account;
    if let Some(Some(limit)) = change.spend_limit {
        if !(limit.is_finite() && limit > 0.0) {
            let arg = "spend_limit".to_string();
            let message = "The spend limit must be an amount above 0.";
            return req.fail(Error::InvalidArgument { arg }, message);
        }
    }
    let updated = app.ai.update(id, |settings| {
        if let Some(offered) = change.offered {
            settings.offered = unique_names(offered);
        }
        if let Some(limit) = change.spend_limit {
            settings.spend_limit = limit;
        }
    });
    match updated {
        Ok(()) => {
            log::info!("Settings of AI account {id} saved");
            post(app, id);
            req.ok(());
        }
        Err(Error::NotFound) => req.err(Error::NotFound),
        Err(e) => save_failed(req, e),
    }
}

// ----------------------------------------------------------------------------
/// The AI half of `disconnect_account`: forget the key of the account `id`,
/// keep the account (and its id), marked `disconnected`. The account
/// follows as the `account` event.
pub fn disconnect(app: &mut App, id: Id) -> Result<()> {
    log::info!("Disconnecting AI account {id}");
    app.ai.disconnect(id)?;
    post(app, id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeOllama, Harness, shape};
    use serde_json::{Value, json};
    use std::net::TcpListener;

    // ------------------------------------------------------------------------
    #[test]
    fn a_null_spend_limit_removes_it_and_absent_keeps_it() {
        let parse = |json: &str| serde_json::from_str::<SettingsChange>(json).unwrap();
        let id = Id::generate();
        let change = parse(&format!(r#"{{"account": "{id}", "spend_limit": null}}"#));
        assert_eq!(change.spend_limit, Some(None));
        let change = parse(&format!(r#"{{"account": "{id}", "offered": []}}"#));
        assert_eq!(change.spend_limit, None);
        let change = parse(&format!(r#"{{"account": "{id}", "spend_limit": 10}}"#));
        assert_eq!(change.spend_limit, Some(Some(10.0)));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn rejected_keys_and_a_stopped_ollama_read_well() {
        let rejected = Error::Http {
            status: 401,
            reason: "invalid_api_key".into(),
            msg: "Incorrect API key provided.".into(),
        };
        assert_eq!(
            check_error(Provider::OpenAi, "https://api.openai.com/v1", &rejected),
            "The key was rejected: Incorrect API key provided."
        );
        let refused = Error::Network {
            msg: "connection refused".into(),
        };
        assert_eq!(
            check_error(Provider::Ollama, "http://localhost:11434", &refused),
            "Ollama isn't reachable at http://localhost:11434. Is it running?"
        );
    }

    // ------------------------------------------------------------------------
    /// A harness with an empty vault open.
    fn with_vault(name: &str) -> Harness {
        Harness::with_vault(name, &[("Notes/.keep", "")])
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_ollama_account_is_stored_once_its_address_answers() {
        let mut h = with_vault("ai-connect");
        let address = FakeOllama::new(&["qwen3:8b", "llama3.2"]).start();
        let args = json!({ "provider": "ollama", "address": address });
        let sent = h.call_waiting("connect_ai", args);

        assert_eq!(shape(&sent), ["account", "reply"]);
        assert_eq!(sent[0]["data"]["kind"], "ollama");
        assert_eq!(sent[0]["data"]["status"], "running");
        let id = sent[1]["ok"]["id"].as_str().unwrap();
        assert_eq!(sent[0]["data"]["id"], id);
        let models: Vec<_> = sent[1]["ok"]["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(models, ["llama3.2", "qwen3:8b"]);
        let text = std::fs::read_to_string(h.vault().join(".avelor/accounts/ollama.yaml")).unwrap();
        assert!(text.contains(&address), "{text}");

        let sent = h.call_waiting("list_ai_models", json!({ "account": id }));
        assert_eq!(shape(&sent), ["account", "reply"]);
        assert_eq!(sent[1]["ok"]["models"].as_array().unwrap().len(), 2);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_address_that_does_not_answer_stores_nothing() {
        let mut h = with_vault("ai-refused");
        // A port that was free a moment ago: nothing listens there.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let address = format!("http://127.0.0.1:{port}");
        let args = json!({ "provider": "ollama", "address": address });
        let sent = h.call_waiting("connect_ai", args);

        assert_eq!(shape(&sent), ["reply"]);
        assert_eq!(sent[0]["err"]["code"], "network");
        let message = format!("Ollama isn't reachable at {address}. Is it running?");
        assert_eq!(sent[0]["err"]["message"], message);
        assert!(h.app.ai.iter().next().is_none());
        assert!(!h.vault().join(".avelor/accounts/ollama.yaml").exists());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn a_key_provider_needs_a_key_and_a_vault() {
        let mut h = Harness::new("ai-no-vault");
        let args = json!({ "provider": "openai", "key": "sk-0123456789abcdef" });
        let sent = h.call("connect_ai", args);
        assert_eq!(sent[0]["err"]["code"], "invalid_path");

        let mut h = with_vault("ai-no-key");
        let sent = h.call("connect_ai", json!({ "provider": "openai", "key": " " }));
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        assert_eq!(sent[0]["err"]["message"], "Enter the API key.");
        let sent = h.call("connect_ai", json!({ "provider": "lmstudio" }));
        assert_eq!(sent[0]["err"]["code"], "bad_request");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn settings_are_saved_and_pushed() {
        let id = "01a0e1cc-a24c-7490-82fe-e75669985b32";
        let ollama = format!("id: {id}\nollama:\n  custom: kept\n");
        let mut h = Harness::with_vault("ai-settings", &[(".avelor/accounts/ollama.yaml", ollama)]);
        let change = json!({
            "account": id,
            "offered": ["qwen", "llama", "qwen", ""],
            "spend_limit": 5,
        });
        let sent = h.call("set_ai_settings", change);

        assert_eq!(shape(&sent), ["account", "reply"]);
        assert_eq!(sent[0]["data"]["offered"], json!(["qwen", "llama"]));
        assert_eq!(sent[1]["ok"], Value::Null);
        let text = std::fs::read_to_string(h.vault().join(".avelor/accounts/ollama.yaml")).unwrap();
        assert!(
            text.contains("spend-limit: 5") && text.contains("custom: kept"),
            "{text}"
        );

        let sent = h.call(
            "set_ai_settings",
            json!({ "account": id, "spend_limit": 0 }),
        );
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        let unknown = Id::generate();
        let sent = h.call(
            "set_ai_settings",
            json!({ "account": unknown, "offered": ["x"] }),
        );
        assert_eq!(sent[0]["err"]["code"], "not_found");
        let sent = h.call("list_ai_models", json!({ "account": unknown }));
        assert_eq!(sent[0]["err"]["code"], "not_found");
    }
}
