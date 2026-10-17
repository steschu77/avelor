//! The libsoup 3 backend (`lib.rs`): a request sent and its body read, the
//! cancellables a `Cancel` cancels, and GIO's errors as
//! `av_utils::error::Error`. A `soup::Session` stays on the thread that
//! made it, so each worker thread has its own, one per timeout: the
//! requests of a task share their connections.

use std::cell::RefCell;
use std::time::Duration;

use av_utils::cancel::{Cancel, Hook};
use av_utils::error::{Error, Result};
use gio::prelude::*;
use glib::translate::IntoGlib;
use soup::prelude::*;

use crate::client::Request;

// ----------------------------------------------------------------------------
/// What a client's sessions send as `User-Agent`; the sessions themselves
/// are the threads' (`session`).
pub(crate) struct Session {
    user_agent: String,
}

impl Session {
    // ------------------------------------------------------------------------
    /// A session sending `user_agent`.
    pub(crate) fn new(user_agent: &str) -> Result<Self> {
        Ok(Self {
            user_agent: user_agent.to_string(),
        })
    }
}

thread_local! {
    /// This thread's sessions: user agent, timeout in s, session.
    static SESSIONS: RefCell<Vec<(String, u32, soup::Session)>> = const { RefCell::new(Vec::new()) };
}

// ----------------------------------------------------------------------------
/// This thread's session for `user_agent` with `timeout` (s; 0 means
/// none), made the first time it is asked for. libsoup has one timeout for
/// every phase.
fn session(user_agent: &str, timeout: u32) -> soup::Session {
    SESSIONS.with_borrow_mut(|sessions| {
        let found = sessions
            .iter()
            .find(|(agent, secs, _)| agent == user_agent && *secs == timeout);
        if let Some((_, _, session)) = found {
            return session.clone();
        }
        let session = soup::Session::new();
        session.set_user_agent(user_agent);
        session.set_timeout(timeout);
        sessions.push((user_agent.to_string(), timeout, session.clone()));
        session
    })
}

// ----------------------------------------------------------------------------
/// Send `request` and wait for the response headers (blocking): its status
/// and body.
pub(crate) fn send(request: &Request, cancel: Cancel) -> Result<(u16, Body)> {
    let message = soup::Message::new(request.method.as_str(), &request.url).map_err(|err| {
        Error::Network {
            msg: err.to_string(),
        }
    })?;
    if let Some(headers) = message.request_headers() {
        for line in request.headers.split("\r\n") {
            if let Some((name, value)) = line.split_once(": ") {
                headers.append(name, value);
            }
        }
    }
    if !request.body.is_empty() {
        let body = glib::Bytes::from(request.body.as_slice());
        message.set_request_body_from_bytes(None, Some(&body));
    }
    let open = Open::new(cancel)?;
    let timeout = secs(request.timeout.unwrap_or(request.client.timeout));
    let session = session(&request.client.session.user_agent, timeout);
    let stream = session
        .send(&message, Some(&open.cancellable))
        .map_err(|err| open.error(err))?;
    let status = message.status().into_glib() as u16;
    let body = Body {
        message,
        stream,
        open,
    };
    Ok((status, body))
}

// ----------------------------------------------------------------------------
/// A response's body as libsoup reads it.
pub(crate) struct Body {
    message: soup::Message,
    stream: gio::InputStream,
    open: Open,
}

impl Body {
    // ------------------------------------------------------------------------
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        let value = self.message.response_headers()?.one(name)?;
        Some(value.to_string())
    }

    // ------------------------------------------------------------------------
    /// Up to `buf.len()` bytes of the body, what has arrived, blocking until
    /// some has; 0 at its end.
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.stream
            .read(buf, Some(&self.open.cancellable))
            .map_err(|err| self.open.error(err))
    }

    // ------------------------------------------------------------------------
    #[cfg(test)]
    pub(crate) fn http2(&self) -> bool {
        self.message.http_version() == soup::HTTPVersion::Http20
    }
}

// ----------------------------------------------------------------------------
/// A request's cancellable as its `Cancel` holds it (`Cancel::on_cancel`).
struct Abort(gio::Cancellable);

impl Abort {
    // ------------------------------------------------------------------------
    fn abort(self) {
        // A call blocked on the request returns G_IO_ERROR_CANCELLED.
        self.0.cancel();
    }
}

// ----------------------------------------------------------------------------
/// A request's cancellable while the request is open, registered with its
/// `Cancel` until it is dropped.
struct Open {
    cancellable: gio::Cancellable,
    cancel: Cancel,
    _hook: Hook,
}

impl Open {
    // ------------------------------------------------------------------------
    /// Register a new cancellable; `Error::Cancelled` if `cancel` was
    /// cancelled already.
    fn new(cancel: Cancel) -> Result<Self> {
        let cancellable = gio::Cancellable::new();
        let abort = Abort(cancellable.clone());
        let hook = cancel.on_cancel(move || abort.abort())?;
        Ok(Self {
            cancellable,
            cancel,
            _hook: hook,
        })
    }

    // ------------------------------------------------------------------------
    /// The error of a call on the request: `Error::Cancelled` once its
    /// `Cancel` was cancelled, whatever GIO answered.
    fn error(&self, err: glib::Error) -> Error {
        if self.cancel.is_cancelled() {
            Error::Cancelled
        } else {
            from_glib(err)
        }
    }
}

// ----------------------------------------------------------------------------
fn from_glib(err: glib::Error) -> Error {
    match err.kind::<gio::IOErrorEnum>() {
        Some(gio::IOErrorEnum::Cancelled) => Error::Cancelled,
        Some(gio::IOErrorEnum::TimedOut) => Error::Timeout,
        _ => Error::Network {
            msg: err.message().to_string(),
        },
    }
}

// ----------------------------------------------------------------------------
/// A libsoup timeout: whole seconds, rounded up, where 0 means none.
fn secs(timeout: Duration) -> u32 {
    u32::try_from(timeout.as_millis().div_ceil(1000)).unwrap_or(u32::MAX)
}
