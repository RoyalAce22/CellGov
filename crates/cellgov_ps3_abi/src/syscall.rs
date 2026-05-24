//! LV2 syscall numbers (the value the guest puts in r11 before `sc`).
//!
//! Names mirror RPCS3's `syscall_table_t` entries. Behaviour
//! (the dispatch match in `cellgov_lv2::request::classify`) lives in
//! `cellgov_lv2`; this module is data only.
//!
//! All public `u64` syscall constants are emitted by the
//! `lv2_syscalls!` macro from a single declarative source: a docstring,
//! a name, and a number. The macro emits the per-syscall `pub const`,
//! the public number array, and a `#[cfg(test)]` named array used by
//! the collision tests.

/// Emit a group of LV2 syscall `pub const`s plus a derived number
/// array and a test-only named array, from a single declarative list.
///
/// Grammar:
///
/// ```text
/// lv2_syscalls! {
///     $(#[doc = "..."])* NUMBERS_ARRAY;
///     NAMED_ARRAY;
///     $( /// docs. NAME = number; )*
/// }
/// ```
///
/// Emits:
///
/// - `pub const NAME: u64 = number;` for each entry, with its docs.
/// - `pub const NUMBERS_ARRAY: &[u64] = &[NAME, ...];` for the
///   collected numbers in declaration order.
/// - `#[cfg(test)] const NAMED_ARRAY: &[(&'static str, u64)] = &[
///   (stringify!(NAME), NAME), ...];` for the collision tests
///   that need to refer to each constant by both symbol and value.
macro_rules! lv2_syscalls {
    (
        $(#[$arr_attr:meta])*
        $arr:ident;
        $named:ident;
        $( $(#[$attr:meta])* $name:ident = $value:expr; )*
    ) => {
        $(
            $(#[$attr])*
            pub const $name: u64 = $value;
        )*

        $(#[$arr_attr])*
        pub const $arr: &[u64] = &[ $( $name ),* ];

        #[cfg(test)]
        #[allow(dead_code)]
        const $named: &[(&'static str, u64)] = &[
            $( (stringify!($name), $name) ),*
        ];
    };
}

lv2_syscalls! {
    /// Every LV2 syscall number this module exposes as a typed-arm
    /// `pub const`, in declaration order. Consumers iterate this to
    /// drive classifier-coverage cross-checks (e.g.
    /// `every_lv2_syscall_with_narrowing_appears_in_a_table` in
    /// `cellgov_lv2::request::classify`).
    ///
    /// # Invariant
    ///
    /// Every typed-arm `pub const FOO: u64 = ...` emitted by the
    /// macro appears here exactly once. The macro derives both this
    /// list and the test-only `ALL_LV2_NAMED` from the same source;
    /// the `all_lv2_numbers_are_unique` test pins the uniqueness
    /// half. The `unsupported_routed_syscall_numbers_do_not_collide_with_typed_arms`
    /// test pins disjointness against the unsupported-routed set.
    ALL_LV2_NUMBERS;
    ALL_LV2_NAMED;

    /// `sys_process_getpid`.
    PROCESS_GETPID = 1;

    /// `sys_process_get_number_of_object`.
    PROCESS_GET_NUMBER_OF_OBJECT = 12;

    /// `sys_process_is_spu_lock_line_reservation_address` -- check
    /// whether `addr` falls in the SPU lock-line reservation range.
    /// Behavioural oracle:
    /// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_process.cpp:263`.
    PROCESS_IS_SPU_LOCK_LINE_RESERVATION_ADDRESS = 14;

    /// `sys_process_getppid`.
    PROCESS_GETPPID = 18;

    /// `sys_process_exit`.
    PROCESS_EXIT = 22;

    /// `sys_process_get_sdk_version`.
    PROCESS_GET_SDK_VERSION = 25;

    /// `_sys_process_get_paramsfo`.
    PROCESS_GET_PARAMSFO = 30;

    /// `sys_process_get_ppu_guid`.
    PROCESS_GET_PPU_GUID = 31;

    /// `sys_timer_create`.
    TIMER_CREATE = 70;
    /// `sys_timer_destroy`.
    TIMER_DESTROY = 71;

    /// `sys_timer_usleep`.
    TIMER_USLEEP = 141;
    /// `sys_timer_sleep`.
    TIMER_SLEEP = 142;

    /// `sys_rwlock_create`.
    RWLOCK_CREATE = 120;
    /// `sys_rwlock_destroy`.
    RWLOCK_DESTROY = 121;

    /// `sys_event_port_create`.
    EVENT_PORT_CREATE = 134;
    /// `sys_event_port_destroy`.
    EVENT_PORT_DESTROY = 135;

    /// `sys_ppu_thread_exit`.
    PPU_THREAD_EXIT = 41;
    /// `sys_ppu_thread_yield`.
    PPU_THREAD_YIELD = 43;
    /// `sys_ppu_thread_join`.
    PPU_THREAD_JOIN = 44;
    /// `_sys_ppu_thread_create` (LV2-side; sysPrxForUser wraps it).
    PPU_THREAD_CREATE = 52;
    /// `sys_ppu_thread_start`.
    PPU_THREAD_START = 53;

    /// `sys_event_flag_create`.
    EVENT_FLAG_CREATE = 82;
    /// `sys_event_flag_destroy`.
    EVENT_FLAG_DESTROY = 83;
    /// `sys_event_flag_wait`.
    EVENT_FLAG_WAIT = 85;
    /// `sys_event_flag_trywait`.
    EVENT_FLAG_TRY_WAIT = 86;
    /// `sys_event_flag_set`.
    EVENT_FLAG_SET = 87;

    /// `sys_semaphore_create`.
    SEMAPHORE_CREATE = 90;
    /// `sys_semaphore_destroy`.
    SEMAPHORE_DESTROY = 91;
    /// `sys_semaphore_wait`.
    SEMAPHORE_WAIT = 92;
    /// `sys_semaphore_trywait`.
    SEMAPHORE_TRY_WAIT = 93;
    /// `sys_semaphore_post`.
    SEMAPHORE_POST = 94;

    /// `sys_lwmutex_create`.
    LWMUTEX_CREATE = 95;
    /// `sys_lwmutex_destroy`.
    LWMUTEX_DESTROY = 96;

    /// `sys_mutex_destroy`.
    MUTEX_DESTROY = 101;
    /// `sys_lwmutex_lock`.
    LWMUTEX_LOCK = 97;
    /// `sys_lwmutex_unlock`.
    LWMUTEX_UNLOCK = 98;
    /// `sys_lwmutex_trylock`.
    LWMUTEX_TRYLOCK = 99;

    /// `sys_mutex_create`.
    MUTEX_CREATE = 100;
    /// `sys_mutex_lock`.
    MUTEX_LOCK = 102;
    /// `sys_mutex_trylock`.
    MUTEX_TRYLOCK = 103;
    /// `sys_mutex_unlock`.
    MUTEX_UNLOCK = 104;

    /// `sys_cond_create`.
    COND_CREATE = 105;
    /// `sys_cond_destroy`.
    COND_DESTROY = 106;
    /// `sys_cond_wait`.
    COND_WAIT = 107;
    /// `sys_cond_signal`.
    COND_SIGNAL = 108;
    /// `sys_cond_signal_all`.
    COND_SIGNAL_ALL = 109;
    /// `sys_cond_signal_to`.
    COND_SIGNAL_TO = 110;

    /// `sys_semaphore_get_value`.
    SEMAPHORE_GET_VALUE = 114;

    /// `sys_event_flag_cancel`.
    EVENT_FLAG_CANCEL = 132;
    /// `sys_event_flag_get`.
    EVENT_FLAG_GET = 139;

    /// `sys_event_flag_clear`.
    EVENT_FLAG_CLEAR = 118;

    /// `sys_event_queue_create`.
    EVENT_QUEUE_CREATE = 128;
    /// `sys_event_queue_destroy`.
    EVENT_QUEUE_DESTROY = 129;
    /// `sys_event_queue_receive`.
    EVENT_QUEUE_RECEIVE = 130;
    /// `sys_event_queue_tryreceive`.
    EVENT_QUEUE_TRY_RECEIVE = 131;
    /// `sys_event_port_send`.
    EVENT_PORT_SEND = 138;

    /// `sys_time_get_timezone`.
    TIME_GET_TIMEZONE = 144;
    /// `sys_time_get_current_time`.
    TIME_GET_CURRENT_TIME = 145;
    /// `sys_time_get_timebase_frequency`.
    TIME_GET_TIMEBASE_FREQUENCY = 147;

    /// `sys_spu_image_open`.
    SPU_IMAGE_OPEN = 156;
    /// `sys_spu_image_import`.
    SPU_IMAGE_IMPORT = 158;
    /// `sys_spu_initialize` -- announce per-process SPU resource
    /// limits (max usable / max raw SPUs). Behavioural oracle:
    /// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_spu.cpp:455`.
    SPU_INITIALIZE = 169;
    /// `sys_spu_thread_group_create`.
    SPU_THREAD_GROUP_CREATE = 170;
    /// `sys_spu_thread_group_destroy` -- destroy a non-running thread
    /// group. Returns CELL_ESRCH on unknown id, CELL_EBUSY when the
    /// group is still running. Behavioural oracle:
    /// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_spu.cpp:1118`.
    SPU_THREAD_GROUP_DESTROY = 171;
    /// `sys_spu_thread_initialize`.
    SPU_THREAD_INITIALIZE = 172;
    /// `sys_spu_thread_group_start`.
    SPU_THREAD_GROUP_START = 173;
    /// `sys_spu_thread_group_terminate`.
    SPU_THREAD_GROUP_TERMINATE = 177;
    /// `sys_spu_thread_group_join`.
    SPU_THREAD_GROUP_JOIN = 178;
    /// `sys_spu_thread_write_ls_mb` family entry point.
    SPU_THREAD_WRITE_MB = 190;

    /// `sys_memory_container_create`.
    MEMORY_CONTAINER_CREATE = 341;
    /// `sys_memory_allocate`.
    MEMORY_ALLOCATE = 348;
    /// `sys_memory_free`.
    MEMORY_FREE = 349;
    /// `sys_memory_get_user_memory_size`.
    MEMORY_GET_USER_MEMORY_SIZE = 352;

    /// `sys_tty_write` (`fd=1` is the TTY guest debug log).
    TTY_WRITE = 403;

    /// `sys_fs_open` (path-validating file-open; minimal handler returns
    /// CELL_ENOENT for unknown paths).
    FS_OPEN = 801;

    /// `sys_fs_read` (read up to `nbytes` from an open fd into a guest
    /// buffer; routed through the in-memory FS layer).
    FS_READ = 802;

    /// `sys_fs_write`.
    FS_WRITE = 803;

    /// `sys_fs_close`.
    FS_CLOSE = 804;

    /// `sys_fs_opendir` (open a directory snapshot for read-only
    /// enumeration; allocates a directory fd whose entries are
    /// captured at open time and served lexicographically).
    FS_OPENDIR = 805;

    /// `sys_fs_readdir` (return the next snapshotted entry as a
    /// 258-byte `CellFsDirent`; writes 0 to `nread` at EOF).
    FS_READDIR = 806;

    /// `sys_fs_closedir` (release a directory fd allocated by
    /// `sys_fs_opendir`).
    FS_CLOSEDIR = 807;

    /// `sys_fs_stat` (populate a `CellFsStat` from a path).
    FS_STAT = 808;

    /// `sys_fs_fstat` (populate a `CellFsStat` for an open fd).
    FS_FSTAT = 809;

    /// `sys_fs_lseek` (move an fd's offset to a new absolute position;
    /// SEEK_SET / SEEK_CUR / SEEK_END semantics).
    FS_LSEEK = 818;

    /// `sys_rsx_memory_allocate`.
    SYS_RSX_MEMORY_ALLOCATE = 668;
    /// `sys_rsx_memory_free`.
    SYS_RSX_MEMORY_FREE = 669;
    /// `sys_rsx_context_allocate`.
    SYS_RSX_CONTEXT_ALLOCATE = 670;
    /// `sys_rsx_context_free`.
    SYS_RSX_CONTEXT_FREE = 671;
    /// `sys_rsx_context_attribute`.
    SYS_RSX_CONTEXT_ATTRIBUTE = 674;

    /// `sys_ss_access_control_engine` -- privileged authority/identity
    /// gate used during user-PRX init to query the caller's SELF
    /// program-authority-id. Behavioral oracle:
    /// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_ss.cpp`.
    SS_ACCESS_CONTROL_ENGINE = 871;
}

/// CellGov-private pseudo-syscall: fired by the unresolved-import
/// trampoline when the guest calls through a GOT slot whose NID
/// has no firmware export. The trampoline loads the NID into r4
/// and the dispatcher emits a structured diagnostic.
///
/// Sits at the start of [`crate::syscall_namespace::SyscallNamespace::UnresolvedImport`]
/// so the namespace classifier routes it without colliding with
/// the LV2 syscall range (0..0x10000). Sits outside the
/// `lv2_syscalls!{}` macro: it is not in the Lv2 namespace and
/// therefore must not appear in [`ALL_LV2_NUMBERS`].
pub const UNRESOLVED_IMPORT: u64 = 0x10000;

// -----------------------------------------------------------------
// Syscall numbers routed via `Lv2Request::Unsupported { number, ..}`
// in `cellgov_lv2::host::dispatch_route`. These have explicit
// handlers but are not first-class `Lv2Request` variants (the
// classifier leaves them in `Unsupported` and the dispatcher
// branches on the number). Naming them here lets the dispatcher
// match against a constant instead of a hand-typed integer literal.
//
// NOT in [`ALL_LV2_NUMBERS`]: that list is the typed-
// arm set. These constants live in the Lv2 namespace too, but the
// classifier produces `Lv2Request::Unsupported` for them rather
// than a typed arm. The `unsupported_routed_syscall_numbers_do_not_collide_with_typed_arms`
// test pins the partition.
// -----------------------------------------------------------------

lv2_syscalls! {
    /// Every LV2 syscall number routed through the
    /// `Lv2Request::Unsupported` arm of the dispatcher. Disjoint
    /// from [`ALL_LV2_NUMBERS`] by the collision-test invariant.
    ALL_LV2_UNSUPPORTED_ROUTED_NUMBERS;
    ALL_LV2_UNSUPPORTED_ROUTED_NAMED;

    /// `sys_prx_load_module` -- behavioral oracle:
    /// `tools/rpcs3-src/rpcs3/Emu/Cell/lv2/sys_prx.cpp`.
    SYS_PRX_LOAD_MODULE = 480;
    /// `sys_prx_load_module_on_memcontainer`.
    SYS_PRX_LOAD_MODULE_ON_MEMCONTAINER = 497;
    /// `sys_prx_start_module`.
    SYS_PRX_START_MODULE = 481;
    /// `sys_tty_read`.
    TTY_READ = 402;
    /// Unnamed syscall 462 (RPCS3 references it as part of the
    /// `sys_storage` / fs management surface).
    UNS_FUNC_462 = 462;
    /// `sys_prx_register_module`.
    SYS_PRX_REGISTER_MODULE = 484;
    /// `sys_prx_register_library`.
    SYS_PRX_REGISTER_LIBRARY = 486;
    /// `sys_ppu_thread_get_priority`.
    PPU_THREAD_GET_PRIORITY = 48;
    /// `sys_prx_get_module_list`.
    SYS_PRX_GET_MODULE_LIST = 494;
    /// `sys_event_port_connect_local`.
    EVENT_PORT_CONNECT_LOCAL = 136;
    /// Gamepad YCON interface (RPCS3 `sys_io.cpp`).
    GAMEPAD_YCON_IF = 621;
    /// HID is-root query (RPCS3 `sys_io.cpp`).
    HID_IS_ROOT = 512;
    /// `sys_rsx_attribute` (671 is `_FREE`; 677 is the attribute setter
    /// variant per RPCS3 `sys_rsx.cpp`).
    RSX_ATTRIBUTE = 677;
    /// `sys_memory_container_create` alternate entry (341 is the main
    /// entry; 324 is an older / authority-gated form).
    MEMORY_CONTAINER_CREATE_324 = 324;
    /// `sys_mmapper_allocate_address`.
    MMAPPER_ALLOCATE_ADDRESS = 330;
    /// `sys_mmapper_map_shared_memory`.
    MMAPPER_MAP_SHARED_MEMORY = 334;
    /// `sys_mmapper_search_and_map`.
    MMAPPER_SEARCH_AND_MAP = 337;
    /// `sys_mmapper_allocate_shared_memory_from_container`.
    MMAPPER_ALLOCATE_SHARED_MEMORY_FROM_CONTAINER = 362;
    /// `sys_mmapper_allocate_shared_memory`.
    MMAPPER_ALLOCATE_SHARED_MEMORY = 332;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn all_lv2_numbers_are_unique() {
        let set: BTreeSet<u64> = ALL_LV2_NUMBERS.iter().copied().collect();
        assert_eq!(
            set.len(),
            ALL_LV2_NUMBERS.len(),
            "ALL_LV2_NUMBERS contains a duplicate; len()={} unique={}",
            ALL_LV2_NUMBERS.len(),
            set.len(),
        );
    }

    /// The unsupported-routed syscall constants must NOT collide
    /// with any number in `ALL_LV2_NUMBERS` (the typed-arm set).
    /// The dispatcher routes typed numbers to typed variants and
    /// unsupported-routed numbers to the `Lv2Request::Unsupported`
    /// arms; a collision means the same number is claimed by both
    /// routing paths and the classifier's behavior depends on arm
    /// ordering rather than identity.
    #[test]
    fn unsupported_routed_syscall_numbers_do_not_collide_with_typed_arms() {
        let typed: BTreeSet<u64> = ALL_LV2_NUMBERS.iter().copied().collect();
        for &(name, n) in ALL_LV2_UNSUPPORTED_ROUTED_NAMED {
            assert!(
                !typed.contains(&n),
                "{name} ({n}) collides with a typed-arm Lv2Request number; \
                 either remove it from ALL_LV2_NUMBERS (if it should route via Unsupported) \
                 or add a typed Lv2Request variant (and remove the Unsupported arm)",
            );
        }
        // Also enforce intra-list uniqueness within the unsupported set.
        let mut seen: BTreeSet<u64> = BTreeSet::new();
        for &(name, n) in ALL_LV2_UNSUPPORTED_ROUTED_NAMED {
            assert!(
                seen.insert(n),
                "{name} duplicates another unsupported-routed syscall number ({n})",
            );
        }
        // Macro emits both arrays from the same source list, so
        // their lengths must agree by construction.
        assert_eq!(
            ALL_LV2_UNSUPPORTED_ROUTED_NAMED.len(),
            ALL_LV2_UNSUPPORTED_ROUTED_NUMBERS.len(),
        );
    }

    /// Named-array values match `ALL_LV2_NUMBERS` exactly.
    #[test]
    fn audit_array_matches_all_lv2_numbers() {
        let audit_set: BTreeSet<u64> = ALL_LV2_NAMED.iter().map(|&(_, v)| v).collect();
        let array_set: BTreeSet<u64> = ALL_LV2_NUMBERS.iter().copied().collect();
        let missing_from_array: Vec<&(&str, u64)> = ALL_LV2_NAMED
            .iter()
            .filter(|(_, v)| !array_set.contains(v))
            .collect();
        let missing_from_audit: Vec<u64> = ALL_LV2_NUMBERS
            .iter()
            .copied()
            .filter(|v| !audit_set.contains(v))
            .collect();
        assert!(
            missing_from_array.is_empty(),
            "constants in ALL_LV2_NAMED missing from ALL_LV2_NUMBERS: {missing_from_array:?}",
        );
        assert!(
            missing_from_audit.is_empty(),
            "values in ALL_LV2_NUMBERS missing from ALL_LV2_NAMED: {missing_from_audit:?}",
        );
        assert_eq!(ALL_LV2_NAMED.len(), ALL_LV2_NUMBERS.len());
    }
}
