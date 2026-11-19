//! Google's OAuth client (`accounts.md#own-client`): the build's, or else
//! the user's own, kept as a secret in the config folder (`google.client`)
//! and given on the Accounts page, typed or as the file Google hands out
//! (`set_google_client`, `load_google_client`).

use std::fs;
use std::path::Path;

use av_google::auth::{self, OAuth, OAuthClient};
use av_utils::cancel::Cancel;
use av_utils::error::Error;
use av_utils::id::Id;
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::{Value, json};

use crate::app::App;
use crate::config::files::{SecretFile, secret_hint};
use crate::protocol::Req;
use crate::sys;
use crate::texts::{self, Text};

/// The user's own client, in the config folder beside the accounts'
/// secrets.
const FILE: &str = "google.client";
/// How the id of every client Google hands out ends.
const ID_END: &str = ".apps.googleusercontent.com";

// ----------------------------------------------------------------------------
/// Where Google's OAuth client comes from (`accounts.md#design`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoogleClient {
    /// The build has no Google (no feature `google-account`).
    Absent,
    /// The build's (`auth::built_in`).
    BuiltIn,
    /// The user's own (`google.client`), once given.
    Own,
}

// ----------------------------------------------------------------------------
/// The client a run starts with: the build's, or the user's own from
/// `config_dir` if they gave one.
pub fn start(config_dir: &Path) -> (GoogleClient, OAuth) {
    if !auth::has_google() {
        return (GoogleClient::Absent, OAuth::default());
    }
    match auth::built_in() {
        Some(client) => (GoogleClient::BuiltIn, OAuth::new(Some(client))),
        None => (GoogleClient::Own, OAuth::new(read(config_dir))),
    }
}

// ----------------------------------------------------------------------------
fn secret(config_dir: &Path) -> SecretFile {
    SecretFile::named(config_dir, FILE)
}

// ----------------------------------------------------------------------------
/// The user's own client, if they gave one that can be read here (one of
/// another Windows user can't: logged).
fn read(config_dir: &Path) -> Option<OAuthClient> {
    let text = match secret(config_dir).read() {
        Ok(text) => text,
        Err(Error::FileNotFound { .. }) => return None,
        Err(e) => {
            log::warn!("The Google client can't be read: {e}");
            return None;
        }
    };
    serde_json::from_str(&text)
        .inspect_err(|e| log::warn!("The Google client can't be read: {e}"))
        .ok()
}

// ----------------------------------------------------------------------------
/// What the page gets of the client (`ready`, the `google_client` event):
/// `"built-in"`, the user's own as its id and its secret's ends, or null
/// (none yet, or no Google).
pub fn meta(app: &App) -> Value {
    match app.google_client {
        GoogleClient::BuiltIn => json!("built-in"),
        GoogleClient::Own => app.oauth.get().map_or(
            Value::Null,
            |client| json!({ "id": client.id, "secret": secret_hint(&client.secret) }),
        ),
        GoogleClient::Absent => Value::Null,
    }
}

// ----------------------------------------------------------------------------
/// The two commands' answer in a build that doesn't take the user's own
/// client.
fn unsupported() -> Error {
    let what = "a Google client of the user's own in this build".to_string();
    Error::Unsupported { what }
}

// ----------------------------------------------------------------------------
/// `set_google_client {id, secret}`: the user's own client, checked with
/// Google (`auth::check_client`) and only then kept, replacing the one
/// before; the `google_client` event, then `null`.
pub fn set_google_client(app: &mut App, req: Req, id: &str, secret: &str) {
    if app.google_client != GoogleClient::Own {
        return req.err(unsupported());
    }
    match shaped(id, secret) {
        Ok(client) => check_and_keep(app, req, client),
        Err((error, text)) => req.fail(error, text),
    }
}

// ----------------------------------------------------------------------------
/// `load_google_client`: the client in the file Google hands out, picked by
/// the user; then as `set_google_client`. `null` if the dialog was
/// cancelled.
pub fn load_google_client(app: &mut App, req: Req) {
    if app.google_client != GoogleClient::Own {
        return req.err(unsupported());
    }
    app.on_ui(
        |owner| {
            let title = texts::text("google.client.pick");
            sys::pick_file(
                owner,
                title,
                texts::text("google.client.file_kind"),
                "*.json",
            )
        },
        |app, picked| {
            let Some(path) = picked else {
                return req.ok(Value::Null);
            };
            let read = fs::read_to_string(&path)
                .map_err(|e| (Error::from(e), Text::new("google.client.not_a_file")));
            match read.and_then(|text| from_google_file(&text)) {
                Ok(client) => check_and_keep(app, req, client),
                Err((error, text)) => req.fail(error, text),
            }
        },
    );
}

// ----------------------------------------------------------------------------
/// A client of what the user gave, trimmed: an id as Google's end
/// (`….apps.googleusercontent.com`), and a secret.
fn shaped(id: &str, secret: &str) -> Result<OAuthClient, (Error, Text)> {
    let (id, secret) = (id.trim(), secret.trim());
    let invalid = |arg: &str| Error::InvalidArgument {
        arg: arg.to_string(),
    };
    if id.len() <= ID_END.len() || !id.ends_with(ID_END) {
        return Err((invalid("id"), Text::new("google.client.id_invalid")));
    }
    if secret.is_empty() {
        return Err((invalid("secret"), Text::new("google.client.secret_missing")));
    }
    Ok(OAuthClient {
        id: id.to_string(),
        secret: secret.to_string(),
    })
}

// ----------------------------------------------------------------------------
/// The file Google hands out for a client: a Desktop app's under
/// `installed`, a web application's under `web`.
#[derive(Deserialize)]
struct GoogleFile {
    installed: Option<FileClient>,
    web: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct FileClient {
    client_id: String,
    client_secret: String,
}

// ----------------------------------------------------------------------------
/// The client in Google's file `text`; a web application's is refused, as
/// its sign-in can't come back to the app.
fn from_google_file(text: &str) -> Result<OAuthClient, (Error, Text)> {
    let not_a_file = |error| (error, Text::new("google.client.not_a_file"));
    let file: GoogleFile = serde_json::from_str(text).map_err(|e| not_a_file(Error::from(e)))?;
    match file {
        GoogleFile {
            installed: Some(client),
            ..
        } => shaped(&client.client_id, &client.client_secret),
        GoogleFile { web: Some(_), .. } => {
            let what = "a web application's client".to_string();
            Err((Error::Unsupported { what }, Text::new("google.client.web")))
        }
        _ => Err(not_a_file(Error::InvalidFileType)),
    }
}

// ----------------------------------------------------------------------------
/// Check `client` with Google on a worker, then keep it: written to
/// `google.client`, the accounts' client from now on. A newer client
/// supersedes one that is still being checked, which answers `cancelled`.
fn check_and_keep(app: &mut App, req: Req, client: OAuthClient) {
    log::info!("Checking the Google client {}", client.id);
    let checked = client.clone();
    let latest = Id::generate().to_string();
    let work = move |cancel: &Cancel| auth::check_client(cancel, &checked);
    app.run_superseding("google_client", &latest, work, move |app, result| {
        if let Err(e) = result {
            log::info!("The Google client {} wasn't taken: {e}", client.id);
            return match &e {
                Error::Http { reason, .. } if reason == "invalid_client" => {
                    req.fail(e, Text::new("google.client.refused"))
                }
                _ => req.err(e),
            };
        }
        let kept = serde_json::to_string(&client)
            .map_err(Error::from)
            .and_then(|text| secret(app.config.dir()).write(&text));
        if let Err(e) = kept {
            return req.save_failed(e, Text::new("save.google_client"));
        }
        log::info!("Keeping the Google client {}", client.id);
        app.oauth.set(client);
        app.post_event("google_client", meta(app));
        req.ok(Value::Null);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Harness;
    use serde_json::json;

    const ID: &str = "123-abc.apps.googleusercontent.com";

    // ------------------------------------------------------------------------
    #[test]
    fn a_client_needs_google_s_id_and_a_secret() {
        let client = shaped(&format!(" {ID} "), " GOCSPX-secret\n").ok().unwrap();
        assert_eq!(
            (client.id.as_str(), client.secret.as_str()),
            (ID, "GOCSPX-secret")
        );
        let code = |given: Result<OAuthClient, (Error, Text)>| given.err().unwrap().1.key;
        assert_eq!(code(shaped("123-abc", "s")), "google.client.id_invalid");
        assert_eq!(code(shaped(ID_END, "s")), "google.client.id_invalid");
        assert_eq!(code(shaped(ID, "  ")), "google.client.secret_missing");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn google_s_file_gives_a_desktop_app_s_client() {
        let installed = json!({ "installed": {
            "client_id": ID, "project_id": "avelor-1", "client_secret": "GOCSPX-s",
            "redirect_uris": ["http://localhost"] } });
        let client = from_google_file(&installed.to_string()).ok().unwrap();
        assert_eq!(
            (client.id.as_str(), client.secret.as_str()),
            (ID, "GOCSPX-s")
        );

        let key = |text: &str| from_google_file(text).err().unwrap().1.key;
        let web = json!({ "web": { "client_id": ID, "client_secret": "s" } });
        assert_eq!(key(&web.to_string()), "google.client.web");
        assert_eq!(key("{\"other\": {}}"), "google.client.not_a_file");
        assert_eq!(key("[1]"), "google.client.not_a_file");
        assert_eq!(key("not json"), "google.client.not_a_file");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_own_client_is_read_from_its_file() {
        let h = Harness::new("own-client-file");
        let dir = h.app.config.dir().to_path_buf();
        assert!(read(&dir).is_none(), "none given yet");
        let client = OAuthClient {
            id: ID.to_string(),
            secret: "GOCSPX-0123456789abcd".to_string(),
        };
        secret(&dir)
            .write(&serde_json::to_string(&client).unwrap())
            .unwrap();
        assert!(read(&dir) == Some(client));
        secret(&dir).write("broken").unwrap();
        assert!(read(&dir).is_none(), "a broken one is logged");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn the_page_gets_the_client_by_where_it_comes_from() {
        let mut h = Harness::new("own-client-meta");
        assert_eq!(meta(&h.app), json!("built-in"));
        h.app.google_client = GoogleClient::Absent;
        assert_eq!(meta(&h.app), Value::Null);
        h.app.google_client = GoogleClient::Own;
        h.app.oauth = OAuth::default();
        assert_eq!(meta(&h.app), Value::Null, "none given yet");
        h.app.oauth.set(OAuthClient {
            id: ID.to_string(),
            secret: "GOCSPX-0123456789abcd".to_string(),
        });
        assert_eq!(meta(&h.app), json!({ "id": ID, "secret": "GOC…abcd" }));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn only_a_build_without_a_client_takes_one() {
        let mut h = Harness::new("own-client-refused");
        let args = json!({ "id": ID, "secret": "s" });
        let sent = h.call("set_google_client", args.clone());
        assert_eq!(sent[0]["err"]["code"], "unsupported", "the build's");
        let sent = h.call("load_google_client", json!({}));
        assert_eq!(sent[0]["err"]["code"], "unsupported");

        h.app.google_client = GoogleClient::Own;
        let sent = h.call("set_google_client", json!({ "id": "x", "secret": "s" }));
        assert_eq!(sent[0]["err"]["code"], "invalid_argument");
        assert_eq!(sent[0]["err"]["text"]["key"], "google.client.id_invalid");
    }
}
