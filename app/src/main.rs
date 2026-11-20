//! Avelor — Markdown/YAML Document Manager.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod ai;
mod analysis;
mod api;
mod app;
mod attachments;
mod bank;
mod blueprints;
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
mod reader;
mod recurring_payments;
mod scans;
mod search;
mod shell;
mod structure;
mod sys;
#[cfg(test)]
mod testing;
mod texts;
mod trash;
mod vault_settings;
mod watches;

use av_utils::error::Result;
use av_utils::logger;

// ----------------------------------------------------------------------------
/// The run: the app's folders and the log first. A failed start is named in
/// a box, with the log's folder if there is a log.
pub fn main() {
    let paths = sys::paths();
    let log_dir = paths.as_ref().ok().map(|paths| paths.log_dir());
    let log_dir = log_dir.filter(|dir| logger::init_logger(dir).is_ok());
    if let Err(e) = paths.and_then(|paths| run(&paths)) {
        log::error!("Avelor failed: {e}");
        log::logger().flush();

        let text = match &log_dir {
            Some(dir) => texts::filled("start.failed", &[("folder", &dir.display().to_string())]),
            None => texts::text("start.failed_no_log").to_string(),
        };
        sys::show_error("Avelor", &text);
    }
}

// ----------------------------------------------------------------------------
fn run(paths: &sys::Paths) -> Result<()> {
    let mut config = config::load_config(&paths.config_dir())?;
    texts::init(config.language());
    // `avelor <folder>` opens that vault for this session.
    if let Some(vault) = std::env::args_os().nth(1) {
        config.set_session_vault(std::path::absolute(vault)?);
    }

    sys::run(paths, config)
}
