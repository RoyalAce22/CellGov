//! Constants from `sys/process.h`.
//!
//! `SYS_*_OBJECT` class ids for `sys_process_get_number_of_object`
//! (syscall 24): a single integer that selects which kernel-object
//! class to count. Real LV2 carries one count per class in
//! `sys_object`; CellGov maps each id to a table length or a
//! side-counter (see `cellgov_lv2::host::process`).

/// Numeric class id passed to `sys_process_get_number_of_object`.
pub type ProcessObjectClassId = u32;

/// Pid LV2 assigns the first non-kernel process.
// Firmware observation: sys_process_getpid returns this to the
// first user process on real hardware; PSL1GHT keys on the value.
pub const BOOT_PROCESS_PID: u32 = 0x0100_0500;

/// Ppid the boot process reports; `sys_process_get_ppu_guid`
/// returns the same value and PSL1GHT keys on the equality.
// Firmware observation, same provenance as BOOT_PROCESS_PID.
pub const BOOT_PROCESS_PPID: u32 = 0x0100_0300;

/// `sys_event_port` objects.
pub const SYS_EVENT_PORT_OBJECT: ProcessObjectClassId = 0x0E;
/// `sys_timer` objects.
pub const SYS_TIMER_OBJECT: ProcessObjectClassId = 0x11;
/// File descriptors opened via `sys_fs_open` / `sys_fs_opendir`.
pub const SYS_FS_FD_OBJECT: ProcessObjectClassId = 0x73;
/// `sys_mutex` objects.
pub const SYS_MUTEX_OBJECT: ProcessObjectClassId = 0x85;
/// Heavy `sys_cond` objects (syscall 105 path).
pub const SYS_COND_OBJECT: ProcessObjectClassId = 0x86;
/// `sys_rwlock` objects.
pub const SYS_RWLOCK_OBJECT: ProcessObjectClassId = 0x88;
/// `sys_event_queue` objects.
pub const SYS_EVENT_QUEUE_OBJECT: ProcessObjectClassId = 0x8D;
/// `sys_lwmutex` objects.
pub const SYS_LWMUTEX_OBJECT: ProcessObjectClassId = 0x95;
/// `sys_semaphore` objects.
pub const SYS_SEMAPHORE_OBJECT: ProcessObjectClassId = 0x96;
/// Light-weight `sys_lwcond` objects.
pub const SYS_LWCOND_OBJECT: ProcessObjectClassId = 0x97;
/// `sys_event_flag` objects.
pub const SYS_EVENT_FLAG_OBJECT: ProcessObjectClassId = 0x98;

/// Every documented class id, in numeric order. The class-id
/// coverage test in `cellgov_lv2::host::process::counts` drives
/// off this slice.
pub const ALL_PROCESS_OBJECT_CLASS_IDS: &[ProcessObjectClassId] = &[
    SYS_EVENT_PORT_OBJECT,
    SYS_TIMER_OBJECT,
    SYS_FS_FD_OBJECT,
    SYS_MUTEX_OBJECT,
    SYS_COND_OBJECT,
    SYS_RWLOCK_OBJECT,
    SYS_EVENT_QUEUE_OBJECT,
    SYS_LWMUTEX_OBJECT,
    SYS_SEMAPHORE_OBJECT,
    SYS_LWCOND_OBJECT,
    SYS_EVENT_FLAG_OBJECT,
];

#[cfg(test)]
#[path = "tests/sys_process_tests.rs"]
mod tests;
