//! The thread-group table: its maps, its construction, and one file of
//! methods per concern.

mod hash;
mod lifecycle;
mod queues;
mod run_control;
mod signal;
mod threads;

#[cfg(doc)]
use super::NotifySpuFinishedError;
use super::ThreadGroup;
use cellgov_event::UnitId;
use cellgov_mem::lanes::{source, LaneMap};
use cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID;
#[cfg(doc)]
use cellgov_ps3_abi::lv2::spu::pool;

/// Table of managed SPU thread groups.
///
/// `unit_to_group` and `finished_units` partition the SPU
/// registration space: a unit lives in exactly one of them, and
/// `notify_spu_finished` moves entries from the former to the
/// latter. Keeping them distinct is what lets a double-notify
/// surface as [`NotifySpuFinishedError::AlreadyFinished`] rather
/// than re-decrementing the counter.
#[derive(Debug, Clone)]
pub struct ThreadGroupTable {
    groups: LaneMap<u32, ThreadGroup>,
    unit_to_group: LaneMap<UnitId, u32>,
    finished_units: LaneMap<UnitId, u32>,
    /// Synthetic `group_id * 256 + slot` -> raw runtime `UnitId`.
    thread_id_to_unit: LaneMap<u32, u64>,
    next_id: u32,
    /// The physical SPUs each process's `sys_spu_initialize` set aside
    /// for Raw SPUs, by pid; the rest of [`pool::USABLE_SPUS`] runs that
    /// process's SPU threads. A process with no entry sets none aside.
    // The oracle keeps the split and the group count per process. A
    // console probe runs as one process, so no capture shows it.
    raw_limits: LaneMap<u32, u32>,
}

/// The hash object of a process's SPU pool split: 0 for the boot
/// process, its pid for any other.
fn split_object(pid: u32) -> u64 {
    if pid == BOOT_PROCESS_PID {
        0
    } else {
        u64::from(pid)
    }
}

impl Default for ThreadGroupTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ThreadGroupTable {
    /// Construct an empty table.
    pub fn new() -> Self {
        Self {
            groups: LaneMap::new(source::THREAD_GROUP, u64::from),
            unit_to_group: LaneMap::new(source::GROUP_UNIT, UnitId::raw),
            finished_units: LaneMap::new(source::GROUP_FINISHED_UNIT, UnitId::raw),
            thread_id_to_unit: LaneMap::new(source::GROUP_THREAD_ID, u64::from),
            next_id: 1,
            raw_limits: LaneMap::new(source::SPU_LIMITS, split_object),
        }
    }
}

#[cfg(test)]
#[path = "tests/thread_group_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/thread_group_event_tests.rs"]
mod event_tests;

#[cfg(test)]
#[path = "tests/thread_group_restart_tests.rs"]
mod restart_tests;
