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
/// helpers, not primary state. They only move when something else
/// also moves; do not reflexively add them to the hash.
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
mod tests {
    use super::*;
    use cellgov_ps3_abi::sys_process::ALL_PROCESS_OBJECT_CLASS_IDS;

    /// Drives off [`ALL_PROCESS_OBJECT_CLASS_IDS`] so a new constant
    /// landing in `cellgov_ps3_abi::sys_process` without a
    /// `count_for_class` arm shows up as a test failure rather than
    /// as a silent zero forever. Empty host: every documented class
    /// reports 0.
    #[test]
    fn count_for_class_covers_every_documented_class_id() {
        let host = Lv2Host::new();
        let counts = ProcessCounts::new();
        for &class in ALL_PROCESS_OBJECT_CLASS_IDS {
            assert_eq!(
                counts.count_for_class(class, &host),
                0,
                "fresh host must report 0 for class 0x{class:02X}"
            );
        }
        // Unknown class falls to zero.
        assert_eq!(counts.count_for_class(0xFF, &host), 0);
    }

    /// Catches the strongest realistic regression: someone adds a
    /// constant to [`ALL_PROCESS_OBJECT_CLASS_IDS`] (so the coverage
    /// test passes vacuously) but forgets to add a `count_for_class`
    /// arm. Bumping each counter and asserting at least one class
    /// becomes nonzero exercises the wiring beyond the empty-host
    /// case.
    #[test]
    fn each_counter_bump_moves_a_documented_class() {
        let host = Lv2Host::new();
        let mut counts = ProcessCounts::new();
        for (label, bump, expected_class) in [
            (
                "timer",
                &ProcessCounts::timer_inc as &dyn Fn(&mut ProcessCounts),
                SYS_TIMER_OBJECT,
            ),
            (
                "rwlock",
                &ProcessCounts::rwlock_inc as &dyn Fn(&mut ProcessCounts),
                SYS_RWLOCK_OBJECT,
            ),
            (
                "event_port",
                &ProcessCounts::event_port_inc as &dyn Fn(&mut ProcessCounts),
                SYS_EVENT_PORT_OBJECT,
            ),
            (
                "lwcond",
                &ProcessCounts::lwcond_inc as &dyn Fn(&mut ProcessCounts),
                SYS_LWCOND_OBJECT,
            ),
            (
                "fs_fd",
                &ProcessCounts::fs_fd_inc as &dyn Fn(&mut ProcessCounts),
                SYS_FS_FD_OBJECT,
            ),
        ] {
            let before = counts.count_for_class(expected_class, &host);
            bump(&mut counts);
            let after = counts.count_for_class(expected_class, &host);
            assert_eq!(
                after,
                before + 1,
                "{label}: bump did not increment count for class 0x{expected_class:02X}"
            );
        }
    }

    #[test]
    fn counter_classes_observe_inc_dec() {
        let host = Lv2Host::new();
        let mut counts = ProcessCounts::new();
        counts.timer_inc();
        counts.timer_inc();
        counts.rwlock_inc();
        counts.event_port_inc();
        counts.lwcond_inc();
        counts.fs_fd_inc();
        assert_eq!(counts.count_for_class(SYS_TIMER_OBJECT, &host), 2);
        assert_eq!(counts.count_for_class(SYS_RWLOCK_OBJECT, &host), 1);
        assert_eq!(counts.count_for_class(SYS_EVENT_PORT_OBJECT, &host), 1);
        assert_eq!(counts.count_for_class(SYS_LWCOND_OBJECT, &host), 1);
        assert_eq!(counts.count_for_class(SYS_FS_FD_OBJECT, &host), 1);

        counts.timer_dec();
        counts.rwlock_dec();
        counts.lwcond_dec();
        assert_eq!(counts.count_for_class(SYS_TIMER_OBJECT, &host), 1);
        assert_eq!(counts.count_for_class(SYS_RWLOCK_OBJECT, &host), 0);
        assert_eq!(counts.count_for_class(SYS_LWCOND_OBJECT, &host), 0);
    }

    #[test]
    fn dec_saturates_at_zero() {
        let mut counts = ProcessCounts::new();
        counts.timer_dec();
        counts.rwlock_dec();
        counts.event_port_dec();
        counts.lwcond_dec();
        let host = Lv2Host::new();
        assert_eq!(counts.count_for_class(SYS_TIMER_OBJECT, &host), 0);
        assert_eq!(counts.count_for_class(SYS_RWLOCK_OBJECT, &host), 0);
        assert_eq!(counts.count_for_class(SYS_EVENT_PORT_OBJECT, &host), 0);
        assert_eq!(counts.count_for_class(SYS_LWCOND_OBJECT, &host), 0);
    }
}
