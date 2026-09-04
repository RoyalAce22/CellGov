//! The guard that serializes the tests under `progress` against the
//! process-global bar state.

use std::sync::{Mutex, MutexGuard, PoisonError};

static SERIAL: Mutex<()> = Mutex::new(());

/// Take the process-global bar registration for one test body.
///
/// Two kinds of test must take this guard:
///
/// - a test that reads or writes `LIVE_BAR`;
/// - a test that panics, such as a `#[should_panic]` case anywhere
///   under `progress`.
///
/// A live bar installs a panic hook for the rest of the process, and
/// that hook clears `LIVE_BAR` on any panic. Such a panic also poisons
/// this mutex. The hook leaves the slot empty, so this function
/// recovers the guard.
pub(super) fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}
