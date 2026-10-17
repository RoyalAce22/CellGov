//! `sys_process_*` ABI constants.
//!
//! `SYS_*_OBJECT` class ids for `sys_process_get_number_of_object`
//! (syscall 12): a single integer that selects which kernel-object
//! class to count. Real LV2 carries one count per class in
//! `sys_object`; CellGov maps each id to a table length or a
//! side-counter (see `cellgov_lv2::host::process`).

/// Numeric class id passed to `sys_process_get_number_of_object`.
pub type ProcessObjectClassId = u32;

/// Pid CellGov gives the boot process.
// CellGov's choice: no capture reads it. A game the XMB launches gets
// a pid that depends on launch history -- the lv2_process_queries,
// lv2_process_stack_small and lv2_process_classes captures read
// 0x011D0200, 0x011F0200 and 0x01370200 -- and vsh's own pid, the
// parent each of them reports, is 0x01000300.
pub const BOOT_PROCESS_PID: u32 = 0x0100_0500;

/// Ppid the boot process reports: vsh's pid, as a game the XMB launched
/// reads it.
// The lv2_process_queries and lv2_process_classes captures read it from
// a process the XMB launched. It is the right parent for a game boot
// and not for a vsh boot, which has no captured parent.
pub const BOOT_PROCESS_PPID: u32 = 0x0100_0300;

/// `sys_process_get_status` return value for a live process.
// The lv2_process_queries capture reads 2 for the calling process and
// for its parent.
pub const SYS_PROCESS_STATUS_LIVE: u64 = 2;

/// One past the highest `sys_process_param.primary_prio` the kernel
/// adopts; a declaration at or above it leaves the default of 1001
/// standing rather than failing the load.
// The console captures of tests/micro/lv2_process_prio_3071 and
// tests/micro/lv2_process_prio_3072: a process declaring 3071 runs its
// primary thread at 3071, and one declaring 3072 launches and runs it
// at 1001.
pub const SYS_PROCESS_PARAM_PRIO_LIMIT: i32 = 3072;

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

/// `sys_exit2_param`, the block `_sys_process_exit2` (26) carries.
///
/// liblv2.sprx's `sys_game_process_exitspawn` marshals the block
/// before it issues the syscall. The header size, the argv-array slot
/// and the data reservation come from that caller's layout:
///
/// - `tag@0`
/// - `this_size@8` -- the header size
/// - `next_size@0x10` -- the marshalled region past the header
/// - `prio@0x18`
/// - `flags@0x20`
/// - `args@0x28`
///
/// What the kernel does with the priority and the flags across the
/// handoff is unestablished. A probe the console runner starts hangs in
/// the exitspawn whatever it names, its own EBOOT included, so no
/// capture answers it yet.
pub mod exit2_param {
    /// Bytes of header ahead of the marshalled strings.
    pub const HEADER_SIZE: u32 = 0x30;
    /// `args` -- guest pointer to the marshalled pointer array: argv
    /// strings, NULL, envp strings, NULL.
    pub const ARGV_ARRAY_OFFSET: u32 = 0x28;
    /// Size of the caller's opaque data blob, which the marshaller
    /// puts in the last bytes of the block.
    pub const DATA_BLOB_SIZE: u32 = 0x1000;
    /// An `arg_size` above this carries a [`DATA_BLOB_SIZE`] blob at
    /// the tail of the block. A block of exactly this size carries
    /// none.
    pub const DATA_BLOB_THRESHOLD: u32 = HEADER_SIZE + DATA_BLOB_SIZE;
}

/// Every class `sys_process_get_number_of_object` counts, in numeric
/// order. Any other class is EINVAL.
// The console capture of tests/micro/lv2_process_classes sweeps every
// class from 0x00 to 0xFF: these 24 answer CELL_OK and the other 232
// answer EINVAL. `kernel_id` names 0x08 (shared memory) and 0x41 / 0x42
// (config handles and listeners), the lv2_prx_start_gate capture reads
// a PRX id in class 0x23, and no capture names the others.
pub const COUNTED_PROCESS_OBJECT_CLASS_IDS: &[ProcessObjectClassId] = &[
    super::kernel_id::SHARED_MEMORY_CLASS,
    0x0A,
    0x0B,
    SYS_EVENT_PORT_OBJECT,
    SYS_TIMER_OBJECT,
    0x21,
    0x22,
    0x23,
    0x24,
    super::kernel_id::CONFIG_HANDLE_CLASS,
    super::kernel_id::CONFIG_LISTENER_CLASS,
    0x43,
    0x44,
    0x45,
    0x65,
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

/// The class ids CellGov keeps a count for, in numeric order. The
/// class-id coverage test in `cellgov_lv2::host::process::counts`
/// drives off this slice.
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
#[path = "tests/process_tests.rs"]
mod tests;
