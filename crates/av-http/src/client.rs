//! The session, and requests on it.

use std::fmt;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use av_utils::base64::encode_base64_url;
use av_utils::cancel::Cancel;
use av_utils::error::{Error, Result};
use av_utils::random::os_random;
use av_utils::url::encode_url_component;
use serde::Serialize;

use crate::backend;
use crate::response::Response;

// ----------------------------------------------------------------------------
/// A session: the connection pool, with the system's proxy settings. Share
/// one per process.
pub struct Client {
    pub(crate) session: backend::Session,
    /// The default timeout of every phase.
    pub(crate) timeout: Duration,
}

impl Client {
    // ------------------------------------------------------------------------
    /// A session sending `user_agent`. `timeout` limits each phase of a
    /// request (resolve, connect, send, each read), not the request as a
    /// whole; zero means none.
    pub fn new(user_agent: &str, timeout: Duration) -> Result<Self> {
        Ok(Self {
            session: backend::Session::new(user_agent)?,
            timeout,
        })
    }

    // ------------------------------------------------------------------------
    /// The process's one session, shared by every API the app talks to
    /// (Google, the AI providers): one connection pool. Its timeout limits
    /// each phase of a request to 60 s.
    pub fn shared() -> &'static Client {
        static SHARED: OnceLock<Client> = OnceLock::new();
        SHARED.get_or_init(|| {
            let agent = concat!("avelor/", env!("CARGO_PKG_VERSION"));
            Client::new(agent, Duration::from_secs(60)).expect("failed to open the HTTP session")
        })
    }

    // ------------------------------------------------------------------------
    pub fn request(&self, method: Method, url: &str) -> Request<'_> {
        Request {
            client: self,
            method,
            url: url.to_string(),
            headers: String::new(),
            body: Vec::new(),
            timeout: None,
            cancel: None,
        }
    }
}

// ----------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    // ------------------------------------------------------------------------
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ----------------------------------------------------------------------------
/// A request being built; `send` runs it.
pub struct Request<'a> {
    pub(crate) client: &'a Client,
    pub(crate) method: Method,
    pub(crate) url: String,
    /// `Name: value\r\n` lines.
    pub(crate) headers: String,
    pub(crate) body: Vec<u8>,
    pub(crate) timeout: Option<Duration>,
    cancel: Option<Cancel>,
}

impl Request<'_> {
    // ------------------------------------------------------------------------
    /// Append `pairs` to the URL's query, percent-encoded.
    pub fn query(mut self, pairs: &[(&str, &str)]) -> Self {
        if !pairs.is_empty() {
            self.url
                .push(if self.url.contains('?') { '&' } else { '?' });
            self.url.push_str(&encode_pairs(pairs));
        }
        self
    }

    // ------------------------------------------------------------------------
    pub fn header(mut self, name: &str, value: &str) -> Self {
        debug_assert!(
            !format!("{name}{value}").contains(['\r', '\n']),
            "line break in header {name}"
        );
        self.headers.push_str(&format!("{name}: {value}\r\n"));
        self
    }

    // ------------------------------------------------------------------------
    pub fn bearer_auth(self, token: &str) -> Self {
        self.header("Authorization", &format!("Bearer {token}"))
    }

    // ------------------------------------------------------------------------
    /// An `application/x-www-form-urlencoded` body.
    pub fn form(self, pairs: &[(&str, &str)]) -> Self {
        let body = encode_pairs(pairs).into_bytes();
        self.body(body, "application/x-www-form-urlencoded")
    }

    // ------------------------------------------------------------------------
    pub fn json<T: Serialize + ?Sized>(self, value: &T) -> Result<Self> {
        let body = serde_json::to_vec(value)?;
        Ok(self.body(body, "application/json"))
    }

    // ------------------------------------------------------------------------
    /// A `multipart/form-data` body (file uploads).
    pub fn multipart(self, parts: &[FormPart]) -> Self {
        let mut random = [0u8; 18];
        os_random(&mut random);
        let boundary = format!("avelor-{}", encode_base64_url(&random));
        let mut body = Vec::new();
        for part in parts {
            let mut head = format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"",
                escape_field(part.name)
            );
            if let Some(filename) = part.filename {
                head.push_str(&format!("; filename=\"{}\"", escape_field(filename)));
            }
            head.push_str("\r\n");
            if let Some(content_type) = part.content_type {
                head.push_str(&format!("Content-Type: {content_type}\r\n"));
            }
            head.push_str("\r\n");
            body.extend_from_slice(head.as_bytes());
            body.extend_from_slice(part.data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        self.body(body, &format!("multipart/form-data; boundary={boundary}"))
    }

    // ------------------------------------------------------------------------
    pub fn body(mut self, body: Vec<u8>, content_type: &str) -> Self {
        self.body = body;
        self.header("Content-Type", content_type)
    }

    // ------------------------------------------------------------------------
    /// Override the client's timeout for sending and for each read, which
    /// includes waiting for the response headers: long for an answer that
    /// takes a while to start, zero for none.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    // ------------------------------------------------------------------------
    /// Let `cancel` abort this request, while it is sent or while its
    /// response is read: a request blocked in WinHTTP (libsoup on Linux)
    /// fails with `Error::Cancelled` at once, and so does every later
    /// request attached to it.
    pub fn cancel_with(mut self, cancel: &Cancel) -> Self {
        self.cancel = Some(cancel.clone());
        self
    }

    // ------------------------------------------------------------------------
    pub fn method(&self) -> Method {
        self.method
    }

    // ------------------------------------------------------------------------
    /// The URL, including the query.
    pub fn url(&self) -> &str {
        &self.url
    }

    // ------------------------------------------------------------------------
    /// The headers set so far, as sent: `Name: value\r\n` lines.
    pub fn headers(&self) -> &str {
        &self.headers
    }

    // ------------------------------------------------------------------------
    /// `send`, logged as `METHOD host/path` -- no scheme, no query: queries
    /// can carry keys -- with how long it took to the response headers. A
    /// status other than 2xx is `Error::Http`: `reason` reads the API's
    /// reason and message from the body, else the body is the message. The
    /// caller decides what a status means (a conflict can be expected), so
    /// a refusal is logged as info.
    pub fn send_logged(self, reason: fn(&[u8]) -> Option<(String, String)>) -> Result<Response> {
        let url = self
            .url
            .split_once("://")
            .map_or(&*self.url, |(_, rest)| rest);
        let url = url.split_once('?').map_or(url, |(path, _)| path);
        let call = format!("{} {url}", self.method);
        let started = Instant::now();
        let resp = match self.send() {
            Ok(resp) => resp,
            Err(Error::Cancelled) => {
                let ms = started.elapsed().as_millis();
                log::debug!("{call}: cancelled after {ms} ms");
                return Err(Error::Cancelled);
            }
            Err(err) => {
                log::warn!("{call}: {err} after {} ms", started.elapsed().as_millis());
                return Err(err);
            }
        };
        let status = resp.status();
        log::debug!("{call}: {status} in {} ms", started.elapsed().as_millis());
        if resp.is_success() {
            return Ok(resp);
        }
        let body = resp.bytes()?;
        let (reason, msg) = reason(&body).unwrap_or_else(|| {
            let text = String::from_utf8_lossy(&body).trim().to_string();
            (String::new(), text)
        });
        let err = Error::Http {
            status,
            reason,
            msg,
        };
        log::info!("{call}: {err}");
        Err(err)
    }

    // ------------------------------------------------------------------------
    /// Send the request and wait for the response headers (blocking). Any
    /// status is a `Response`; failing to get one is `Error::Timeout`,
    /// `Error::Cancelled`, or `Error::Network`.
    pub fn send(self) -> Result<Response> {
        let cancel = self.cancel.clone().unwrap_or_default();
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let (status, body) = backend::send(&self, cancel)?;
        Ok(Response::new(status, body))
    }
}

// ----------------------------------------------------------------------------
/// One part of a `multipart/form-data` body: a field, or a file when it has
/// a `filename`.
pub struct FormPart<'a> {
    pub name: &'a str,
    pub filename: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub data: &'a [u8],
}

impl<'a> FormPart<'a> {
    // ------------------------------------------------------------------------
    pub fn text(name: &'a str, value: &'a str) -> Self {
        Self {
            name,
            filename: None,
            content_type: None,
            data: value.as_bytes(),
        }
    }

    // ------------------------------------------------------------------------
    pub fn file(name: &'a str, filename: &'a str, content_type: &'a str, data: &'a [u8]) -> Self {
        Self {
            name,
            filename: Some(filename),
            content_type: Some(content_type),
            data,
        }
    }
}

// ----------------------------------------------------------------------------
/// A name or filename inside a quoted `Content-Disposition` parameter, the
/// way browsers escape it (WHATWG form-data encoding).
fn escape_field(value: &str) -> String {
    value
        .replace('"', "%22")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

// ----------------------------------------------------------------------------
/// `key=value&…`, percent-encoded (query strings and form bodies).
fn encode_pairs(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                encode_url_component(key),
                encode_url_component(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}
