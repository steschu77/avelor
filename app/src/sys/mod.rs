//! The platform layer: what differs between Windows and Linux
//! (`architecture.md#platforms`). Each backend provides the same items,
//! re-exported here, so the rest of the app never names a platform:
//!
//! - `paths() -> Result<Paths>`: the app's folders.
//! - `run(paths: &Paths, config: ConfigStore) -> Result<()>`: show the main
//!   window and run the main loop; returns once the window is gone.
//! - `UiSignal` (`Send + Sync`): `wake()` has the UI thread run the app's
//!   queue (`Shell::run_tasks`), from any thread; tests use `UiSignal::none()`.
//! - `BRIDGE_JS`: the page's bridge to the host, `window.host.post(text)`
//!   and `window.host.receive(text)` (`architecture.md#platforms`).
//! - `show_error(title, text)`: a failure before or after the main window,
//!   when there is no page.

#[cfg(target_os = "windows")]
mod win32;
#[cfg(target_os = "windows")]
pub use win32::run;
#[cfg(target_os = "windows")]
pub use win32::{BRIDGE_JS, UiSignal};
#[cfg(target_os = "windows")]
pub use win32::{paths, show_error};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::run;
#[cfg(target_os = "linux")]
pub use linux::{BRIDGE_JS, UiSignal};
#[cfg(target_os = "linux")]
pub use linux::{paths, show_error};

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
compile_error!("Windows and Linux only.");
