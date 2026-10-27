//! Avelor — Markdown/YAML Document Manager.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod ai;
mod analysis;
mod api;
mod app;
mod calendar;
mod catalog;
mod chat;
mod close;
mod config;
mod documents;
mod email;
mod google;
mod indexing;
mod jobs;
mod page;
mod protocol;
mod scans;
mod settings;
mod shell;
mod structure;
mod sys;
#[cfg(test)]
mod testing;
mod texts;
mod trash;
mod undo;
mod vault_settings;
mod watches;

use av_utils::error::Result;
use av_utils::logger;

// ----------------------------------------------------------------------------
pub fn main() {
    if let Err(e) = run() {
        log::error!("Avelor failed: {e}");
        log::logger().flush();

        sys::show_error("Avelor", texts::text("start.failed"));
    }
}

// ----------------------------------------------------------------------------
fn run() -> Result<()> {
    let paths = sys::paths()?;
    let _ = logger::init_logger(&paths.log_dir());
    let mut config = config::load_config(&paths.config_dir())?;
    texts::init(config.language());
    // `avelor <folder>` opens that vault for this session.
    if let Some(vault) = std::env::args_os().nth(1) {
        config.set_session_vault(std::path::absolute(vault)?);
    }

    sys::run(&paths, config)
}
