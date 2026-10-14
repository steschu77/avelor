//! The platform layer: what differs between Windows and Linux
//! (`architecture.md#platforms`). Each backend provides the same items,
//! re-exported here, so the rest of the app never names a platform:
//!
//! - `paths() -> Result<Paths>`: the app's folders.
//! - `run(paths: &Paths, config: ConfigStore) -> Result<()>`: show the main
//!   window and run the main loop; returns once the window is gone.
//! - `show_error(title, text)`: a failure before or after the main window,
//!   when there is no page.

#[cfg(target_os = "windows")]
mod win32;
#[cfg(target_os = "windows")]
pub use win32::run;
#[cfg(target_os = "windows")]
pub use win32::{paths, show_error};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::run;
#[cfg(target_os = "linux")]
pub use linux::{paths, show_error};

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
compile_error!("Windows and Linux only.");
