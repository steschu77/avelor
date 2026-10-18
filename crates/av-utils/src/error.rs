//! Shared error codes across the entire app. No user facing wording - the app
//! translates errors within its context into meaningful messages in the user's
//! language. Error text is only used for logging and API calls.

use std::path::PathBuf;

// ----------------------------------------------------------------------------
#[derive(Debug)]
pub enum Error {
    // Generic errors
    Logging,
    NotInitialized,
    /// A deadline passed: a network request, or waiting for the user to
    /// finish a browser sign-in.
    Timeout,
    Cancelled,
    /// A stored credential is no longer accepted (e.g. an OAuth refresh
    /// token was revoked); the user has to reconnect the account.
    Expired,
    /// A sign-in answer didn't belong to the request we sent (OAuth
    /// `state` mismatch).
    Aborted,
    /// The user declined a sign-in.
    Denied,
    BadRequest {
        msg: String,
    },
    InvalidArgument {
        arg: String,
    },
    InvalidDate,
    InvalidTime,
    InvalidPath {
        path: PathBuf,
    },
    InvalidFileType,
    /// No entity, account or message has the id: it never did, or it is
    /// gone.
    NotFound,
    /// The id names something of another kind than the command takes (a
    /// task where a project is expected, a reply where the user's own
    /// message is).
    WrongKind,
    /// A 2xx answer whose content can't be used.
    InvalidResponse {
        msg: String,
    },
    /// The service doesn't offer this.
    Unsupported {
        what: String,
    },
    /// The account has no usable credential: disconnected, or its secret is
    /// missing or can't be decrypted here.
    NotConnected,
    /// An AI account has spent its monthly limit; it takes no more calls
    /// this month.
    SpendLimit,
    /// A sign-in for an account came back as another one: `email` is who
    /// signed in.
    WrongAccount {
        email: String,
    },
    /// The entity was created in the app and has no file yet.
    Unsaved,
    /// The entity's file can't be written: read-only, or its frontmatter
    /// didn't parse (rewriting it would lose the broken original).
    NotWritable {
        path: PathBuf,
    },
    /// The entity's file changed outside the app since it was last read or
    /// written, so it wasn't overwritten. Only for callers that can't let
    /// the user choose; av-vault reports it as `SaveOutcome::Conflict`.
    ChangedOutside {
        path: PathBuf,
    },
    FileNotFound {
        path: PathBuf,
    },
    /// Something is already where a file would go -- a restore from the
    /// trash: a file in its place, or an entity with its `id`; `path` is
    /// the one in the way.
    FileExists {
        path: PathBuf,
    },
    FileWrite {
        path: PathBuf,
        err: std::io::Error,
    },

    // Error forwards
    FileIo {
        err: std::io::Error,
    },
    FromUtf8 {
        err: std::string::FromUtf8Error,
    },
    FromUtf16 {
        err: std::string::FromUtf16Error,
    },
    /// The request never got an HTTP answer (connection, TLS, transport).
    Network {
        msg: String,
    },
    /// The server answered with a non-success status.
    Http {
        status: u16,
        /// Machine-readable error code from the response body (e.g. Google's
        /// `ErrorInfo.reason`, or OAuth's `error`); empty if there is none.
        reason: String,
        msg: String,
    },
    Serde {
        line: usize,
        column: usize,
        msg: String,
    },
    Win32 {
        code: i32, // the Win32 error code
    },
    Gtk {
        msg: String,
    },
}

// ----------------------------------------------------------------------------
impl Error {
    // ------------------------------------------------------------------------
    /// The error's code (`api.md#rules`).
    pub fn code(&self) -> &'static str {
        match self {
            Error::Logging => "logging",
            Error::NotInitialized => "not_initialized",
            Error::Timeout => "timeout",
            Error::Cancelled => "cancelled",
            Error::Expired => "expired",
            Error::Aborted => "aborted",
            Error::Denied => "denied",
            Error::BadRequest { .. } => "bad_request",
            Error::InvalidArgument { .. } => "invalid_argument",
            Error::InvalidDate => "invalid_date",
            Error::InvalidTime => "invalid_time",
            Error::InvalidPath { .. } => "invalid_path",
            Error::InvalidFileType => "invalid_file_type",
            Error::NotFound => "not_found",
            Error::WrongKind => "wrong_kind",
            Error::InvalidResponse { .. } => "invalid_response",
            Error::Unsupported { .. } => "unsupported",
            Error::NotConnected => "not_connected",
            Error::SpendLimit => "spend_limit",
            Error::WrongAccount { .. } => "wrong_account",
            Error::Unsaved => "unsaved",
            Error::NotWritable { .. } => "not_writable",
            Error::ChangedOutside { .. } => "changed_outside",
            Error::FileNotFound { .. } => "file_not_found",
            Error::FileExists { .. } => "file_exists",
            Error::FileWrite { .. } => "file_write",
            Error::FileIo { .. } => "file_io",
            Error::FromUtf8 { .. } => "from_utf8",
            Error::FromUtf16 { .. } => "from_utf16",
            Error::Network { .. } => "network",
            Error::Http { .. } => "http",
            Error::Serde { .. } => "serde",
            Error::Win32 { .. } => "win32",
            Error::Gtk { .. } => "gtk",
        }
    }
}

// ----------------------------------------------------------------------------
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let err = format!("{:?}", self);
        f.write_str(&err)
    }
}

// ----------------------------------------------------------------------------
impl std::error::Error for Error {}

// ----------------------------------------------------------------------------
impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::FileIo { err }
    }
}

// ----------------------------------------------------------------------------
impl From<std::string::FromUtf8Error> for Error {
    fn from(err: std::string::FromUtf8Error) -> Self {
        Error::FromUtf8 { err }
    }
}

// ----------------------------------------------------------------------------
impl From<std::string::FromUtf16Error> for Error {
    fn from(err: std::string::FromUtf16Error) -> Self {
        Error::FromUtf16 { err }
    }
}

// ----------------------------------------------------------------------------
impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Serde {
            line: err.line(),
            column: err.column(),
            msg: err.to_string(),
        }
    }
}

// ----------------------------------------------------------------------------
impl From<serde_yaml_ng::Error> for Error {
    fn from(err: serde_yaml_ng::Error) -> Self {
        let (line, column) = err.location().map_or((0, 0), |l| (l.line(), l.column()));
        Error::Serde {
            line,
            column,
            msg: err.to_string(),
        }
    }
}

// ----------------------------------------------------------------------------
#[cfg(target_os = "windows")]
impl From<windows::core::Error> for Error {
    fn from(err: windows::core::Error) -> Self {
        Error::Win32 { code: err.code().0 }
    }
}

// ----------------------------------------------------------------------------
#[cfg(target_os = "linux")]
impl From<glib::error::BoolError> for Error {
    fn from(err: glib::error::BoolError) -> Self {
        Error::Gtk {
            msg: err.to_string(),
        }
    }
}

// ----------------------------------------------------------------------------
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn codes_are_the_variant_names_in_snake_case() {
        assert_eq!(Error::Cancelled.code(), "cancelled");
        assert_eq!(Error::NotFound.code(), "not_found");
        let arg = "url".to_string();
        assert_eq!(Error::InvalidArgument { arg }.code(), "invalid_argument");
        let path = PathBuf::from("a.md");
        assert_eq!(Error::ChangedOutside { path }.code(), "changed_outside");
        let msg = String::new();
        assert_eq!(Error::BadRequest { msg }.code(), "bad_request");
    }
}
