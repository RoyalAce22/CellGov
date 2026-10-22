//! A cap on concurrent `cellgov boot run` subprocesses, sized so their
//! peak memory fits what the host has free.
#![allow(
    dead_code,
    reason = "each integration-test binary compiles this module separately and uses a subset"
)]

use std::sync::{Condvar, Mutex};

/// A counting semaphore over boot subprocesses.
pub struct BootSlots {
    limit: usize,
    available: Mutex<usize>,
    cv: Condvar,
}

impl BootSlots {
    /// One slot per `per_slot_bytes` of the host's available memory, at
    /// least one. A non-negative integer in `override_env` replaces that
    /// count, with the same floor.
    ///
    /// # Panics
    ///
    /// If `override_env` is set to a value that is not a non-negative
    /// integer.
    pub fn new(override_env: &str, per_slot_bytes: u64) -> Self {
        let limit = compute_limit(override_env, per_slot_bytes);
        Self {
            limit,
            available: Mutex::new(limit),
            cv: Condvar::new(),
        }
    }

    /// How many boots may run at once.
    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Wait for a free slot; the slot frees when the permit drops.
    pub fn acquire(&self) -> Permit<'_> {
        let mut g = self.available.lock().expect("boot slot mutex poisoned");
        while *g == 0 {
            g = self.cv.wait(g).expect("boot slot condvar wait failed");
        }
        *g -= 1;
        Permit { slots: self }
    }
}

/// One held slot.
pub struct Permit<'a> {
    slots: &'a BootSlots,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut g = self
            .slots
            .available
            .lock()
            .expect("boot slot mutex poisoned");
        *g += 1;
        self.slots.cv.notify_one();
    }
}

fn compute_limit(override_env: &str, per_slot_bytes: u64) -> usize {
    if let Ok(s) = std::env::var(override_env) {
        let n: usize = s
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("{override_env}={s:?}: not a non-negative integer ({e})"));
        // 0 clamps to the floor rather than deadlocking on a semaphore
        // no permit can ever be taken from.
        return n.max(1);
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    // `available_memory` reflects what the OS will hand out;
    // `total_memory` ignores RAM held by other processes.
    let ram = sys.available_memory();
    ((headroom(&sys, ram) / per_slot_bytes) as usize).max(1)
}

/// Windows refuses an allocation past the commit limit even with RAM
/// free; sysinfo reports the commit headroom there as `free_swap`.
#[cfg(windows)]
fn headroom(sys: &sysinfo::System, ram: u64) -> u64 {
    ram.min(sys.free_swap())
}

#[cfg(not(windows))]
fn headroom(_sys: &sysinfo::System, ram: u64) -> u64 {
    ram
}
