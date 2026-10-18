//! The platform layer: what differs between Windows and Linux
//! (`architecture.md#platforms`). Each backend provides the same items,
//! re-exported here, so the rest of the app never names a platform:
//!
//! - `paths() -> Result<Paths>`: the app's folders.
//! - `run(paths: &Paths, config: ConfigStore) -> Result<()>`: show the main
//!   window and run the main loop; returns once the window is gone.
//! - `UiSignal` (`Send + Sync`): `wake()` has the UI thread run the app's
//!   queue (`Shell::run_tasks`), from any thread; tests use `UiSignal::none()`.
//! - `Owner`: the main window as the owner of a `UiTask::Modal`'s dialog;
//!   tests use `Owner::none()`.
//! - `pick_folder(owner: &Owner, title) -> Option<PathBuf>`: a folder the
//!   user picks; `open_path(path) -> Result<()>`: a file or folder opened
//!   in its default app; `open_url(url) -> Result<()>`: a link opened in
//!   its default app. All three wait in the platform's modal loop.
//! - `expand_env(text) -> PathBuf`, `collapse_env(path) -> String`: a
//!   watch's folder as used and as stored: `%NAME%` and `%USERPROFILE%\…`
//!   on Windows, a leading `~` and `$NAME` and `~/…` on Linux.
//! - `BRIDGE_JS`: the page's bridge to the host, `window.host.post(text)`
//!   and `window.host.receive(text)` (`architecture.md#platforms`).
//! - `show_error(title, text)`: a failure before or after the main window,
//!   when there is no page.

#[cfg(target_os = "windows")]
mod win32;
#[cfg(target_os = "windows")]
pub use win32::open_url;
#[cfg(target_os = "windows")]
pub use win32::run;
#[cfg(target_os = "windows")]
pub use win32::{BRIDGE_JS, UiSignal};
#[cfg(target_os = "windows")]
pub use win32::{Owner, open_path, pick_folder};
#[cfg(target_os = "windows")]
pub use win32::{collapse_env, expand_env};
#[cfg(target_os = "windows")]
pub use win32::{paths, show_error};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::open_url;
#[cfg(target_os = "linux")]
pub use linux::run;
#[cfg(target_os = "linux")]
pub use linux::{BRIDGE_JS, UiSignal};
#[cfg(target_os = "linux")]
pub use linux::{Owner, open_path, pick_folder};
#[cfg(target_os = "linux")]
pub use linux::{collapse_env, expand_env};
#[cfg(target_os = "linux")]
pub use linux::{paths, show_error};

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
compile_error!("Windows and Linux only.");
