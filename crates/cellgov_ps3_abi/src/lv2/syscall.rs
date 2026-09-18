//! LV2 syscall numbers (the value the guest puts in r11 before `sc`).
//!
//! Each constant is named after the syscall it selects, or, where the
//! syscall has no known name, after its number. Behaviour
//! (the dispatch match in `cellgov_lv2::request::classify`) lives in
//! `cellgov_lv2`; this module is data only. The `lv2_syscalls!` macro
//! emits every `pub const` syscall number, and the LV2 name CellGov
//! gives each one, from a single declarative source.
//!
//! The number is extracted: the kernel's dispatch table fixes it. The
//! name is attributed: nothing in a firmware image carries it, so the
//! name field is CellGov's own vocabulary. The LV2 archive's `name.tsv`
//! records it under the `cellgov` source beside what the other
//! committed sources say.

/// One entry of an `lv2_syscalls!` list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv2Syscall {
    /// The number the guest puts in r11.
    pub number: u64,
    /// The `pub const` identifier.
    pub constant: &'static str,
    /// The LV2 name CellGov uses; `None` where no source names the
    /// number.
    pub name: Option<&'static str>,
}

macro_rules! lv2_name {
    () => {
        None
    };
    ($name:literal) => {
        Some($name)
    };
}

/// Emit a group of LV2 syscall `pub const`s plus a derived number
/// array and an [`Lv2Syscall`] array, from one list.
///
/// ```text
/// lv2_syscalls! {
///     $(#[doc = "..."])* NUMBERS_ARRAY;
///     $(#[doc = "..."])* SYSCALLS_ARRAY;
///     $( /// docs. NAME = number => "lv2_name"; )*
/// }
/// ```
///
/// The `=> "lv2_name"` field is optional; an entry without one has
/// `name: None` in the syscalls array.
macro_rules! lv2_syscalls {
    (
        $(#[$arr_attr:meta])*
        $arr:ident;
        $(#[$syscalls_attr:meta])*
        $syscalls:ident;
        $( $(#[$attr:meta])* $name:ident = $value:expr $(=> $lv2:literal)?; )*
    ) => {
        $(
            $(#[$attr])*
            pub const $name: u64 = $value;
        )*

        $(#[$arr_attr])*
        pub const $arr: &[u64] = &[ $( $name ),* ];

        $(#[$syscalls_attr])*
        pub const $syscalls: &[Lv2Syscall] = &[
            $( Lv2Syscall {
                number: $name,
                constant: stringify!($name),
                name: lv2_name!($($lv2)?),
            } ),*
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
    /// list and [`ALL_LV2_SYSCALLS`] from the same source; the
    /// `all_lv2_numbers_are_unique` test pins the uniqueness half.
    /// The `unsupported_routed_syscall_numbers_do_not_collide_with_typed_arms`
    /// test pins disjointness against the unsupported-routed set.
    ALL_LV2_NUMBERS;
    /// The typed-arm entries with their constants and LV2 names, in
    /// the order of [`ALL_LV2_NUMBERS`].
    ALL_LV2_SYSCALLS;

    /// `sys_process_getpid`.
    PROCESS_GETPID = 1 => "sys_process_getpid";

    /// `sys_process_get_status`; the status is the return value.
    PROCESS_GET_STATUS = 4 => "sys_process_get_status";

    /// `sys_process_get_number_of_object`.
    PROCESS_GET_NUMBER_OF_OBJECT = 12 => "sys_process_get_number_of_object";

    /// `sys_process_is_spu_lock_line_reservation_address` -- ask
    /// whether an SPU thread or Raw SPU may wait for the lock-line
    /// reservation lost event at `addr`.
    // [CBE-Handbook p:479 s:18.6.4] The lock-line reservation lost event fires
    // when an outside entity modifies the 128-byte line an SPU reserved with
    // getllar, and not for a reservation the SPE itself resets.
    PROCESS_IS_SPU_LOCK_LINE_RESERVATION_ADDRESS = 14 => "sys_process_is_spu_lock_line_reservation_address";

    /// `sys_process_getppid`.
    PROCESS_GETPPID = 18 => "sys_process_getppid";

    /// `_sys_process_spawn` -- legacy spawn, same marshalled-block
    /// family as `sys_process_spawns_a_self2`. Not a shorter call:
    /// the two values sc 27 stores into the pair block behind its
    /// r10 go directly in r8/r9 here, and there is no r10 slot (vsh
    /// private wrapper 0x608be8, sc at 0x608cc0). The audio-fallback
    /// site at 0xcb484 spawns via the public entry 0x608fd4, which
    /// zeroes both extras.
    PROCESS_SPAWN = 21 => "_sys_process_spawn";

    /// `sys_process_exit`.
    PROCESS_EXIT = 22 => "sys_process_exit";

    /// `sys_process_get_sdk_version`.
    PROCESS_GET_SDK_VERSION = 25 => "sys_process_get_sdk_version";

    /// `_sys_process_exit2` -- exit carrying an argv/envp block;
    /// a non-empty argv makes it an exitspawn instead of a plain exit.
    PROCESS_EXIT2 = 26 => "_sys_process_exit2";

    /// `sys_process_spawns_a_self2` -- spawn a SELF as a child
    /// process. Decoded actual signature (vsh 0x608a8c): (pid_out,
    /// prio, flags, marshalled path/argv/envp block ptr, block size,
    /// data word, 64B cfg block, dbg pair ptr).
    PROCESS_SPAWNS_A_SELF2 = 27 => "sys_process_spawns_a_self2";

    /// `_sys_process_get_paramsfo`.
    PROCESS_GET_PARAMSFO = 30 => "_sys_process_get_paramsfo";

    /// `sys_process_get_ppu_guid`.
    PROCESS_GET_PPU_GUID = 31 => "sys_process_get_ppu_guid";

    /// `sys_timer_create`.
    TIMER_CREATE = 70 => "sys_timer_create";
    /// `sys_timer_destroy`.
    TIMER_DESTROY = 71 => "sys_timer_destroy";

    /// `sys_timer_usleep`.
    TIMER_USLEEP = 141 => "sys_timer_usleep";
    /// `sys_timer_sleep`.
    TIMER_SLEEP = 142 => "sys_timer_sleep";

    /// `sys_rwlock_create`.
    RWLOCK_CREATE = 120 => "sys_rwlock_create";
    /// `sys_rwlock_destroy`.
    RWLOCK_DESTROY = 121 => "sys_rwlock_destroy";

    /// `sys_event_port_create`.
    EVENT_PORT_CREATE = 134 => "sys_event_port_create";
    /// `sys_event_port_destroy`.
    EVENT_PORT_DESTROY = 135 => "sys_event_port_destroy";

    /// `sys_ppu_thread_exit`.
    PPU_THREAD_EXIT = 41 => "sys_ppu_thread_exit";
    /// `sys_ppu_thread_yield`.
    PPU_THREAD_YIELD = 43 => "sys_ppu_thread_yield";
    /// `sys_ppu_thread_join`.
    PPU_THREAD_JOIN = 44 => "sys_ppu_thread_join";
    /// `_sys_ppu_thread_create` (LV2-side; sysPrxForUser wraps it).
    PPU_THREAD_CREATE = 52 => "_sys_ppu_thread_create";
    /// `sys_ppu_thread_start`.
    PPU_THREAD_START = 53 => "sys_ppu_thread_start";

    /// `sys_event_flag_create`.
    EVENT_FLAG_CREATE = 82 => "sys_event_flag_create";
    /// `sys_event_flag_destroy`.
    EVENT_FLAG_DESTROY = 83 => "sys_event_flag_destroy";
    /// `sys_event_flag_wait`.
    EVENT_FLAG_WAIT = 85 => "sys_event_flag_wait";
    /// `sys_event_flag_trywait`.
    EVENT_FLAG_TRY_WAIT = 86 => "sys_event_flag_trywait";
    /// `sys_event_flag_set`.
    EVENT_FLAG_SET = 87 => "sys_event_flag_set";

    /// `sys_semaphore_create`.
    SEMAPHORE_CREATE = 90 => "sys_semaphore_create";
    /// `sys_semaphore_destroy`.
    SEMAPHORE_DESTROY = 91 => "sys_semaphore_destroy";
    /// `sys_semaphore_wait`.
    SEMAPHORE_WAIT = 92 => "sys_semaphore_wait";
    /// `sys_semaphore_trywait`.
    SEMAPHORE_TRY_WAIT = 93 => "sys_semaphore_trywait";
    /// `sys_semaphore_post`.
    SEMAPHORE_POST = 94 => "sys_semaphore_post";

    /// `sys_lwmutex_create`.
    LWMUTEX_CREATE = 95 => "sys_lwmutex_create";
    /// `sys_lwmutex_destroy`.
    LWMUTEX_DESTROY = 96 => "sys_lwmutex_destroy";

    /// `sys_mutex_destroy`.
    MUTEX_DESTROY = 101 => "sys_mutex_destroy";
    /// `sys_lwmutex_lock`.
    LWMUTEX_LOCK = 97 => "sys_lwmutex_lock";
    /// `sys_lwmutex_unlock`.
    LWMUTEX_UNLOCK = 98 => "sys_lwmutex_unlock";
    /// `sys_lwmutex_trylock`.
    LWMUTEX_TRYLOCK = 99 => "sys_lwmutex_trylock";

    /// `sys_mutex_create`.
    MUTEX_CREATE = 100 => "sys_mutex_create";
    /// `sys_mutex_lock`.
    MUTEX_LOCK = 102 => "sys_mutex_lock";
    /// `sys_mutex_trylock`.
    MUTEX_TRYLOCK = 103 => "sys_mutex_trylock";
    /// `sys_mutex_unlock`.
    MUTEX_UNLOCK = 104 => "sys_mutex_unlock";

    /// `sys_cond_create`.
    COND_CREATE = 105 => "sys_cond_create";
    /// `sys_cond_destroy`.
    COND_DESTROY = 106 => "sys_cond_destroy";
    /// `sys_cond_wait`.
    COND_WAIT = 107 => "sys_cond_wait";
    /// `sys_cond_signal`.
    COND_SIGNAL = 108 => "sys_cond_signal";
    /// `sys_cond_signal_all`.
    COND_SIGNAL_ALL = 109 => "sys_cond_signal_all";
    /// `sys_cond_signal_to`.
    COND_SIGNAL_TO = 110 => "sys_cond_signal_to";

    /// `sys_semaphore_get_value`.
    SEMAPHORE_GET_VALUE = 114 => "sys_semaphore_get_value";

    /// `sys_event_flag_cancel`.
    EVENT_FLAG_CANCEL = 132 => "sys_event_flag_cancel";
    /// `sys_event_flag_get`.
    EVENT_FLAG_GET = 139 => "sys_event_flag_get";

    /// `sys_event_flag_clear`.
    EVENT_FLAG_CLEAR = 118 => "sys_event_flag_clear";

    /// `sys_event_queue_create`.
    EVENT_QUEUE_CREATE = 128 => "sys_event_queue_create";
    /// `sys_event_queue_destroy`.
    EVENT_QUEUE_DESTROY = 129 => "sys_event_queue_destroy";
    /// `sys_event_queue_receive`.
    EVENT_QUEUE_RECEIVE = 130 => "sys_event_queue_receive";
    /// `sys_event_queue_tryreceive`.
    EVENT_QUEUE_TRY_RECEIVE = 131 => "sys_event_queue_tryreceive";
    /// `sys_event_port_send`.
    EVENT_PORT_SEND = 138 => "sys_event_port_send";

    /// `sys_time_get_timezone`.
    TIME_GET_TIMEZONE = 144 => "sys_time_get_timezone";
    /// `sys_time_get_current_time`.
    TIME_GET_CURRENT_TIME = 145 => "sys_time_get_current_time";
    /// `sys_time_get_timebase_frequency`.
    TIME_GET_TIMEBASE_FREQUENCY = 147 => "sys_time_get_timebase_frequency";

    /// `sys_spu_image_open`.
    SPU_IMAGE_OPEN = 156 => "sys_spu_image_open";
    /// `sys_spu_image_import`.
    SPU_IMAGE_IMPORT = 158 => "sys_spu_image_import";
    /// `sys_spu_initialize` -- announce per-process SPU resource
    /// limits: how many physical SPUs the process may use, and how
    /// many of those may be handed out as Raw SPUs.
    SPU_INITIALIZE = 169 => "sys_spu_initialize";
    /// `sys_spu_thread_group_create`.
    SPU_THREAD_GROUP_CREATE = 170 => "sys_spu_thread_group_create";
    /// `sys_spu_thread_group_destroy` -- destroy a non-running thread
    /// group. Returns CELL_ESRCH when the id names no group, and
    /// CELL_EBUSY while the group is operational or in use by another
    /// syscall.
    SPU_THREAD_GROUP_DESTROY = 171 => "sys_spu_thread_group_destroy";
    /// `sys_spu_thread_initialize`.
    SPU_THREAD_INITIALIZE = 172 => "sys_spu_thread_initialize";
    /// `sys_spu_thread_group_start`.
    SPU_THREAD_GROUP_START = 173 => "sys_spu_thread_group_start";
    /// `sys_spu_thread_group_terminate`.
    SPU_THREAD_GROUP_TERMINATE = 177 => "sys_spu_thread_group_terminate";
    /// `sys_spu_thread_group_join`.
    SPU_THREAD_GROUP_JOIN = 178 => "sys_spu_thread_group_join";
    /// `sys_spu_thread_write_ls_mb` family entry point.
    SPU_THREAD_WRITE_MB = 190 => "sys_spu_thread_write_ls_mb";

    /// `sys_memory_container_create`.
    MEMORY_CONTAINER_CREATE = 341 => "sys_memory_container_create";
    /// `sys_memory_allocate`.
    MEMORY_ALLOCATE = 348 => "sys_memory_allocate";
    /// `sys_memory_free`.
    MEMORY_FREE = 349 => "sys_memory_free";
    /// `sys_memory_allocate_from_container`.
    MEMORY_ALLOCATE_FROM_CONTAINER = 350 => "sys_memory_allocate_from_container";
    /// `sys_memory_get_user_memory_size`.
    MEMORY_GET_USER_MEMORY_SIZE = 352 => "sys_memory_get_user_memory_size";

    /// `sys_tty_write` (`fd=1` is the TTY guest debug log).
    TTY_WRITE = 403 => "sys_tty_write";

    /// `sys_fs_open` (path-validating file-open; minimal handler returns
    /// CELL_ENOENT for unknown paths).
    FS_OPEN = 801 => "sys_fs_open";

    /// `sys_fs_read` (read up to `nbytes` from an open fd into a guest
    /// buffer; routed through the in-memory FS layer).
    FS_READ = 802 => "sys_fs_read";

    /// `sys_fs_write`.
    FS_WRITE = 803 => "sys_fs_write";

    /// `sys_fs_close`.
    FS_CLOSE = 804 => "sys_fs_close";

    /// `sys_fs_opendir` (open a directory snapshot for read-only
    /// enumeration; allocates a directory fd whose entries are
    /// captured at open time and served lexicographically).
    FS_OPENDIR = 805 => "sys_fs_opendir";

    /// `sys_fs_readdir` (return the next snapshotted entry as a
    /// 258-byte `CellFsDirent`; writes 0 to `nread` at EOF).
    FS_READDIR = 806 => "sys_fs_readdir";

    /// `sys_fs_closedir` (release a directory fd allocated by
    /// `sys_fs_opendir`).
    FS_CLOSEDIR = 807 => "sys_fs_closedir";

    /// `sys_fs_stat` (populate a `CellFsStat` from a path).
    FS_STAT = 808 => "sys_fs_stat";

    /// `sys_fs_fstat` (populate a `CellFsStat` for an open fd).
    FS_FSTAT = 809 => "sys_fs_fstat";

    /// `sys_fs_lseek` (move an fd's offset to a new absolute position;
    /// SEEK_SET / SEEK_CUR / SEEK_END semantics).
    FS_LSEEK = 818 => "sys_fs_lseek";

    /// `sys_rsx_memory_allocate`.
    SYS_RSX_MEMORY_ALLOCATE = 668 => "sys_rsx_memory_allocate";
    /// `sys_rsx_memory_free`.
    SYS_RSX_MEMORY_FREE = 669 => "sys_rsx_memory_free";
    /// `sys_rsx_context_allocate`.
    SYS_RSX_CONTEXT_ALLOCATE = 670 => "sys_rsx_context_allocate";
    /// `sys_rsx_context_free`.
    SYS_RSX_CONTEXT_FREE = 671 => "sys_rsx_context_free";
    /// `sys_rsx_context_iomap`.
    SYS_RSX_CONTEXT_IOMAP = 672 => "sys_rsx_context_iomap";
    /// `sys_rsx_context_attribute`.
    SYS_RSX_CONTEXT_ATTRIBUTE = 674 => "sys_rsx_context_attribute";
    /// `sys_rsx_device_map`.
    SYS_RSX_DEVICE_MAP = 675 => "sys_rsx_device_map";

    /// `sys_uart_initialize`.
    UART_INITIALIZE = 367 => "sys_uart_initialize";
    /// `sys_uart_receive`.
    UART_RECEIVE = 368 => "sys_uart_receive";
    /// `sys_uart_send`.
    UART_SEND = 369 => "sys_uart_send";
    /// `sys_uart_get_params`.
    UART_GET_PARAMS = 370 => "sys_uart_get_params";

    /// `sys_config_open`.
    CONFIG_OPEN = 516 => "sys_config_open";
    /// `sys_config_close`.
    CONFIG_CLOSE = 517 => "sys_config_close";
    /// `sys_config_get_service_event`.
    CONFIG_GET_SERVICE_EVENT = 518 => "sys_config_get_service_event";
    /// `sys_config_add_service_listener`.
    CONFIG_ADD_SERVICE_LISTENER = 519 => "sys_config_add_service_listener";
    /// `sys_config_remove_service_listener`.
    CONFIG_REMOVE_SERVICE_LISTENER = 520 => "sys_config_remove_service_listener";
    /// `sys_config_register_service`.
    CONFIG_REGISTER_SERVICE = 521 => "sys_config_register_service";
    /// `sys_config_unregister_service`.
    CONFIG_UNREGISTER_SERVICE = 522 => "sys_config_unregister_service";

    /// `sys_usbd_initialize`.
    USBD_INITIALIZE = 530 => "sys_usbd_initialize";
    /// `sys_usbd_finalize`.
    USBD_FINALIZE = 531 => "sys_usbd_finalize";
    /// `sys_usbd_get_device_list`.
    USBD_GET_DEVICE_LIST = 532 => "sys_usbd_get_device_list";
    /// `sys_usbd_get_descriptor_size`.
    USBD_GET_DESCRIPTOR_SIZE = 533 => "sys_usbd_get_descriptor_size";
    /// `sys_usbd_get_descriptor`.
    USBD_GET_DESCRIPTOR = 534 => "sys_usbd_get_descriptor";
    /// `sys_usbd_register_ldd`.
    USBD_REGISTER_LDD = 535 => "sys_usbd_register_ldd";
    /// `sys_usbd_unregister_ldd`.
    USBD_UNREGISTER_LDD = 536 => "sys_usbd_unregister_ldd";
    /// `sys_usbd_open_pipe`.
    USBD_OPEN_PIPE = 537 => "sys_usbd_open_pipe";
    /// `sys_usbd_open_default_pipe`.
    USBD_OPEN_DEFAULT_PIPE = 538 => "sys_usbd_open_default_pipe";
    /// `sys_usbd_close_pipe`.
    USBD_CLOSE_PIPE = 539 => "sys_usbd_close_pipe";
    /// `sys_usbd_receive_event`.
    USBD_RECEIVE_EVENT = 540 => "sys_usbd_receive_event";
    /// `sys_usbd_detect_event`.
    USBD_DETECT_EVENT = 541 => "sys_usbd_detect_event";

    /// Privileged authority/identity gate in the `sys_ss` block, just
    /// below the open-PSID and product-mode queries.
    ///
    /// `r3` carries a `pkg_id` that selects the subcommand; `pkg_id` 2
    /// yields the calling process's SELF program-authority-id. Every
    /// call site in the installed firmware image loads 1, 2 or 3, so
    /// the kernel's answer to any other `pkg_id` is unestablished. The
    /// name is inherited vocabulary; no first-party list carries it.
    SS_ACCESS_CONTROL_ENGINE = 871 => "sys_ss_access_control_engine";
}

/// Slots in the LV2 syscall dispatch table. `sc` selects one by the
/// value in r11. A number at or past this count selects none.
pub const SYSCALL_TABLE_SLOTS: u64 = 1024;

/// CellGov-private pseudo-syscall: fired by the unresolved-import
/// trampoline when the guest calls through a GOT slot whose NID
/// has no firmware export. The trampoline loads the NID into r4
/// and the dispatcher emits a structured diagnostic.
///
/// This private number starts
/// [`crate::lv2::namespace::SyscallNamespace::UnresolvedImport`] and
/// stays outside [`ALL_LV2_NUMBERS`].
pub const UNRESOLVED_IMPORT: u64 = 0x10000;

// -----------------------------------------------------------------
// Syscall numbers routed via `Lv2Request::Unsupported { number, ..}`
// in `cellgov_lv2::host::dispatch_route`: these have explicit
// handlers but are not first-class `Lv2Request` variants (the
// classifier leaves them in `Unsupported` and the dispatcher
// branches on the number).
// -----------------------------------------------------------------

lv2_syscalls! {
    /// Every LV2 syscall number routed through the
    /// `Lv2Request::Unsupported` arm of the dispatcher. Disjoint
    /// from [`ALL_LV2_NUMBERS`] by the collision-test invariant.
    ALL_LV2_UNSUPPORTED_ROUTED_NUMBERS;
    /// The routed entries with their constants and LV2 names, in the
    /// order of [`ALL_LV2_UNSUPPORTED_ROUTED_NUMBERS`].
    ALL_LV2_UNSUPPORTED_ROUTED_SYSCALLS;

    /// `_sys_prx_load_module`.
    SYS_PRX_LOAD_MODULE = 480 => "_sys_prx_load_module";
    /// `_sys_prx_load_module_on_memcontainer`.
    SYS_PRX_LOAD_MODULE_ON_MEMCONTAINER = 497 => "_sys_prx_load_module_on_memcontainer";
    /// `_sys_prx_start_module`.
    SYS_PRX_START_MODULE = 481 => "_sys_prx_start_module";
    /// `_sys_prx_stop_module`.
    SYS_PRX_STOP_MODULE = 482 => "_sys_prx_stop_module";
    /// `_sys_prx_unload_module`.
    SYS_PRX_UNLOAD_MODULE = 483 => "_sys_prx_unload_module";
    /// `sys_tty_read`.
    TTY_READ = 402 => "sys_tty_read";
    /// Unnamed syscall 462. The number sits in the `sys_prx` block,
    /// just above the module-id-by-address query at 461. What it does
    /// is unestablished. No committed source names it, so the constant
    /// carries no LV2 name.
    UNS_FUNC_462 = 462;
    /// `_sys_prx_register_module`.
    SYS_PRX_REGISTER_MODULE = 484 => "_sys_prx_register_module";
    /// `_sys_prx_register_library`.
    SYS_PRX_REGISTER_LIBRARY = 486 => "_sys_prx_register_library";
    /// `sys_ppu_thread_set_priority`.
    PPU_THREAD_SET_PRIORITY = 47 => "sys_ppu_thread_set_priority";
    /// `sys_ppu_thread_get_priority`.
    PPU_THREAD_GET_PRIORITY = 48 => "sys_ppu_thread_get_priority";
    /// `_sys_prx_get_module_list`.
    SYS_PRX_GET_MODULE_LIST = 494 => "_sys_prx_get_module_list";
    /// `sys_event_port_connect_local`.
    EVENT_PORT_CONNECT_LOCAL = 136 => "sys_event_port_connect_local";
    /// `sys_event_port_disconnect`.
    EVENT_PORT_DISCONNECT = 137 => "sys_event_port_disconnect";
    /// `sys_event_port_connect_ipc`.
    EVENT_PORT_CONNECT_IPC = 140 => "sys_event_port_connect_ipc";
    /// Gamepad YCON interface. The number sits just below the sys_io
    /// buffer block that starts at 624; the name is inherited
    /// vocabulary, not a Sony one.
    GAMEPAD_YCON_IF = 621 => "sys_gamepad_ycon_if";
    /// HID is-root query. The number's owner is unestablished, and the
    /// name is uncorroborated: inherited vocabulary the archive reports
    /// under the `cellgov` source alone.
    HID_IS_ROOT = 512 => "sys_hid_manager_is_process_permission_root";
    /// `sys_rsx_attribute`, the last entry of the sys_rsx block. The
    /// per-context setter is `sys_rsx_context_attribute` at 674.
    RSX_ATTRIBUTE = 677 => "sys_rsx_attribute";
    /// `sys_memory_container_create` alternate entry (341 is the main
    /// entry; 324 is an older / authority-gated form).
    MEMORY_CONTAINER_CREATE_324 = 324 => "sys_memory_container_create";
    /// `sys_mmapper_allocate_address`.
    MMAPPER_ALLOCATE_ADDRESS = 330 => "sys_mmapper_allocate_address";
    /// `sys_mmapper_map_shared_memory`.
    MMAPPER_MAP_SHARED_MEMORY = 334 => "sys_mmapper_map_shared_memory";
    /// `sys_mmapper_search_and_map`.
    MMAPPER_SEARCH_AND_MAP = 337 => "sys_mmapper_search_and_map";
    /// `sys_mmapper_allocate_shared_memory_from_container`.
    MMAPPER_ALLOCATE_SHARED_MEMORY_FROM_CONTAINER = 362 => "sys_mmapper_allocate_shared_memory_from_container";
    /// `sys_mmapper_allocate_shared_memory_ext`.
    MMAPPER_ALLOCATE_SHARED_MEMORY_EXT = 339 => "sys_mmapper_allocate_shared_memory_ext";
    /// `sys_mmapper_allocate_shared_memory`.
    MMAPPER_ALLOCATE_SHARED_MEMORY = 332 => "sys_mmapper_allocate_shared_memory";
}

#[cfg(test)]
#[path = "tests/syscall_tests.rs"]
mod tests;
