//! HTTP for Avelor over WinHTTP, Windows' own HTTP stack: TLS
//! (Schannel), connection pooling, the system proxy (WPAD/PAC), HTTP/2,
//! and gzip/deflate decompression come with it. On Linux over libsoup 3,
//! which comes with WebKitGTK as WinHTTP comes with Windows: TLS
//! (glib-networking), the system proxy (GIO's resolver), HTTP/2 and gzip
//! come with it too. `client.rs` and `response.rs` are the same on both;
//! `win32.rs` and `linux.rs` are the backends.
//!
//! Requests block; run them on a worker thread. Timeouts limit each phase
//! (connect, send, each read), not a request as a whole, so an answer that
//! keeps streaming doesn't time out. A `Cancel` aborts requests from
//! another thread. Any status is a `Response`, what it means is up to the
//! caller; failing to get one is `av_utils::error::Error::Timeout`,
//! `Cancelled`, or `Network`.

mod client;
mod response;

#[cfg(target_os = "windows")]
mod win32;
#[cfg(target_os = "windows")]
use win32 as backend;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;

pub use client::{Client, FormPart, Method, Request};
pub use response::Response;

#[cfg(test)]
mod tests;
