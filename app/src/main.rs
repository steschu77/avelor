//! Avelor — Markdown/YAML Document Manager.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod close;
mod config;
mod jobs;
mod page;
mod protocol;
mod shell;
mod sys;
#[cfg(test)]
mod testing;
mod vault_scan;

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
    let mut config = config::load_config(&paths.config_dir())?;
    // `avelor <folder>` opens that vault for this session.
    if let Some(vault) = std::env::args_os().nth(1) {
        config.set_session_vault(std::path::absolute(vault)?);
    }

    sys::run(&paths, config)
}
