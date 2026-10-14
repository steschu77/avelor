//! Shared error codes across the entire app. No user facing wording - the app
//! translates errors within its context into meaningful messages in the user's
//! language. Error text is only used for logging and API calls.

// ----------------------------------------------------------------------------
#[derive(Debug)]
pub enum Error {
    // Generic errors
    Logging,
    InvalidDate,
    InvalidTime,

    // Error forwards
    FileIo { err: std::io::Error },
}

// ----------------------------------------------------------------------------
impl Error {
    // ------------------------------------------------------------------------
    /// The error's code.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Logging => "logging",
            Error::InvalidDate => "invalid_date",
            Error::InvalidTime => "invalid_time",
            Error::FileIo { .. } => "file_io",
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
pub type Result<T> = std::result::Result<T, Error>;
