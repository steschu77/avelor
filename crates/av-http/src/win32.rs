//! The WinHTTP backend (`lib.rs`): the session, a request sent and its
//! body read, the handles a `Cancel` closes -- the raw WinHTTP layer: owned
//! handles, URL cracking, and WinHTTP's error codes as
//! `av_utils::error::Error`.

use std::ffi::c_void;
use std::sync::Arc;
use std::time::Duration;

use av_utils::cancel::{Cancel, Hook};
use av_utils::error::{Error, Result};
use windows::Win32::Foundation::GetLastError;
use windows::Win32::Networking::WinHttp::{
    ERROR_WINHTTP_OPERATION_CANCELLED, ERROR_WINHTTP_TIMEOUT, URL_COMPONENTS,
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DECOMPRESSION_FLAG_DEFLATE,
    WINHTTP_DECOMPRESSION_FLAG_GZIP, WINHTTP_FLAG_SECURE, WINHTTP_INTERNET_SCHEME_HTTPS,
    WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_OPTION_DECOMPRESSION, WINHTTP_OPTION_ENABLE_HTTP_PROTOCOL,
    WINHTTP_OPTION_IPV6_FAST_FALLBACK, WINHTTP_OPTION_RECEIVE_RESPONSE_TIMEOUT,
    WINHTTP_PROTOCOL_FLAG_HTTP2, WINHTTP_QUERY_CUSTOM, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData,
    WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
};
use windows::Win32::System::Diagnostics::Debug::{
    FORMAT_MESSAGE_FROM_HMODULE, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
    FormatMessageW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::core::{HSTRING, PCWSTR, PWSTR, w};

use crate::client::Request;

// ----------------------------------------------------------------------------
/// A WinHTTP session: the connection pool, with the system's proxy
/// settings. Shared with the responses: closing it would cancel them.
pub(crate) struct Session(Arc<Handle>);

impl Session {
    // ------------------------------------------------------------------------
    /// A session sending `user_agent`.
    pub(crate) fn new(user_agent: &str) -> Result<Self> {
        let session = Handle::new(unsafe {
            WinHttpOpen(
                &HSTRING::from(user_agent),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            )
        })?;
        // Requests work without these, just larger, one at a time, or slow to
        // connect.
        let decompression = WINHTTP_DECOMPRESSION_FLAG_GZIP | WINHTTP_DECOMPRESSION_FLAG_DEFLATE;
        if let Err(e) = set_option(session.raw(), WINHTTP_OPTION_DECOMPRESSION, decompression) {
            log::debug!("WinHTTP decompression unavailable: {e}");
        }
        let http2 = WINHTTP_PROTOCOL_FLAG_HTTP2;
        if let Err(e) = set_option(session.raw(), WINHTTP_OPTION_ENABLE_HTTP_PROTOCOL, http2) {
            log::debug!("WinHTTP HTTP/2 unavailable: {e}");
        }
        // Try IPv4 too when IPv6 doesn't connect at once, as browsers do: on
        // a network whose IPv6 goes nowhere, every new connection would wait
        // out the 21-second TCP timeout first.
        if let Err(e) = set_option(session.raw(), WINHTTP_OPTION_IPV6_FAST_FALLBACK, 1) {
            log::debug!("WinHTTP IPv6 fast fallback unavailable: {e}");
        }
        Ok(Self(Arc::new(session)))
    }
}

// ----------------------------------------------------------------------------
/// Send `request` and wait for the response headers (blocking): its status
/// and body.
pub(crate) fn send(request: &Request, cancel: Cancel) -> Result<(u16, Body)> {
    let target = crack_url(&request.url)?;
    let connect = Handle::new(unsafe {
        WinHttpConnect(
            request.client.session.0.raw(),
            PCWSTR(target.host.as_ptr()),
            target.port,
            0,
        )
    })?;
    let flags = if target.secure {
        WINHTTP_FLAG_SECURE
    } else {
        WINHTTP_OPEN_REQUEST_FLAGS(0)
    };
    let raw = unsafe {
        WinHttpOpenRequest(
            connect.raw(),
            &HSTRING::from(request.method.as_str()),
            PCWSTR(target.object.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        )
    };
    let handle = RequestHandle::new(raw, cancel)?;

    let phase = millis(request.client.timeout);
    let io = request.timeout.map_or(phase, millis);
    handle.call(|h| unsafe { WinHttpSetTimeouts(h, phase, phase, io, io) })?;
    handle.call(|h| set_option(h, WINHTTP_OPTION_RECEIVE_RESPONSE_TIMEOUT, io as u32))?;

    let headers: Vec<u16> = request.headers.encode_utf16().collect();
    let len = u32::try_from(request.body.len()).map_err(|_| Error::InvalidArgument {
        arg: format!("request body of {} bytes", request.body.len()),
    })?;
    handle.call(|h| unsafe {
        WinHttpSendRequest(
            h,
            (!headers.is_empty()).then_some(headers.as_slice()),
            (!request.body.is_empty()).then_some(request.body.as_ptr().cast()),
            len,
            len,
            0,
        )
    })?;
    handle.call(|h| unsafe { WinHttpReceiveResponse(h, std::ptr::null_mut()) })?;

    let mut status = 0u32;
    let mut size = size_of::<u32>() as u32;
    handle.call(|h| unsafe {
        WinHttpQueryHeaders(
            h,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
    })?;
    let body = Body {
        request: handle,
        _connect: connect,
        _session: Arc::clone(&request.client.session.0),
    };
    Ok((status as u16, body))
}

// ----------------------------------------------------------------------------
/// A response's body as WinHTTP reads it.
pub(crate) struct Body {
    // Field order is drop order: the request closes before its connection,
    // the connection before the session.
    request: RequestHandle,
    _connect: Handle,
    _session: Arc<Handle>,
}

impl Body {
    // ------------------------------------------------------------------------
    pub(crate) fn header(&self, name: &str) -> Option<String> {
        let name = HSTRING::from(name);
        // Asked with no buffer, WinHTTP reports the size needed, in bytes
        // with the terminating null; an absent header leaves it at 0.
        let mut size = 0u32;
        let _ = self.request.call(|h| unsafe {
            WinHttpQueryHeaders(
                h,
                WINHTTP_QUERY_CUSTOM,
                &name,
                None,
                &mut size,
                std::ptr::null_mut(),
            )
        });
        if size == 0 {
            return None;
        }
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        self.request
            .call(|h| unsafe {
                WinHttpQueryHeaders(
                    h,
                    WINHTTP_QUERY_CUSTOM,
                    &name,
                    Some(buf.as_mut_ptr().cast()),
                    &mut size,
                    std::ptr::null_mut(),
                )
            })
            .ok()?;
        // Now the length without the null.
        buf.truncate(size as usize / 2);
        String::from_utf16(&buf).ok()
    }

    // ------------------------------------------------------------------------
    /// Up to `buf.len()` bytes of the body, what has arrived, blocking until
    /// some has; 0 at its end. Asked for more than has arrived, a
    /// synchronous `WinHttpReadData` waits until its buffer is full, which
    /// would hold a stream back.
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let mut available = 0u32;
        self.request
            .call(|h| unsafe { WinHttpQueryDataAvailable(h, &mut available) })?;
        if available == 0 {
            return Ok(0);
        }
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX).min(available);
        let mut read = 0u32;
        self.request
            .call(|h| unsafe { WinHttpReadData(h, buf.as_mut_ptr().cast(), len, &mut read) })?;
        Ok(read as usize)
    }

    // ------------------------------------------------------------------------
    #[cfg(test)]
    pub(crate) fn http2(&self) -> bool {
        use windows::Win32::Networking::WinHttp::{
            WINHTTP_OPTION_HTTP_PROTOCOL_USED, WINHTTP_PROTOCOL_FLAG_HTTP2, WinHttpQueryOption,
        };
        let mut used = 0u32;
        let mut size = size_of::<u32>() as u32;
        self.request
            .call(|h| unsafe {
                WinHttpQueryOption(
                    h,
                    WINHTTP_OPTION_HTTP_PROTOCOL_USED,
                    Some((&mut used as *mut u32).cast()),
                    &mut size,
                )
            })
            .is_ok_and(|()| used & WINHTTP_PROTOCOL_FLAG_HTTP2 != 0)
    }
}

// ----------------------------------------------------------------------------
/// A request handle as its `Cancel` holds it (`Cancel::on_cancel`).
struct Abort(*mut c_void);

// WinHTTP handles may be closed from any thread.
unsafe impl Send for Abort {}

impl Abort {
    // ------------------------------------------------------------------------
    fn abort(self) {
        // A call blocked on the handle returns OPERATION_CANCELLED.
        close(self.0);
    }
}

// ----------------------------------------------------------------------------
/// A request handle its `Cancel` may close from another thread. Every
/// WinHTTP call on it goes through `call`, which doesn't start one once the
/// handle is cancelled.
pub(crate) struct RequestHandle {
    raw: *mut c_void,
    cancel: Cancel,
    hook: Hook,
}

// WinHTTP handles may be used and closed from any thread.
unsafe impl Send for RequestHandle {}

impl RequestHandle {
    // ------------------------------------------------------------------------
    /// Take ownership of what `WinHttpOpenRequest` returned.
    pub(crate) fn new(raw: *mut c_void, cancel: Cancel) -> Result<Self> {
        if raw.is_null() {
            return Err(last_error());
        }
        let abort = Abort(raw);
        let hook = cancel
            .on_cancel(move || abort.abort())
            .inspect_err(|_| close(raw))?;
        Ok(Self { raw, cancel, hook })
    }

    // ------------------------------------------------------------------------
    /// Run the WinHTTP call `f` on the handle. A cancel that lands between
    /// the check and the call leaves `f` a closed handle, which fails too;
    /// either way the result is `Error::Cancelled`.
    pub(crate) fn call(
        &self,
        f: impl FnOnce(*mut c_void) -> windows::core::Result<()>,
    ) -> Result<()> {
        if self.cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        f(self.raw).map_err(|err| {
            if self.cancel.is_cancelled() {
                Error::Cancelled
            } else {
                from_win(err)
            }
        })
    }
}

impl Drop for RequestHandle {
    fn drop(&mut self) {
        // Closed already if `cancel` ran the abort.
        if self.hook.release() {
            close(self.raw);
        }
    }
}

// ----------------------------------------------------------------------------
/// An owned WinHTTP session or connection handle, closed on drop.
pub(crate) struct Handle(*mut c_void);

// WinHTTP handles may be used and closed from any thread.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    // ------------------------------------------------------------------------
    /// Take ownership of what a WinHTTP open call returned (null: it failed).
    pub(crate) fn new(raw: *mut c_void) -> Result<Self> {
        if raw.is_null() {
            return Err(last_error());
        }
        Ok(Self(raw))
    }

    // ------------------------------------------------------------------------
    pub(crate) fn raw(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        close(self.0);
    }
}

// ----------------------------------------------------------------------------
pub(crate) fn close(raw: *mut c_void) {
    unsafe {
        let _ = WinHttpCloseHandle(raw);
    }
}

// ----------------------------------------------------------------------------
/// Set a `u32` option on a session or request handle.
pub(crate) fn set_option(
    handle: *mut c_void,
    option: u32,
    value: u32,
) -> windows::core::Result<()> {
    unsafe { WinHttpSetOption(Some(handle), option, Some(&value.to_ne_bytes())) }
}

// ----------------------------------------------------------------------------
/// Where a request goes, split the way WinHTTP's open calls want it
/// (null-terminated UTF-16).
pub(crate) struct Target {
    pub host: Vec<u16>,
    pub port: u16,
    /// Path and query, still percent-encoded.
    pub object: Vec<u16>,
    pub secure: bool,
}

// ----------------------------------------------------------------------------
pub(crate) fn crack_url(url: &str) -> Result<Target> {
    let wide: Vec<u16> = url.encode_utf16().collect();
    // Null pointers with non-zero lengths: WinHTTP points them into `wide`.
    let mut parts = URL_COMPONENTS {
        dwStructSize: size_of::<URL_COMPONENTS>() as u32,
        dwSchemeLength: u32::MAX,
        dwHostNameLength: u32::MAX,
        dwUrlPathLength: u32::MAX,
        dwExtraInfoLength: u32::MAX,
        ..Default::default()
    };
    unsafe { WinHttpCrackUrl(&wide, 0, &mut parts) }.map_err(from_win)?;

    let piece = |ptr: PWSTR, len: u32| -> &[u16] {
        if ptr.is_null() {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(ptr.0, len as usize) }
    };
    let mut host = piece(parts.lpszHostName, parts.dwHostNameLength).to_vec();
    host.push(0);
    let mut object = piece(parts.lpszUrlPath, parts.dwUrlPathLength).to_vec();
    if object.is_empty() {
        object.push(u16::from(b'/'));
    }
    object.extend_from_slice(piece(parts.lpszExtraInfo, parts.dwExtraInfoLength));
    object.push(0);
    Ok(Target {
        host,
        port: parts.nPort,
        object,
        secure: parts.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS,
    })
}

// ----------------------------------------------------------------------------
/// The error of a WinHTTP call that returned null.
pub(crate) fn last_error() -> Error {
    from_code(unsafe { GetLastError() }.0)
}

// ----------------------------------------------------------------------------
/// The error of a WinHTTP call that returned `FALSE`.
pub(crate) fn from_win(err: windows::core::Error) -> Error {
    // It carries GetLastError as HRESULT_FROM_WIN32.
    let hr = err.code().0 as u32;
    from_code(if hr >> 16 == 0x8007 { hr & 0xFFFF } else { hr })
}

// ----------------------------------------------------------------------------
fn from_code(code: u32) -> Error {
    match code {
        ERROR_WINHTTP_TIMEOUT => Error::Timeout,
        ERROR_WINHTTP_OPERATION_CANCELLED => Error::Cancelled,
        _ => Error::Network {
            msg: format!("{} ({code})", message(code)),
        },
    }
}

// ----------------------------------------------------------------------------
/// The text for `code`: WinHTTP's messages live in winhttp.dll, others are
/// the system's.
fn message(code: u32) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe {
        let module = GetModuleHandleW(w!("winhttp.dll")).ok();
        FormatMessageW(
            FORMAT_MESSAGE_FROM_HMODULE
                | FORMAT_MESSAGE_FROM_SYSTEM
                | FORMAT_MESSAGE_IGNORE_INSERTS,
            module.map(|m| m.0 as *const c_void),
            code,
            0,
            PWSTR(buf.as_mut_ptr()),
            buf.len() as u32,
            None,
        )
    };
    let text = String::from_utf16_lossy(&buf[..len as usize]);
    match text.trim() {
        "" => "Network error".to_string(),
        text => text.to_string(),
    }
}

// ----------------------------------------------------------------------------
/// A WinHTTP timeout: ms, where 0 means none.
fn millis(timeout: Duration) -> i32 {
    i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX)
}
