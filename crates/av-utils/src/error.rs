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
    Cancelled,
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
    /// No entity has the id: it never did, or it is gone.
    NotFound,
    /// The id names something of another kind than the command takes (a
    /// task where a project is expected).
    WrongKind,
    /// The entity's file can't be written: read-only, or its frontmatter
    /// didn't parse (rewriting it would lose the broken original).
    NotWritable {
        path: PathBuf,
    },
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
            Error::Cancelled => "cancelled",
            Error::BadRequest { .. } => "bad_request",
            Error::InvalidArgument { .. } => "invalid_argument",
            Error::InvalidDate => "invalid_date",
            Error::InvalidTime => "invalid_time",
            Error::InvalidPath { .. } => "invalid_path",
            Error::NotFound => "not_found",
            Error::WrongKind => "wrong_kind",
            Error::NotWritable { .. } => "not_writable",
            Error::FileExists { .. } => "file_exists",
            Error::FileWrite { .. } => "file_write",
            Error::FileIo { .. } => "file_io",
            Error::FromUtf8 { .. } => "from_utf8",
            Error::FromUtf16 { .. } => "from_utf16",
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
