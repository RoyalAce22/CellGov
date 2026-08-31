//! The counters a worker writes and the render thread reads.

use super::sink::ProgressSink;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Shared task state: written by the worker through the sink trait,
/// read by the render thread. Counters are `Relaxed`; nothing orders
/// on them.
#[derive(Debug)]
pub struct ProgressState {
    phase: AtomicU8,
    total_amount: AtomicU64,
    done_amount: AtomicU64,
    /// Counts toward the ratio but not the rate.
    preset_amount: AtomicU64,
    total_items: AtomicUsize,
    done_items: AtomicUsize,
    /// Touched once per item, never per piece.
    current: Mutex<String>,
    pub(crate) finished: AtomicBool,
}

impl ProgressState {
    pub(crate) fn new() -> Self {
        Self {
            phase: AtomicU8::new(0),
            total_amount: AtomicU64::new(0),
            done_amount: AtomicU64::new(0),
            preset_amount: AtomicU64::new(0),
            total_items: AtomicUsize::new(0),
            done_items: AtomicUsize::new(0),
            current: Mutex::new(String::new()),
            finished: AtomicBool::new(false),
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            phase: self.phase.load(Ordering::Relaxed),
            total_amount: self.total_amount.load(Ordering::Relaxed),
            done_amount: self.done_amount.load(Ordering::Relaxed),
            preset_amount: self.preset_amount.load(Ordering::Relaxed),
            total_items: self.total_items.load(Ordering::Relaxed),
            done_items: self.done_items.load(Ordering::Relaxed),
            current: self
                .current
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        }
    }
}

impl ProgressSink for ProgressState {
    fn phase(&self, code: u8) {
        self.phase.store(code, Ordering::Relaxed);
    }
    fn totals(&self, items: usize, amount: u64) {
        self.total_items.store(items, Ordering::Relaxed);
        self.total_amount.store(amount, Ordering::Relaxed);
    }
    fn preset_done(&self, amount: u64) {
        // Both counters take the high-water mark, so a repeated or
        // lowered preset cannot leave `preset_amount` under a
        // `done_amount` it already raised. The preset is raised first
        // so a reader that catches the pair mid-update lands on the
        // understating side.
        self.preset_amount.fetch_max(amount, Ordering::Relaxed);
        self.done_amount.fetch_max(amount, Ordering::Relaxed);
    }
    fn item_started(&self, name: &str) {
        let mut cur = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cur.clear();
        cur.push_str(name);
    }
    fn advanced(&self, delta: u64) {
        self.done_amount.fetch_add(delta, Ordering::Relaxed);
    }
    fn item_finished(&self) {
        self.done_items.fetch_add(1, Ordering::Relaxed);
    }
    fn finished(&self) {
        self.finished.store(true, Ordering::Relaxed);
    }
}

/// A point-in-time copy of the state, for pure frame composition.
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub(crate) phase: u8,
    pub(crate) total_amount: u64,
    pub(crate) done_amount: u64,
    pub(crate) preset_amount: u64,
    pub(crate) total_items: usize,
    pub(crate) done_items: usize,
    pub(crate) current: String,
}

impl Snapshot {
    /// Amount this run actually moved, excluding a resumed transfer's
    /// preset. The rate measures this.
    pub(crate) fn advanced(&self) -> u64 {
        self.done_amount.saturating_sub(self.preset_amount)
    }
}

#[cfg(test)]
#[path = "tests/state_tests.rs"]
mod tests;
