//! Avelor — Markdown/YAML Document Manager.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod close;
mod config;
mod page;
mod protocol;
mod shell;
mod sys;
#[cfg(test)]
mod testing;
mod texts;

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
    let config = config::load_config(&paths.config_dir())?;
    texts::init(config.language());

    sys::run(&paths, config)
}
