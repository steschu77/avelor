//! A response, its body read in one piece or as a stream.

use std::io::{self, Read};
use std::marker::PhantomData;

use av_utils::error::Result;
use serde::de::DeserializeOwned;

use crate::backend::Body;

// ----------------------------------------------------------------------------
/// The status is in; the body is read through `bytes`, `json`, or `Read`
/// (streaming, e.g. server-sent events). Decompressed if the server
/// compressed it.
pub struct Response {
    status: u16,
    body: Body,
    /// Not `Send` on either platform: libsoup does a request's I/O on the
    /// thread that sent it, so a response is read there.
    _thread: PhantomData<*const ()>,
}

impl Response {
    // ------------------------------------------------------------------------
    pub(crate) fn new(status: u16, body: Body) -> Self {
        Self {
            status,
            body,
            _thread: PhantomData,
        }
    }

    // ------------------------------------------------------------------------
    pub fn status(&self) -> u16 {
        self.status
    }

    // ------------------------------------------------------------------------
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    // ------------------------------------------------------------------------
    /// The value of the response header `name` (case-insensitive), the first
    /// if it repeats; `None` if it is absent.
    pub fn header(&self, name: &str) -> Option<String> {
        self.body.header(name)
    }

    // ------------------------------------------------------------------------
    /// The whole body.
    pub fn bytes(mut self) -> Result<Vec<u8>> {
        let mut body = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match self.body.read(&mut buf)? {
                0 => return Ok(body),
                n => body.extend_from_slice(&buf[..n]),
            }
        }
    }

    // ------------------------------------------------------------------------
    pub fn json<T: DeserializeOwned>(self) -> Result<T> {
        Ok(serde_json::from_slice(&self.bytes()?)?)
    }

    // ------------------------------------------------------------------------
    #[cfg(test)]
    pub(crate) fn http2(&self) -> bool {
        self.body.http2()
    }
}

impl Read for Response {
    // The `av_utils::error::Error` is the `io::Error`'s inner error.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.body.read(buf).map_err(io::Error::other)
    }
}
