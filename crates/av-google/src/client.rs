//! One connected Google account: its refresh token, its in-memory access
//! token, and the request helpers every service module goes through.
//!
//! This is the only place that sends requests (through av-http) and turns
//! HTTP failures into `av_utils::error::Error`. What a status code *means*
//! (a sync token expired, an etag conflict, …) is up to the service module
//! owning the endpoint, which matches on `Error::Http { status, reason, .. }`.

use std::sync::{Mutex, PoisonError};

use av_http::{Method, Request};
use av_utils::cancel::Cancel;
use av_utils::error::Result;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::auth::{self, AccessToken, GoogleScope};

// ----------------------------------------------------------------------------
/// A connected account. Cheap to share (`Arc<Client>`): the cached access
/// token is refreshed on demand by whichever request finds it stale.
pub struct Client {
    refresh_token: String,
    scope: GoogleScope,
    access: Mutex<Option<AccessToken>>,
}

impl Client {
    // ------------------------------------------------------------------------
    /// A client for a stored account: the refresh token and the scope that
    /// was granted with it, as persisted by the caller.
    pub fn new(refresh_token: String, scope: GoogleScope) -> Self {
        Self {
            refresh_token,
            scope,
            access: Mutex::new(None),
        }
    }

    // ------------------------------------------------------------------------
    /// A freshly consented account, with the access token that came along.
    pub(crate) fn with_access(
        refresh_token: String,
        scope: GoogleScope,
        access: AccessToken,
    ) -> Self {
        Self {
            refresh_token,
            scope,
            access: Mutex::new(Some(access)),
        }
    }

    // ------------------------------------------------------------------------
    /// The long-lived credential; the caller persists it (encrypted, see
    /// `av_utils::secret`) and passes it back to `Client::new`.
    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    // ------------------------------------------------------------------------
    /// What the user actually granted (they can untick scopes at consent).
    pub fn scope(&self) -> &GoogleScope {
        &self.scope
    }

    // ------------------------------------------------------------------------
    /// A valid access token, refreshed through the refresh token when the
    /// cached one is missing or stale. `Error::Expired` if the account has
    /// to be reconnected.
    pub(crate) fn access_token(&self, cancel: &Cancel) -> Result<String> {
        let cached = self
            .lock_access()
            .as_ref()
            .and_then(AccessToken::valid)
            .map(str::to_string);
        if let Some(token) = cached {
            return Ok(token);
        }
        log::debug!("Access token missing or expired; refreshing");
        let fresh = auth::refresh(cancel, &self.refresh_token)?;
        let token = fresh.token().to_string();
        *self.lock_access() = Some(fresh);
        Ok(token)
    }

    // ------------------------------------------------------------------------
    fn lock_access(&self) -> std::sync::MutexGuard<'_, Option<AccessToken>> {
        // A poisoned cache holds at worst a stale token, which is refreshed.
        self.access.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // ------------------------------------------------------------------------
    /// An authorized request under `cancel`; add query, body, and headers,
    /// then `execute`.
    pub(crate) fn request(
        &self,
        cancel: &Cancel,
        method: Method,
        url: &str,
    ) -> Result<Request<'static>> {
        let token = self.access_token(cancel)?;
        Ok(av_http::Client::shared()
            .request(method, url)
            .bearer_auth(&token)
            .cancel_with(cancel))
    }

    // ------------------------------------------------------------------------
    pub(crate) fn get<T: DeserializeOwned>(
        &self,
        cancel: &Cancel,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        execute(self.request(cancel, Method::Get, url)?.query(query))
    }

    // ------------------------------------------------------------------------
    /// Walk a list endpoint to exhaustion. Page requests repeat the original
    /// query verbatim (API contract); only `pageToken` is added.
    pub(crate) fn list_all<P: Page>(
        &self,
        cancel: &Cancel,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<Listed<P::Item>> {
        let mut items = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut req = self.request(cancel, Method::Get, url)?.query(query);
            if let Some(token) = &page_token {
                req = req.query(&[("pageToken", token.as_str())]);
            }
            let (page_items, next_page_token, next_sync_token) = execute::<P>(req)?.into_parts();
            items.extend(page_items);
            match next_page_token {
                Some(token) => page_token = Some(token),
                None => {
                    return Ok(Listed {
                        items,
                        next_sync_token,
                    });
                }
            }
        }
    }
}

// ----------------------------------------------------------------------------
/// One page of a Google list endpoint.
pub(crate) trait Page: DeserializeOwned {
    type Item;
    /// The page's items, `nextPageToken`, and `nextSyncToken` (last page
    /// of a syncable list only).
    fn into_parts(self) -> (Vec<Self::Item>, Option<String>, Option<String>);
}

// ----------------------------------------------------------------------------
/// Everything a list walk produced.
pub struct Listed<T> {
    pub items: Vec<T>,
    /// For the next incremental list (syncable endpoints only).
    pub next_sync_token: Option<String>,
}

// ----------------------------------------------------------------------------
/// Send `req` and decode its JSON answer; any non-2xx status becomes
/// `Error::Http` (`Request::send_logged`). Blocks; the `Cancel` the request
/// was built under aborts it.
pub(crate) fn execute<T: DeserializeOwned>(req: Request<'static>) -> Result<T> {
    req.send_logged(reason)?.json()
}

// ----------------------------------------------------------------------------
/// Send `req`, for endpoints answering without a (useful) body.
pub(crate) fn execute_empty(req: Request<'static>) -> Result<()> {
    req.send_logged(reason)?.bytes().map(drop)
}

// ----------------------------------------------------------------------------
/// Google's error bodies: `{"error": {...}}` from the APIs, or OAuth's
/// `{"error": "invalid_grant", "error_description": "..."}`.
#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorField,
    #[serde(default)]
    error_description: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ErrorField {
    Api(ApiError),
    OAuth(String),
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ApiError {
    message: String,
    /// Canonical code, e.g. `FAILED_PRECONDITION`.
    status: String,
    /// Legacy detail list (Calendar, Gmail): `[{reason: "conditionNotMet"}]`.
    errors: Vec<ErrorReason>,
    /// `google.rpc.ErrorInfo` & co (People): `[{reason: "EXPIRED_SYNC_TOKEN"}]`.
    details: Vec<ErrorReason>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ErrorReason {
    reason: String,
}

// ----------------------------------------------------------------------------
/// The reason and message of a failed response's body
/// (`Request::send_logged`): the most specific machine-readable reason the
/// body offers, and its human-readable message.
fn reason(body: &[u8]) -> Option<(String, String)> {
    match serde_json::from_slice::<ErrorBody>(body).ok()? {
        ErrorBody {
            error: ErrorField::Api(api),
            ..
        } => {
            let reason = api
                .details
                .iter()
                .chain(&api.errors)
                .map(|r| r.reason.as_str())
                .find(|r| !r.is_empty())
                .unwrap_or(&api.status)
                .to_string();
            Some((reason, api.message))
        }
        ErrorBody {
            error: ErrorField::OAuth(code),
            error_description,
        } => Some((code, error_description)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_error(body: &str) -> (String, String) {
        reason(body.as_bytes()).expect("an error body")
    }

    // ------------------------------------------------------------------------
    #[test]
    fn error_bodies_yield_the_most_specific_reason() {
        let calendar = r#"{"error": {"code": 412, "message": "Precondition Failed",
            "errors": [{"domain": "global", "reason": "conditionNotMet"}]}}"#;
        assert_eq!(
            http_error(calendar),
            ("conditionNotMet".into(), "Precondition Failed".into())
        );

        let people = r#"{"error": {"code": 400, "message": "Sync token is expired.",
            "status": "FAILED_PRECONDITION", "details": [{"@type":
            "type.googleapis.com/google.rpc.ErrorInfo", "reason": "EXPIRED_SYNC_TOKEN"}]}}"#;
        assert_eq!(http_error(people).0, "EXPIRED_SYNC_TOKEN");

        let status_only =
            r#"{"error": {"code": 403, "message": "No.", "status": "PERMISSION_DENIED"}}"#;
        assert_eq!(http_error(status_only).0, "PERMISSION_DENIED");

        let oauth = r#"{"error": "invalid_grant", "error_description": "Token has been revoked."}"#;
        assert_eq!(
            http_error(oauth),
            ("invalid_grant".into(), "Token has been revoked.".into())
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
}
