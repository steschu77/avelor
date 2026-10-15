//! The Linux backend: GTK4 through the `gtk4` crate.

mod dialogs;
mod paths;

pub use dialogs::show_error;
pub use paths::paths;
