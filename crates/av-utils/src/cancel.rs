//! Cancelling from another thread.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::error::{Error, Result};

// ----------------------------------------------------------------------------
/// Cancels the work it is attached to, from any thread. Cancelling after the
/// work finished does nothing.
#[derive(Clone, Default)]
pub struct Cancel(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    cancelled: bool,
    /// The aborts of the work open under this `Cancel`, by their hook's key.
    hooks: Vec<(u64, Box<dyn FnOnce() + Send>)>,
    next_key: u64,
}

impl Cancel {
    // ------------------------------------------------------------------------
    pub fn new() -> Self {
        Self::default()
    }

    // ------------------------------------------------------------------------
    pub fn cancel(&self) {
        let mut state = self.lock();
        state.cancelled = true;
        for (_, abort) in state.hooks.drain(..) {
            abort();
        }
    }

    // ------------------------------------------------------------------------
    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    // ------------------------------------------------------------------------
    /// Run `abort` when this is cancelled, while the returned `Hook` holds
    /// it: how work that blocks (a request waiting on the network) stops at
    /// once. `Error::Cancelled` if this is cancelled already.
    pub fn on_cancel(&self, abort: impl FnOnce() + Send + 'static) -> Result<Hook> {
        let mut state = self.lock();
        if state.cancelled {
            return Err(Error::Cancelled);
        }
        let key = state.next_key;
        state.next_key += 1;
        state.hooks.push((key, Box::new(abort)));
        Ok(Hook {
            cancel: self.clone(),
            key,
        })
    }

    // ------------------------------------------------------------------------
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

// ----------------------------------------------------------------------------
/// An abort `Cancel::on_cancel` holds; dropped, it is taken back unrun.
pub struct Hook {
    cancel: Cancel,
    key: u64,
}

impl Hook {
    // ------------------------------------------------------------------------
    /// Take the abort back unrun: `false` if the cancel ran it already.
    pub fn release(&self) -> bool {
        let mut state = self.cancel.lock();
        let i = state.hooks.iter().position(|(key, _)| *key == self.key);
        i.map(|i| state.hooks.swap_remove(i)).is_some()
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        self.release();
    }
}

// ----------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // ------------------------------------------------------------------------
    #[test]
    fn a_cancel_runs_the_aborts_it_holds_once() {
        let runs = Arc::new(AtomicUsize::new(0));
        let cancel = Cancel::new();
        let counted = Arc::clone(&runs);
        let hook = cancel.on_cancel(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        cancel.cancel();
        cancel.cancel();
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(!hook.unwrap().release(), "the cancel ran it");
        assert!(matches!(cancel.on_cancel(|| ()), Err(Error::Cancelled)));
    }

    // ------------------------------------------------------------------------
    #[test]
    fn an_abort_taken_back_doesnt_run() {
        let runs = Arc::new(AtomicUsize::new(0));
        let cancel = Cancel::new();
        let counted = Arc::clone(&runs);
        let released = cancel.on_cancel(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        assert!(released.unwrap().release());
        let counted = Arc::clone(&runs);
        drop(cancel.on_cancel(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }));
        cancel.cancel();
        assert_eq!(runs.load(Ordering::SeqCst), 0);
    }
}
