//! The `Client` of one AI account, and what every provider module sends its
//! requests through.
//!
//! Provider modules build a request, send it, and turn the answer into the
//! shared types. `Client` runs them under the caller's `Cancel`. HTTP failures
//! become `Error::Http` here, whatever the provider's error body looks like.

use av_http::{Method, Request, Response};
use av_utils::cancel::Cancel;
use av_utils::error::Result;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::types::{Auth, ModelInfo, Provider};
use crate::usage::{Price, list_price};
use crate::{anthropic, gemini, mistral, ollama, openai};

// ----------------------------------------------------------------------------
/// `$provider::$f(args…)` for the client's provider.
macro_rules! by_provider {
    ($provider:expr, $f:ident($($arg:expr),*)) => {
        match $provider {
            Provider::Anthropic => anthropic::$f($($arg),*),
            Provider::OpenAi => openai::$f($($arg),*),
            Provider::Mistral => mistral::$f($($arg),*),
            Provider::Gemini => gemini::$f($($arg),*),
            Provider::Ollama => ollama::$f($($arg),*),
        }
    };
}

// ----------------------------------------------------------------------------
/// One AI account: a provider, its key, and where its API lives. Cheap to
/// clone.
#[derive(Clone)]
pub struct Client {
    provider: Provider,
    api_key: Option<String>,
    base_url: String,
}

impl Client {
    // ------------------------------------------------------------------------
    /// A client of `provider`'s own API. `api_key` may be `None` for a local
    /// Ollama only.
    pub fn new(provider: Provider, api_key: Option<String>) -> Self {
        Self {
            provider,
            api_key: api_key.filter(|key| !key.is_empty()),
            base_url: provider.spec().default_url.to_string(),
        }
    }

    // ------------------------------------------------------------------------
    /// Talk to another host, e.g. a remote Ollama (`http://host:11434`) or
    /// ollama.com (`https://ollama.com`).
    pub fn with_base_url(mut self, url: &str) -> Self {
        self.base_url = url.trim_end_matches('/').to_string();
        self
    }

    // ------------------------------------------------------------------------
    /// The price of `model` for this account: free for a model on this
    /// computer, else the list price.
    pub fn price(&self, model: &str) -> Option<Price> {
        let local = self.provider == Provider::Ollama && is_loopback(&self.base_url);
        list_price(self.provider, model).or(local.then_some(Price::FREE))
    }

    // ------------------------------------------------------------------------
    /// The models this account can chat with.
    pub fn models(&self, cancel: &Cancel) -> Result<Vec<ModelInfo>> {
        by_provider!(self.provider, models(self, cancel))
    }

    // ------------------------------------------------------------------------
    pub(crate) fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    // ------------------------------------------------------------------------
    /// `path` (with a leading `/`) under the base URL.
    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    // ------------------------------------------------------------------------
    /// A request to `target` -- a path under the base URL (`/models`), or a
    /// URL as it is (an upload's) -- under `cancel`, with the provider's
    /// headers and the key the way it takes one (`ProviderSpec`).
    pub(crate) fn request(
        &self,
        cancel: &Cancel,
        method: Method,
        target: &str,
    ) -> Request<'static> {
        let url = if target.starts_with('/') {
            self.url(target)
        } else {
            target.to_string()
        };
        let spec = self.provider.spec();
        let mut req = av_http::Client::shared().request(method, &url);
        for (name, value) in spec.headers {
            req = req.header(name, value);
        }
        req = req.cancel_with(cancel);
        match (self.api_key(), spec.auth) {
            (Some(key), Auth::Bearer) => req.bearer_auth(key),
            (Some(key), Auth::Header(name)) => req.header(name, key),
            (None, _) => req,
        }
    }
}

// ----------------------------------------------------------------------------
/// Send `req` and decode its JSON answer; a non-2xx status is `Error::Http`.
pub(crate) fn send_json<T: DeserializeOwned>(req: Request<'static>) -> Result<T> {
    open(req)?.json()
}

// ----------------------------------------------------------------------------
/// Send `req` and return the response to read (a stream, or headers); a
/// non-2xx status is `Error::Http` with the provider's reason
/// (`Request::send_logged`, which logs method, host and path only).
pub(crate) fn open(req: Request<'static>) -> Result<Response> {
    req.send_logged(reason)
}

// ----------------------------------------------------------------------------
/// The provider's error type or code and its message, from a failed
/// response's body (`error_parts`).
fn reason(body: &[u8]) -> Option<(String, String)> {
    error_parts(&serde_json::from_slice(body).ok()?)
}

// ----------------------------------------------------------------------------
/// The error type/code and the message of a provider's error body, if it is
/// one:
/// - Anthropic `{"type": "error", "error": {"type", "message"}}`
/// - OpenAI `{"error": {"message", "type", "code"}}`, or in a stream
///   `{"type": "error", "code", "message"}`
/// - Gemini `{"error": {"code", "message", "status"}}`
/// - Mistral `{"object": "error", "message", "type", "code"}`, or
///   `{"detail": …}`
/// - Ollama `{"error": "…"}`
fn error_parts(json: &Value) -> Option<(String, String)> {
    let error = &json["error"];
    if let Some(msg) = error.as_str() {
        return Some((String::new(), msg.to_string()));
    }
    if error.is_object() {
        let reason = first_str(&[&error["status"], &error["code"], &error["type"]]);
        return Some((reason, text(&error["message"])));
    }
    let is_error = json["type"] == "error" || json["object"] == "error";
    if is_error || !json["detail"].is_null() {
        // `type` is the kind of error, unless it only says "error".
        let kind = json["type"].as_str().filter(|kind| *kind != "error");
        let reason = kind.map_or_else(|| first_str(&[&json["code"]]), str::to_string);
        let msg = if json["message"].is_null() {
            text(&json["detail"])
        } else {
            text(&json["message"])
        };
        return Some((reason, msg));
    }
    None
}

// ----------------------------------------------------------------------------
/// The first non-empty string (numeric codes say less than the next field).
fn first_str(values: &[&Value]) -> String {
    values
        .iter()
        .filter_map(|v| v.as_str())
        .find(|s| !s.is_empty())
        .unwrap_or_default()
        .to_string()
}

// ----------------------------------------------------------------------------
/// A string as is, anything else (a validation error list) as JSON.
fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

// ----------------------------------------------------------------------------
/// A string field, empty if absent.
pub(crate) fn string(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

// ----------------------------------------------------------------------------
/// A count: a number, or an int64 as a string (Google); 0 if absent.
pub(crate) fn count(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse().ok())
        .unwrap_or(0)
}

// ----------------------------------------------------------------------------
/// A token limit a provider reports; `None` if absent or 0 (unknown).
pub(crate) fn limit(value: &Value) -> Option<u64> {
    Some(count(value)).filter(|n| *n > 0)
}

// ----------------------------------------------------------------------------
/// Whether `url` points at this computer.
fn is_loopback(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or_default();
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.split(':').next().unwrap_or_default(),
    };
    host.eq_ignore_ascii_case("localhost") || host == "::1" || host.starts_with("127.")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    /// The request `provider`'s client makes for `target` (a path under the
    /// base URL, or an absolute URL): its method, URL and headers.
    fn built(provider: Provider, key: Option<&str>, target: &str) -> (Method, String, String) {
        let client =
            Client::new(provider, key.map(str::to_string)).with_base_url("https://api.test/v1");
        let req = client.request(&Cancel::new(), Method::Post, target);
        (
            req.method(),
            req.url().to_string(),
            req.headers().to_string(),
        )
    }

    // ------------------------------------------------------------------------
    /// What every provider sends, with and without a key: the golden table
    /// the one request builder must keep (`architecture.md#crates`).
    #[test]
    fn requests_of_every_provider() {
        let url = |path: &str| format!("https://api.test/v1{path}");
        let cases = [
            (
                Provider::Anthropic,
                Some("k"),
                "/messages",
                "anthropic-version: 2023-06-01\r\nx-api-key: k\r\n",
            ),
            (
                Provider::Anthropic,
                None,
                "/messages",
                "anthropic-version: 2023-06-01\r\n",
            ),
            (
                Provider::OpenAi,
                Some("k"),
                "/responses",
                "Authorization: Bearer k\r\n",
            ),
            (Provider::OpenAi, None, "/responses", ""),
            (
                Provider::Mistral,
                Some("k"),
                "/chat/completions",
                "Authorization: Bearer k\r\n",
            ),
            (Provider::Mistral, None, "/chat/completions", ""),
            (
                Provider::Gemini,
                Some("k"),
                "/models/m:generateContent",
                "x-goog-api-key: k\r\n",
            ),
            (Provider::Gemini, None, "/models/m:generateContent", ""),
            (
                Provider::Ollama,
                Some("k"),
                "/api/chat",
                "Authorization: Bearer k\r\n",
            ),
            (Provider::Ollama, None, "/api/chat", ""),
        ];
        for (provider, key, path, headers) in cases {
            let expected = (Method::Post, url(path), headers.to_string());
            assert_eq!(built(provider, key, path), expected, "{provider} {key:?}");
        }

        // Gemini's resumable upload goes to the URL its first answer named.
        let upload = "https://upload.test/upload/v1beta/files?upload_id=1";
        let expected = (
            Method::Post,
            upload.to_string(),
            "x-goog-api-key: k\r\n".to_string(),
        );
        assert_eq!(built(Provider::Gemini, Some("k"), upload), expected);
    }

    // ------------------------------------------------------------------------
    fn http_error(body: &str) -> (String, String) {
        reason(body.as_bytes()).expect("an error body")
    }

    // ------------------------------------------------------------------------
    #[test]
    fn error_bodies_of_every_provider() {
        let anthropic = r#"{"type":"error","error":{"type":"overloaded_error",
            "message":"Overloaded"},"request_id":"req_1"}"#;
        assert_eq!(
            http_error(anthropic),
            ("overloaded_error".into(), "Overloaded".into())
        );

        let openai = r#"{"error":{"message":"Incorrect API key provided.",
            "type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#;
        assert_eq!(http_error(openai).0, "invalid_api_key");
        let openai_no_code = r#"{"error":{"message":"No.","type":"invalid_request_error",
            "param":null,"code":null}}"#;
        assert_eq!(http_error(openai_no_code).0, "invalid_request_error");

        let gemini = r#"{"error":{"code":400,"message":"API key not valid.",
            "status":"INVALID_ARGUMENT"}}"#;
        assert_eq!(
            http_error(gemini),
            ("INVALID_ARGUMENT".into(), "API key not valid.".into())
        );

        let mistral = r#"{"object":"error","message":"Invalid model: x",
            "type":"invalid_model","param":null,"code":"1500"}"#;
        assert_eq!(
            http_error(mistral),
            ("invalid_model".into(), "Invalid model: x".into())
        );
        let mistral_detail = r#"{"detail":"Unauthorized"}"#;
        assert_eq!(
            http_error(mistral_detail),
            (String::new(), "Unauthorized".into())
        );

        let ollama = r#"{"error":"model \"x\" not found, try pulling it first"}"#;
        assert_eq!(
            http_error(ollama),
            (
                String::new(),
                "model \"x\" not found, try pulling it first".into()
            )
        );

        assert_eq!(
            reason(
                b" <html>Bad Gateway</html>
"
            ),
            None,
            "the body is the message"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn loopback_hosts_are_local() {
        assert!(is_loopback("http://localhost:11434"));
        assert!(is_loopback("http://127.0.0.1:11434/"));
        assert!(is_loopback("http://[::1]:11434"));
        assert!(!is_loopback("https://ollama.com"));
        assert!(!is_loopback("http://192.168.1.5:11434"));
    }
}
