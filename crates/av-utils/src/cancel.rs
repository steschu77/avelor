//! Cancelling from another thread.
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

// ----------------------------------------------------------------------------
/// Cancels the work it is attached to, from any thread. Cancelling after the
/// work finished does nothing.
#[derive(Clone, Default)]
pub struct Cancel(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    cancelled: bool,
}

impl Cancel {
    // ------------------------------------------------------------------------
    pub fn new() -> Self {
        Self::default()
    }

    // ------------------------------------------------------------------------
    pub fn cancel(&self) {
        self.lock().cancelled = true;
    }

    // ------------------------------------------------------------------------
    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    // ------------------------------------------------------------------------
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
