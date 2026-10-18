//! Google OAuth 2.0 for installed apps: the loopback consent flow with
//! PKCE, the authorization-code exchange, and access-token refresh.
//!
//! The app ships one OAuth Desktop client (Google treats desktop client
//! secrets as non-confidential), taken from the build: `AVELOR_GOOGLE_CLIENT_ID`
//! and `AVELOR_GOOGLE_CLIENT_SECRET`, never the sources. A build without them
//! fails, unless it hides Google (the feature `hide-google`): then it has no
//! client and can't sign in (`has_client`). The refresh token is the real credential:
//! `PendingConsent::redeem` hands it out inside a `Client`, and the caller
//! persists it (encrypted, `av_utils::secret`) outside the vault. Access
//! tokens live only in memory, inside their `Client`.
//!
//! No extra crates: the loopback listener is `std::net::TcpListener`, PKCE
//! uses av-utils' SHA-256 and OS randomness, HTTP goes through `client`.
//! Waiting on the loopback listener and the token exchanges block, like
//! every request — the app shell decides which thread runs them.

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use av_http::Method;
use av_utils::cancel::Cancel;
use av_utils::error::{Error, Result};
use av_utils::url::{decode_url_component, encode_url_component};
use av_utils::{base64::encode_base64_url, random::os_random, sha256::sha256};
use serde::Deserialize;

use crate::client::{self, Client};

// ----------------------------------------------------------------------------
const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const REVOKE_ENDPOINT: &str = "https://oauth2.googleapis.com/revoke";
/// The OAuth client's id and secret, from the build's environment; an empty
/// one fails the build as a missing one does (CI hands a missing secret
/// over as an empty variable).
#[cfg(not(feature = "hide-google"))]
const CLIENT: Option<(&str, &str)> = {
    let (id, secret) = (
        env!("AVELOR_GOOGLE_CLIENT_ID"),
        env!("AVELOR_GOOGLE_CLIENT_SECRET"),
    );
    assert!(
        !id.is_empty() && !secret.is_empty(),
        "AVELOR_GOOGLE_CLIENT_ID or AVELOR_GOOGLE_CLIENT_SECRET is empty: set both, \
         or build with the feature hide-google"
    );
    Some((id, secret))
};
#[cfg(feature = "hide-google")]
const CLIENT: Option<(&str, &str)> = None;

/// Prefix of the per-service scopes.
const SCOPE_PREFIX: &str = "https://www.googleapis.com/auth/";
/// Always requested: who the account is.
const IDENTITY_SCOPES: [&str; 3] = ["openid", "email", "profile"];

/// How long the user has to finish signing in in the browser.
const CONSENT_TIMEOUT: Duration = Duration::from_secs(300);

// ----------------------------------------------------------------------------
/// Whether this build can sign in to Google: it was built with an OAuth
/// client.
pub fn has_client() -> bool {
    CLIENT.is_some()
}

fn client_id() -> &'static str {
    CLIENT.map_or("", |(id, _)| id)
}

fn client_secret() -> &'static str {
    CLIENT.map_or("", |(_, secret)| secret)
}

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScopeAccess {
    #[default]
    None,
    ReadOnly,
    Full,
}

// ----------------------------------------------------------------------------
/// Per-service access, requested at consent — or, parsed back from a token
/// response, what the user actually granted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GoogleScope {
    pub calendar: ScopeAccess,
    pub gmail: ScopeAccess,
    pub contacts: ScopeAccess,
    pub tasks: ScopeAccess,
}

/// Scope names per service (read-only, full), in `GoogleScope` field order.
const SERVICE_SCOPES: [(&str, &str); 4] = [
    ("calendar.readonly", "calendar"),
    ("gmail.readonly", "gmail.modify"),
    ("contacts.readonly", "contacts"),
    ("tasks.readonly", "tasks"),
];

impl GoogleScope {
    // ------------------------------------------------------------------------
    fn accesses_mut(&mut self) -> [&mut ScopeAccess; 4] {
        [
            &mut self.calendar,
            &mut self.gmail,
            &mut self.contacts,
            &mut self.tasks,
        ]
    }

    // ------------------------------------------------------------------------
    /// The space-separated OAuth `scope` parameter; identity is always in.
    pub fn to_scope_string(&self) -> String {
        let accesses = [self.calendar, self.gmail, self.contacts, self.tasks];
        let services = accesses
            .into_iter()
            .zip(SERVICE_SCOPES)
            .filter_map(|(access, (read_only, full))| match access {
                ScopeAccess::None => None,
                ScopeAccess::ReadOnly => Some(read_only),
                ScopeAccess::Full => Some(full),
            })
            .map(|s| format!("{SCOPE_PREFIX}{s}"));
        IDENTITY_SCOPES
            .into_iter()
            .map(str::to_string)
            .chain(services)
            .collect::<Vec<_>>()
            .join(" ")
    }

    // ------------------------------------------------------------------------
    /// Parse a token response's `scope` (what was granted); unknown scopes
    /// are ignored.
    pub fn from_scope_string(granted: &str) -> Self {
        let mut scope = Self::default();
        for name in granted
            .split_whitespace()
            .filter_map(|s| s.strip_prefix(SCOPE_PREFIX))
        {
            for (access, (read_only, full)) in scope.accesses_mut().into_iter().zip(SERVICE_SCOPES)
            {
                if name == full {
                    *access = ScopeAccess::Full;
                } else if name == read_only {
                    *access = (*access).max(ScopeAccess::ReadOnly);
                }
            }
        }
        scope
    }
}

// ----------------------------------------------------------------------------
/// A short-lived OAuth access token.
pub(crate) struct AccessToken {
    token: String,
    expiry: Instant,
}

impl AccessToken {
    // ------------------------------------------------------------------------
    /// Valid for `expires_in` seconds, minus a margin for clock and latency.
    fn new(token: String, expires_in: u64) -> Self {
        let expiry = Instant::now() + Duration::from_secs(expires_in.max(60) - 30);
        Self { token, expiry }
    }

    // ------------------------------------------------------------------------
    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    // ------------------------------------------------------------------------
    /// The token, unless it is (about to be) expired.
    pub(crate) fn valid(&self) -> Option<&str> {
        (Instant::now() < self.expiry).then_some(self.token.as_str())
    }
}

// ----------------------------------------------------------------------------
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
    /// Space-separated granted scopes.
    #[serde(default)]
    scope: String,
}

// ----------------------------------------------------------------------------
fn token_request(cancel: &Cancel, form: &[(&str, &str)]) -> Result<TokenResponse> {
    let grant = form
        .iter()
        .find(|(key, _)| *key == "grant_type")
        .map_or("?", |(_, value)| *value);
    let req = av_http::Client::shared()
        .request(Method::Post, TOKEN_ENDPOINT)
        .form(form)
        .cancel_with(cancel);
    let result = client::execute::<TokenResponse>(req);
    match &result {
        Ok(resp) => log::info!(
            "Token request ({grant}): ok, expires in {}s, refresh token {}, scope: {}",
            resp.expires_in,
            if resp.refresh_token.is_some() {
                "issued"
            } else {
                "not issued"
            },
            resp.scope
        ),
        Err(e) => log::warn!("Token request ({grant}) failed: {e}"),
    }
    result
}

// ----------------------------------------------------------------------------
/// A new access token for `refresh_token`; `Error::Expired` if Google no
/// longer accepts it (revoked, or unused for months) — reconnect needed.
pub(crate) fn refresh(cancel: &Cancel, refresh_token: &str) -> Result<AccessToken> {
    let form = [
        ("client_id", client_id()),
        ("client_secret", client_secret()),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];
    match token_request(cancel, &form) {
        Ok(resp) => Ok(AccessToken::new(resp.access_token, resp.expires_in)),
        Err(Error::Http { reason, .. }) if reason == "invalid_grant" => Err(Error::Expired),
        Err(e) => Err(e),
    }
}

// ----------------------------------------------------------------------------
/// Revoke `refresh_token` at Google, and with it every access token issued
/// from it (disconnecting an account).
pub fn revoke(cancel: &Cancel, refresh_token: &str) -> Result<()> {
    let req = av_http::Client::shared()
        .request(Method::Post, REVOKE_ENDPOINT)
        .form(&[("token", refresh_token)])
        .cancel_with(cancel);
    client::execute_empty(req)
}

// ----------------------------------------------------------------------------
// the loopback consent flow
pub struct PendingConsent {
    pub url: String,
    listener: TcpListener,
    verifier: String,
    state: String,
    redirect: String,
    /// Ends the wait, and the requests that redeem the code.
    cancel: Cancel,
}

// ----------------------------------------------------------------------------
/// Phase 1 (UI thread): bind the loopback listener and build the consent
/// URL. The caller opens the URL in the browser, then runs `wait_code` and
/// `redeem` on a worker thread. `login_hint` (an
/// email) preselects the account when reconnecting.
pub fn begin_consent(scope: &GoogleScope, login_hint: Option<&str>) -> Result<PendingConsent> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let redirect = format!("http://127.0.0.1:{port}");

    let mut rnd = [0u8; 32];
    os_random(&mut rnd);
    let verifier = encode_base64_url(&rnd);
    let challenge = encode_base64_url(&sha256(verifier.as_bytes()));
    let mut rnd2 = [0u8; 16];
    os_random(&mut rnd2);
    let state = encode_base64_url(&rnd2);

    let mut url = format!(
        "{AUTH_ENDPOINT}?response_type=code&client_id={}&redirect_uri={}&scope={}\
         &code_challenge={challenge}&code_challenge_method=S256&state={state}\
         &access_type=offline&prompt=consent",
        encode_url_component(client_id()),
        encode_url_component(&redirect),
        encode_url_component(&scope.to_scope_string()),
    );
    if let Some(hint) = login_hint {
        url.push_str(&format!("&login_hint={}", encode_url_component(hint)));
    }
    log::info!(
        "Consent: listening on {redirect}, login hint: {}, scope: {}",
        login_hint.unwrap_or("-"),
        scope.to_scope_string()
    );
    Ok(PendingConsent {
        url,
        listener,
        verifier,
        state,
        redirect,
        cancel: Cancel::new(),
    })
}

// ----------------------------------------------------------------------------
impl PendingConsent {
    // ------------------------------------------------------------------------
    /// Cancelling the returned `Cancel` makes `wait_code` give up (and
    /// return `None`) within a poll interval, and aborts the requests sent
    /// under it (`redeem`).
    pub fn cancel_handle(&self) -> Cancel {
        self.cancel.clone()
    }

    // ------------------------------------------------------------------------
    /// Phase 2a (a worker thread): wait for the browser redirect and
    /// extract the auth code; `None` if cancelled. `Timeout` after five
    /// minutes, `Denied` if the user declined.
    pub fn wait_code(&self) -> Result<Option<String>> {
        wait_for_code(&self.listener, &self.state, CONSENT_TIMEOUT, &self.cancel)
    }

    // ------------------------------------------------------------------------
    /// Phase 2b (a worker thread): exchange the code for tokens. The
    /// returned client carries the refresh token for the caller to persist.
    pub fn redeem(&self, cancel: &Cancel, code: &str) -> Result<Client> {
        let form = [
            ("code", code),
            ("client_id", client_id()),
            ("client_secret", client_secret()),
            ("redirect_uri", self.redirect.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", self.verifier.as_str()),
        ];
        let resp = token_request(cancel, &form)?;
        let refresh_token = resp.refresh_token.ok_or_else(|| Error::InvalidResponse {
            msg: "token response without refresh_token".to_string(),
        })?;
        let scope = GoogleScope::from_scope_string(&resp.scope);
        let access = AccessToken::new(resp.access_token, resp.expires_in);
        Ok(Client::with_access(refresh_token, scope, access))
    }
}

// ----------------------------------------------------------------------------
/// Serve the loopback listener until the OAuth redirect arrives. Browsers
/// open speculative connections that never send a request, and may ask for
/// `/favicon.ico`, so every accepted connection is polled (nonblocking)
/// until one carries the redirect. Only the redirect with *this* consent's
/// `state` ends the wait: a stale one (an older sign-in's tab, reloaded or
/// finished late) is answered and ignored, so it can't close the listener
/// under the live sign-in. The deadline guarantees an abandoned consent
/// never leaks a hung thread; `cancel` ends the wait early.
fn wait_for_code(
    listener: &TcpListener,
    expected_state: &str,
    timeout: Duration,
    cancel: &Cancel,
) -> Result<Option<String>> {
    listener.set_nonblocking(true)?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or_default();
    log::info!("Consent: waiting for the browser redirect on port {port}");
    let started = Instant::now();
    let deadline = started + timeout;
    let mut connections: Vec<TcpStream> = Vec::new();
    while Instant::now() < deadline {
        if cancel.is_cancelled() {
            log::info!("Consent: cancelled after {:.1?}", started.elapsed());
            return Ok(None);
        }
        loop {
            match listener.accept() {
                Ok((stream, peer)) => {
                    log::debug!("Consent: accepted a connection from {peer}");
                    match stream.set_nonblocking(true) {
                        Ok(()) => connections.push(stream),
                        Err(e) => log::warn!("Consent: dropping the connection from {peer}: {e}"),
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                // The browser dropped a connection before it was accepted
                // (Windows reports that here): try again next round.
                Err(e) if is_transient(e.kind()) => {
                    log::info!("Consent: accept failed ({e}); still waiting");
                    break;
                }
                Err(e) => {
                    log::warn!("Consent: the listener on port {port} failed: {e}");
                    return Err(e.into());
                }
            }
        }

        let mut i = 0;
        while i < connections.len() {
            let mut peek = [0u8; 1];
            match connections[i].peek(&mut peek) {
                Err(e) if e.kind() == ErrorKind::WouldBlock => i += 1,
                Ok(0) => {
                    log::debug!("Consent: a connection closed without a request");
                    drop(connections.swap_remove(i));
                }
                Err(e) => {
                    log::debug!("Consent: a connection failed before a request: {e}");
                    drop(connections.swap_remove(i));
                }
                Ok(_) => {
                    let stream = connections.swap_remove(i);
                    if let Some(outcome) = serve_request(stream, expected_state) {
                        log::info!("Consent: done after {:.1?}", started.elapsed());
                        return outcome.map(Some);
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    log::warn!("Consent: no redirect within {}s", timeout.as_secs());
    Err(Error::Timeout)
}

// ----------------------------------------------------------------------------
/// Read and answer one request on the loopback listener; `Some` if it was
/// this consent's redirect, which ends the wait.
fn serve_request(mut stream: TcpStream, expected_state: &str) -> Option<Result<String>> {
    let line = match read_request_line(&mut stream) {
        Ok(line) => line,
        Err(e) => {
            log::info!("Consent: reading a request failed: {e}");
            return None;
        }
    };
    let request = describe_request(&line);
    match redirect_outcome(&line, expected_state) {
        None => {
            log::info!("Consent: {request}: not a redirect, answered 404");
            respond(&mut stream, "404 Not Found", "");
            None
        }
        Some(Err(Error::Aborted)) => {
            log::info!(
                "Consent: {request}: another sign-in's redirect (state mismatch), answered as outdated"
            );
            respond(&mut stream, "200 OK", STALE_PAGE);
            None
        }
        Some(outcome) => {
            let page = match &outcome {
                Ok(_) => {
                    log::info!("Consent: {request}: the redirect, with an auth code");
                    CONNECTED_PAGE
                }
                Err(e) => {
                    log::warn!("Consent: {request}: the redirect, without an auth code ({e:?})");
                    FAILED_PAGE
                }
            };
            respond(&mut stream, "200 OK", page);
            Some(outcome)
        }
    }
}

// ----------------------------------------------------------------------------
/// A request line for the log, without parameter values (the redirect's
/// carry the auth code), except Google's `error`: `GET / [code, state]`.
fn describe_request(line: &str) -> String {
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("?");
    let target = parts.next().unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let params: Vec<String> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some(("error", value)) => format!("error={}", decode_url_component(value)),
            Some((name, _)) => name.to_string(),
            None => pair.to_string(),
        })
        .collect();
    if params.is_empty() {
        format!("{method} {path}")
    } else {
        format!("{method} {path} [{}]", params.join(", "))
    }
}

// ----------------------------------------------------------------------------
const CONNECTED_PAGE: &str = "<html><body style=\"font-family:sans-serif\">\
    <h3>Avelor is connected.</h3>You can close this tab.</body></html>";
const FAILED_PAGE: &str = "<html><body style=\"font-family:sans-serif\">\
    <h3>Sign-in failed.</h3>Return to Avelor and try again.</body></html>";
const STALE_PAGE: &str = "<html><body style=\"font-family:sans-serif\">\
    <h3>This sign-in is outdated.</h3>A newer one is waiting: finish it in its own \
    tab, or start again in Avelor.</body></html>";

// ----------------------------------------------------------------------------
/// Accept errors that concern one connection, not the listener: the peer
/// reset or dropped it before it was accepted.
fn is_transient(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::Interrupted
    )
}

// ----------------------------------------------------------------------------
/// The request line (`GET /?code=…&state=… HTTP/1.1`) of a connection that
/// started sending; read blocking, the rest is on its way. Reads the whole
/// head (up to the blank line; a GET has no body): closing a socket with
/// unread input makes Windows reset the connection, and the browser may
/// then drop our answer and show a blank page.
fn read_request_line(stream: &mut TcpStream) -> std::io::Result<String> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < MAX_REQUEST_HEAD {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            break;
        }
        head.extend_from_slice(&buf[..n]);
    }
    log::debug!("Consent: read a request head of {} bytes", head.len());
    let head = String::from_utf8_lossy(&head);
    Ok(head.lines().next().unwrap_or_default().to_string())
}

/// Larger request heads are cut off (and their rest drained by `respond`).
const MAX_REQUEST_HEAD: usize = 64 * 1024;

// ----------------------------------------------------------------------------
/// The auth code from the redirect's request line; `None` if the request
/// isn't the redirect at all (no `state`).
fn redirect_outcome(request_line: &str, expected_state: &str) -> Option<Result<String>> {
    let target = request_line.split_whitespace().nth(1)?;
    let (_, query) = target.split_once('?')?;
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        match pair.split_once('=') {
            Some(("code", v)) => code = Some(decode_url_component(v)),
            Some(("state", v)) => state = Some(decode_url_component(v)),
            _ => {}
        }
    }
    let state = state?;
    Some(if state != expected_state {
        Err(Error::Aborted)
    } else {
        // Without a code, Google reports `error=access_denied`.
        code.ok_or(Error::Denied)
    })
}

// ----------------------------------------------------------------------------
/// Answer and close gracefully: send, shut down our side (FIN), then drain
/// whatever the browser still sends until it closes too, so the socket
/// never closes with unread input (which would reset the connection).
fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let answer = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    if let Err(e) = stream
        .write_all(answer.as_bytes())
        .and_then(|()| stream.flush())
    {
        log::info!("Consent: sending the answer failed: {e}");
        return;
    }
    let _ = stream.shutdown(Shutdown::Write);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 1024];
    let mut drained = 0;
    while let Ok(n @ 1..) = stream.read(&mut buf) {
        drained += n;
    }
    if drained > 0 {
        log::debug!("Consent: drained {drained} more bytes after answering");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn scope_string_always_has_identity_and_round_trips() {
        assert_eq!(
            GoogleScope::default().to_scope_string(),
            "openid email profile"
        );

        let scope = GoogleScope {
            calendar: ScopeAccess::Full,
            gmail: ScopeAccess::ReadOnly,
            ..GoogleScope::default()
        };
        let s = scope.to_scope_string();
        assert_eq!(
            s,
            "openid email profile https://www.googleapis.com/auth/calendar \
             https://www.googleapis.com/auth/gmail.readonly"
        );
        assert_eq!(GoogleScope::from_scope_string(&s), scope);

        // Google echoes identity scopes in URL form; a full scope wins over
        // its read-only sibling in either order.
        let granted = "https://www.googleapis.com/auth/userinfo.email openid \
            https://www.googleapis.com/auth/contacts https://www.googleapis.com/auth/contacts.readonly";
        let parsed = GoogleScope::from_scope_string(granted);
        assert_eq!(parsed.contacts, ScopeAccess::Full);
        assert_eq!(parsed.calendar, ScopeAccess::None);
    }

    // ------------------------------------------------------------------------
    #[test]
    fn consent_url_carries_pkce_state_and_scope() {
        let pending = begin_consent(&GoogleScope::default(), None).unwrap();
        assert!(pending.url.contains("code_challenge_method=S256"));
        assert!(!pending.url.contains("login_hint"));
        let hinted = begin_consent(&GoogleScope::default(), Some("a@b.c")).unwrap();
        assert!(hinted.url.contains("&login_hint=a%40b.c"));
        assert!(pending.url.contains("access_type=offline"));
        assert!(pending.url.contains("scope=openid%20email%20profile"));
        assert!(
            pending
                .url
                .contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A")
        );
        assert!(!pending.verifier.is_empty() && !pending.state.is_empty());
    }

    // ------------------------------------------------------------------------
    #[test]
    fn redirect_outcomes() {
        let line = |q: &str| format!("GET /{q} HTTP/1.1");
        assert!(redirect_outcome(&line("favicon.ico"), "s1").is_none());
        assert!(
            redirect_outcome(&line("?code=c"), "s1").is_none(),
            "no state: not ours"
        );
        assert_eq!(
            redirect_outcome(&line("?code=a%2Fb&state=s1"), "s1")
                .unwrap()
                .unwrap(),
            "a/b"
        );
        assert!(matches!(
            redirect_outcome(&line("?code=c&state=s2"), "s1"),
            Some(Err(Error::Aborted))
        ));
        assert!(matches!(
            redirect_outcome(&line("?error=access_denied&state=s1"), "s1"),
            Some(Err(Error::Denied))
        ));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn request_descriptions_keep_secrets_out_of_the_log() {
        let line = "GET /?code=4%2F0secret&state=s1&scope=openid HTTP/1.1";
        let described = describe_request(line);
        assert_eq!(described, "GET / [code, state, scope]");
        assert!(!described.contains("secret") && !described.contains("s1"));
        assert_eq!(
            describe_request("GET /?error=access_denied&state=s1 HTTP/1.1"),
            "GET / [error=access_denied, state]"
        );
        assert_eq!(
            describe_request("GET /favicon.ico HTTP/1.1"),
            "GET /favicon.ico"
        );
    }

    // ------------------------------------------------------------------------
    #[test]
    fn loopback_skips_idle_unrelated_and_stale_requests() {
        let pending = begin_consent(&GoogleScope::default(), None).unwrap();
        let addr = pending.listener.local_addr().unwrap();
        let state = pending.state.clone();
        let request = move |line: String| {
            let mut stream = TcpStream::connect(addr).unwrap();
            write!(stream, "{line}\r\nHost: x\r\n\r\n").unwrap();
            let mut answer = String::new();
            stream.read_to_string(&mut answer).unwrap();
            answer
        };
        let browser = std::thread::spawn(move || {
            drop(TcpStream::connect(addr).unwrap()); // closed without a request
            let _idle = TcpStream::connect(addr).unwrap(); // speculative, stays silent
            let favicon = request("GET /favicon.ico HTTP/1.1".into());
            // An older sign-in's redirect, e.g. its tab reloaded: answered,
            // but the wait goes on.
            let stale = request("GET /?code=old&state=stale HTTP/1.1".into());
            // Longer than one read, like a real redirect with its headers:
            // the answer must survive the close (no reset from unread input).
            let padding = "a".repeat(4000);
            let live = request(format!(
                "GET /?code=the-code&state={state} HTTP/1.1\r\nX-Padding: {padding}"
            ));
            (favicon, stale, live)
        });
        let timeout = Duration::from_secs(10);
        let code = wait_for_code(&pending.listener, &pending.state, timeout, &pending.cancel);
        assert_eq!(code.unwrap().as_deref(), Some("the-code"));
        let (favicon, stale, live) = browser.join().unwrap();
        assert!(favicon.starts_with("HTTP/1.1 404"));
        assert!(stale.contains("This sign-in is outdated."));
        assert!(live.contains("Avelor is connected."));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn cancelling_ends_the_wait_with_none() {
        let pending = begin_consent(&GoogleScope::default(), None).unwrap();
        let cancel = pending.cancel_handle();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            cancel.cancel();
        });
        let started = Instant::now();
        assert!(pending.wait_code().unwrap().is_none());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "not the 5-minute deadline"
        );
        canceller.join().unwrap();
    }
}
