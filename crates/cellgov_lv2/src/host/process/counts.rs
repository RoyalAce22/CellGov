//! Per-class live-object counters that exist only to feed
//! [`sys_process_get_number_of_object`](super::dispatch_process_get_number_of_object).
//!
//! These primitives are stubbed at the ID-allocator level (no real
//! kernel-side state), so the count is tracked here in a side-table
//! instead of being derived from a primary store like the other
//! [`Lv2Host`] tables.

use cellgov_ps3_abi::sys_process::{
    ProcessObjectClassId, SYS_COND_OBJECT, SYS_EVENT_FLAG_OBJECT, SYS_EVENT_PORT_OBJECT,
    SYS_EVENT_QUEUE_OBJECT, SYS_FS_FD_OBJECT, SYS_LWCOND_OBJECT, SYS_LWMUTEX_OBJECT,
    SYS_MUTEX_OBJECT, SYS_RWLOCK_OBJECT, SYS_SEMAPHORE_OBJECT, SYS_TIMER_OBJECT,
};

use crate::host::Lv2Host;

/// Counters for primitives stubbed as ID allocators only.
///
/// Not folded into [`Lv2Host::state_hash`] -- counts are derived
/// helpers that only move when something else also moves.
#[derive(Debug, Clone, Default)]
pub(in crate::host) struct ProcessCounts {
    timer: u32,
    rwlock: u32,
    event_port: u32,
    lwcond: u32,
    /// Live count of file descriptors opened via `sys_fs_open`;
    /// feeds the `SYS_FS_FD_OBJECT` (0x73) query.
    fs_fd: u32,
}

impl ProcessCounts {
    pub(in crate::host) fn new() -> Self {
        Self::default()
    }

    pub(in crate::host) fn timer_inc(&mut self) {
        self.timer = self.timer.saturating_add(1);
    }

    pub(in crate::host) fn timer_dec(&mut self) {
        self.timer = self.timer.saturating_sub(1);
    }

    pub(in crate::host) fn rwlock_inc(&mut self) {
        self.rwlock = self.rwlock.saturating_add(1);
    }

    pub(in crate::host) fn rwlock_dec(&mut self) {
        self.rwlock = self.rwlock.saturating_sub(1);
    }

    pub(in crate::host) fn event_port_inc(&mut self) {
        self.event_port = self.event_port.saturating_add(1);
    }

    pub(in crate::host) fn event_port_dec(&mut self) {
        self.event_port = self.event_port.saturating_sub(1);
    }

    pub(in crate::host) fn lwcond_inc(&mut self) {
        self.lwcond = self.lwcond.saturating_add(1);
    }

    pub(in crate::host) fn lwcond_dec(&mut self) {
        self.lwcond = self.lwcond.saturating_sub(1);
    }

    /// No decrement counterpart: real PS3's `sys_fs_close` does not
    /// drop the kernel-side fs-object count synchronously, and the
    /// ps3autotests `sys_process` matrix shows `fs_fd` staying at 1
    /// after `fclose`.
    pub(in crate::host) fn fs_fd_inc(&mut self) {
        self.fs_fd = self.fs_fd.saturating_add(1);
    }

    /// Map a `SYS_*_OBJECT` class id to its active-object count.
    /// Unmodeled classes report zero. The primary-table counts
    /// (`mutexes.len()` etc.) live on [`Lv2Host`], so the host is
    /// borrowed alongside `self`.
    pub(in crate::host) fn count_for_class(
        &self,
        class_id: ProcessObjectClassId,
        host: &Lv2Host,
    ) -> u32 {
        // SYS_COND_OBJECT (0x86) is the heavy cond, syscall 105 path.
        match class_id {
            SYS_MUTEX_OBJECT => host.mutexes.len() as u32,
            SYS_COND_OBJECT => host.conds.len() as u32,
            SYS_RWLOCK_OBJECT => self.rwlock,
            SYS_EVENT_PORT_OBJECT => self.event_port,
            SYS_TIMER_OBJECT => self.timer,
            SYS_EVENT_QUEUE_OBJECT => host.event_queues.len() as u32,
            SYS_LWMUTEX_OBJECT => host.lwmutexes.len() as u32,
            SYS_SEMAPHORE_OBJECT => host.semaphores.len() as u32,
            SYS_LWCOND_OBJECT => self.lwcond,
            SYS_FS_FD_OBJECT => self.fs_fd,
            SYS_EVENT_FLAG_OBJECT => host.event_flags.len() as u32,
            _ => 0,
        }
    }
}

#[cfg(test)]
#[path = "tests/counts_tests.rs"]
mod tests;
