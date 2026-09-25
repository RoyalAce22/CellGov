//! The lwmutex hold counts and the fs-fd and lwcond object counters.

use cellgov_event::UnitId;

use crate::ppu_thread::PpuThreadId;

use super::model::Lv2Host;

impl Lv2Host {
    /// Distinct lwmutexes currently held by `tid`.
    pub fn lwmutex_holds_for(&self, tid: PpuThreadId) -> u32 {
        self.state.lwmutex_holds.get(tid).copied().unwrap_or(0)
    }

    /// Bumps the count for a first-acquire (FREE -> tid) or a
    /// kernel-side transfer. Recursive re-acquires (tid already
    /// the owner) are tracked elsewhere and must not pass through
    /// this entry.
    pub fn lwmutex_holds_inc(&mut self, tid: PpuThreadId) {
        let count = self.lwmutex_holds_for(tid);
        debug_assert!(count < u32::MAX, "lwmutex hold count overflow on {tid:?}",);
        self.state
            .lwmutex_holds
            .insert(tid, count.saturating_add(1));
    }

    /// Release builds saturate at 0 so a leak does not corrupt
    /// downstream counters.
    pub fn lwmutex_holds_dec(&mut self, tid: PpuThreadId) {
        if let Some(&count) = self.state.lwmutex_holds.get(tid) {
            debug_assert!(count > 0, "lwmutex hold count underflow on {tid:?}",);
            match count.saturating_sub(1) {
                0 => {
                    self.state.lwmutex_holds.remove(tid);
                }
                left => {
                    self.state.lwmutex_holds.insert(tid, left);
                }
            }
        } else {
            debug_assert!(
                false,
                "lwmutex_holds_dec on {tid:?} with no entry; inc/dec pairing leaked",
            );
        }
    }

    /// Used at thread-exit and stale-owner recovery so a dead
    /// thread's count does not leak.
    pub fn lwmutex_holds_clear(&mut self, tid: PpuThreadId) {
        self.state.lwmutex_holds.remove(tid);
    }

    /// `false` when `unit` has no PPU thread mapping.
    pub fn unit_holds_lwmutex(&self, unit: UnitId) -> bool {
        match self.state.ppu_threads.thread_id_for_unit(unit) {
            Some(tid) => self.lwmutex_holds_for(tid) > 0,
            None => false,
        }
    }

    /// See [`process::ProcessCounts::fs_fd_inc`](crate::host::process::ProcessCounts::fs_fd_inc) for the no-decrement
    /// contract.
    pub(in crate::host) fn fs_fd_count_inc(&mut self) {
        self.state.process_counts.fs_fd_inc();
    }

    /// Increment the live `sys_lwcond` object count.
    pub fn lwcond_count_inc(&mut self) {
        self.state.process_counts.lwcond_inc();
    }

    /// Decrement the live `sys_lwcond` count; saturates at 0.
    pub fn lwcond_count_dec(&mut self) {
        self.state.process_counts.lwcond_dec();
    }
}
