//! The Windows backend: Win32 through the `windows` crate.

mod dialogs;
mod paths;

pub use dialogs::show_error;
pub use paths::paths;
