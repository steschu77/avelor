//! The Windows backend: Win32 through the `windows` crate.

mod dialogs;
mod images;
mod paths;
mod webview;
mod window;

use crate::config::ConfigStore;
use crate::shell::{Shell, TITLE};
use av_utils::error::Result;
use windows::Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
use windows::core::PWSTR;

pub use dialogs::open_url;
pub use dialogs::show_error;
pub use dialogs::{Owner, open_path, pick_file, pick_folder};
pub use images::{ComThread as ImageThread, preview};
pub use paths::Paths;
pub use paths::paths;
pub use paths::{collapse_env, expand_env, mark_downloaded};
pub use webview::BRIDGE_JS;
pub use window::UiSignal;

// ----------------------------------------------------------------------------
/// Show the main window in the state `config` saved, with the page in its
/// web view, and run the message loop until the window is gone.
pub fn run(paths: &Paths, config: ConfigStore) -> Result<()> {
    let webview_dir = paths.webview_dir();
    let shell = Shell::new(config, paths.catalog_dir());
    let window = window::WindowProc::create(TITLE, Some("APP_ICON"), webview_dir, shell)?;
    window::run_message_loop();
    drop(window);
    Ok(())
}

// ----------------------------------------------------------------------------
/// The user's preferred UI languages, the one Windows shows first
/// (`de-DE`, `en-US`); none if Windows can't tell.
pub fn languages() -> Vec<String> {
    let (mut count, mut len) = (0u32, 0u32);
    let mut buffer = Vec::new();
    unsafe {
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut len).is_err() {
            return Vec::new();
        }
        buffer.resize(len as usize, 0u16);
        let names = Some(PWSTR(buffer.as_mut_ptr()));
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, names, &mut len).is_err() {
            return Vec::new();
        }
    }
    buffer
        .split(|&c| c == 0)
        .filter(|name| !name.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}
