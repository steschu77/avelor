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
//! - `mark_downloaded(path)`: a saved attachment marked as downloaded from
//!   the internet (the mark of the web on Windows; nothing on Linux).
//! - `preview(path, max_w, max_h) -> Result<Vec<u8>>`: an image as a JPEG
//!   that fits the box, upright by its EXIF orientation (WIC on Windows,
//!   gdk-pixbuf on Linux); a worker that makes previews holds an
//!   `ImageThread` while it does (COM on Windows).
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
pub use win32::{ImageThread, preview};
#[cfg(target_os = "windows")]
pub use win32::{Owner, open_path, pick_folder};
#[cfg(target_os = "windows")]
pub use win32::{collapse_env, expand_env, mark_downloaded};
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
pub use linux::{ImageThread, preview};
#[cfg(target_os = "linux")]
pub use linux::{Owner, open_path, pick_folder};
#[cfg(target_os = "linux")]
pub use linux::{collapse_env, expand_env, mark_downloaded};
#[cfg(target_os = "linux")]
pub use linux::{paths, show_error};

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
compile_error!("Windows and Linux only.");

// ----------------------------------------------------------------------------
/// `width` x `height` scaled down to fit `max_w` x `max_h`, keeping its
/// aspect; never up, and at least one pixel either way.
fn fit(width: u32, height: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if width <= max_w && height <= max_h {
        return (width.max(1), height.max(1));
    }
    let scale = (max_w as f64 / width as f64).min(max_h as f64 / height as f64);
    let scaled = |n: u32| ((n as f64 * scale).round() as u32).max(1);
    (scaled(width), scaled(height))
}

// ----------------------------------------------------------------------------
/// The page's zoom levels, Chromium's: WebView2's own Ctrl+wheel steps
/// through them, so the host's zoom does too.
const ZOOM_LEVELS: [f64; 17] = [
    0.25,
    1.0 / 3.0,
    0.5,
    2.0 / 3.0,
    0.75,
    0.8,
    0.9,
    1.0,
    1.1,
    1.25,
    1.5,
    1.75,
    2.0,
    2.5,
    3.0,
    4.0,
    5.0,
];

// ----------------------------------------------------------------------------
/// A step of the page's zoom, from a zoom key or Ctrl+wheel.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Zoom {
    In,
    Out,
    Reset,
}

impl Zoom {
    // ------------------------------------------------------------------------
    /// The level after `level`: the next one up or down, the same at either
    /// end, or 100%. A level read back rounded counts as the one it's close to.
    fn level_after(self, level: f64) -> f64 {
        const CLOSE: f64 = 0.01;
        match self {
            Self::In => ZOOM_LEVELS.into_iter().find(|&l| l > level + CLOSE),
            Self::Out => ZOOM_LEVELS.into_iter().rev().find(|&l| l < level - CLOSE),
            Self::Reset => Some(1.0),
        }
        .unwrap_or(level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------------------
    #[test]
    fn sizes_fit_the_box_and_never_grow() {
        assert_eq!(fit(2480, 3508, 360, 508), (359, 508));
        assert_eq!(fit(4000, 1000, 360, 508), (360, 90));
        assert_eq!(fit(100, 50, 360, 508), (100, 50));
        assert_eq!(fit(10_000, 1, 360, 508), (360, 1), "at least a pixel");
    }

    // ------------------------------------------------------------------------
    #[test]
    fn zoom_steps_through_the_levels() {
        assert_eq!(Zoom::In.level_after(1.0), 1.1);
        assert_eq!(Zoom::Out.level_after(1.0), 0.9);
        assert_eq!(Zoom::In.level_after(1.05), 1.1, "between two levels");
        assert_eq!(Zoom::Out.level_after(1.05), 1.0, "between two levels");
        assert_eq!(Zoom::In.level_after(1.1 - 1e-9), 1.25, "read back rounded");
        assert_eq!(Zoom::Out.level_after(1.1 + 1e-9), 1.0, "read back rounded");
        assert_eq!(Zoom::In.level_after(5.0), 5.0, "the top");
        assert_eq!(Zoom::Out.level_after(0.25), 0.25, "the bottom");
        assert_eq!(Zoom::Reset.level_after(2.5), 1.0);
    }
}
