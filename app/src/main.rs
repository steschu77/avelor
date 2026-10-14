//! Avelor — Markdown/YAML Document Manager.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod shell;
mod sys;

use av_utils::error::Result;
use av_utils::logger;

// ----------------------------------------------------------------------------
pub fn main() {
    if let Err(e) = run() {
        log::error!("Avelor failed: {e}");
        log::logger().flush();

        sys::show_error(
            "Avelor",
            "An error occurred.\n\nCheck the log file for more information.",
        );
    }
}

// ----------------------------------------------------------------------------
fn run() -> Result<()> {
    let paths = sys::paths()?;
    let _ = logger::init_logger(&paths.log_dir());
    let config = config::load_config(&paths.config_dir())?;

    sys::run(&paths, config)
}
