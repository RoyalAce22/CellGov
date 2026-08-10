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
/// Folded into [`Lv2Host::state_hash`] when non-zero: the counters
/// feed `sys_process_get_number_of_object`'s return value and,
/// except `event_port`, are tracked nowhere else.
#[derive(Debug, Clone, Default)]
pub(in crate::host) struct ProcessCounts {
    timer: u32,
    rwlock: u32,
    /// Mirrors the hashed `event_ports` table's live count: the
    /// dispatch arms pair inc with create and dec with a successful
    /// destroy.
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

    /// True when every counter is zero.
    pub(in crate::host) fn is_empty(&self) -> bool {
        let Self {
            timer,
            rwlock,
            event_port,
            lwcond,
            fs_fd,
        } = self;
        *timer == 0 && *rwlock == 0 && *event_port == 0 && *lwcond == 0 && *fs_fd == 0
    }

    /// FNV-1a over every counter, via raw little-endian bytes per the
    /// host state-hash contract. The exhaustive destructure makes an
    /// unfolded new counter a compile error.
    pub(in crate::host) fn state_hash(&self) -> u64 {
        let Self {
            timer,
            rwlock,
            event_port,
            lwcond,
            fs_fd,
        } = self;
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for counter in [timer, rwlock, event_port, lwcond, fs_fd] {
            hasher.write(&counter.to_le_bytes());
        }
        hasher.finish()
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
            SYS_MUTEX_OBJECT => host.state.mutexes.len() as u32,
            SYS_COND_OBJECT => host.state.conds.len() as u32,
            SYS_RWLOCK_OBJECT => self.rwlock,
            SYS_EVENT_PORT_OBJECT => self.event_port,
            SYS_TIMER_OBJECT => self.timer,
            SYS_EVENT_QUEUE_OBJECT => host.state.event_queues.len() as u32,
            SYS_LWMUTEX_OBJECT => host.state.lwmutexes.len() as u32,
            SYS_SEMAPHORE_OBJECT => host.state.semaphores.len() as u32,
            SYS_LWCOND_OBJECT => self.lwcond,
            SYS_FS_FD_OBJECT => self.fs_fd,
            SYS_EVENT_FLAG_OBJECT => host.state.event_flags.len() as u32,
            _ => 0,
        }
    }
}

#[cfg(test)]
#[path = "tests/counts_tests.rs"]
mod tests;
