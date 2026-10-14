//! What the main window decides, the same on every platform
//! (`architecture.md#platforms`). Each platform's window only turns its
//! native events into calls on it.

use std::cell::RefCell;

use crate::config::{ConfigStore, WindowState};

// ----------------------------------------------------------------------------
/// The main window's title.
pub const TITLE: &str = "Avelor";

/// The main window's size until one is saved.
const DEFAULT_STATE: WindowState = WindowState {
    width: 1280,
    height: 720,
    maximized: false,
};

// ----------------------------------------------------------------------------
pub struct Shell {
    config: RefCell<ConfigStore>,
}

impl Shell {
    // ------------------------------------------------------------------------
    pub fn new(config: ConfigStore) -> Self {
        Self {
            config: RefCell::new(config),
        }
    }

    // ------------------------------------------------------------------------
    /// The state the main window opens in: as it was last saved, or the
    /// default size.
    pub fn window_state(&self) -> WindowState {
        self.config.borrow().window().unwrap_or(DEFAULT_STATE)
    }

    // ------------------------------------------------------------------------
    /// The window's state changed: a drag ended, it was maximized or
    /// restored, or it closes. Persisted; a failure is only logged, the
    /// window works on.
    pub fn on_window_state(&self, state: WindowState) {
        if let Err(e) = self.config.borrow_mut().set_window(state) {
            log::warn!("Could not save the window state: {e}");
        }
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;

    #[test]
    fn the_window_opens_as_it_was_saved() {
        let dir = std::env::temp_dir().join(format!("avelor-shell-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        let shell = Shell::new(load_config(&dir).unwrap());
        assert_eq!(shell.window_state(), DEFAULT_STATE);
        let state = WindowState {
            width: 900,
            height: 700,
            maximized: true,
        };
        shell.on_window_state(state);

        let shell = Shell::new(load_config(&dir).unwrap());
        assert_eq!(shell.window_state(), state);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
