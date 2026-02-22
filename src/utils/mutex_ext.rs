use std::sync::{Mutex, MutexGuard};

/// Extension trait for [`Mutex`] that provides infallible locking with poison recovery.
pub trait MutexExt<T> {
    /// Lock the mutex, recovering from poison by returning the inner value.
    ///
    /// If a previous thread panicked while holding the lock, the poison is cleared
    /// and the guard is returned with the data intact. This is appropriate for cases
    /// where the data is consistent even if a panic occurred (e.g. progress tracking).
    fn lock_infallible(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_infallible(&self) -> MutexGuard<'_, T> {
        self.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
