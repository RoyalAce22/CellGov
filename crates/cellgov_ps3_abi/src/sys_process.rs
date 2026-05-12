//! Constants from `sys/process.h`.
//!
//! `SYS_*_OBJECT` class ids for `sys_process_get_number_of_object`
//! (syscall 24): a single integer that selects which kernel-object
//! class to count. Real LV2 carries one count per class in
//! `sys_object`; CellGov maps each id to a table length or a
//! side-counter (see `cellgov_lv2::host::process`).

/// Numeric class id passed to `sys_process_get_number_of_object`.
pub type ProcessObjectClassId = u32;

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

/// Every documented class id, in numeric order. Adding a new
/// constant above without listing it here is a regression: the
/// class-id coverage test in `cellgov_lv2::host::process::counts`
/// drives off this slice, and a class consumed by
/// `sys_process_get_number_of_object` but not enumerated here
/// would silently fall through to the count handler's catch-all
/// and report zero forever.
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
mod tests {
    use super::*;

    #[test]
    fn all_class_ids_are_unique() {
        let mut sorted: Vec<u32> = ALL_PROCESS_OBJECT_CLASS_IDS.to_vec();
        sorted.sort_unstable();
        for window in sorted.windows(2) {
            assert_ne!(
                window[0], window[1],
                "duplicate class id 0x{:02X} in ALL_PROCESS_OBJECT_CLASS_IDS",
                window[0]
            );
        }
    }
}
