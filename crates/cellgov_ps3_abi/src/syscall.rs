//! LV2 syscall numbers (the value the guest puts in r11 before `sc`).
//!
//! Names mirror RPCS3's `syscall_table_t` entries. Behaviour
//! (the dispatch match in `cellgov_lv2::request::classify`) lives in
//! `cellgov_lv2`; this module is data only.

/// `sys_process_getpid`.
pub const PROCESS_GETPID: u64 = 1;

/// `sys_process_get_number_of_object`.
pub const PROCESS_GET_NUMBER_OF_OBJECT: u64 = 12;

/// `sys_process_getppid`.
pub const PROCESS_GETPPID: u64 = 18;

/// `sys_process_exit`.
pub const PROCESS_EXIT: u64 = 22;

/// `sys_process_get_sdk_version`.
pub const PROCESS_GET_SDK_VERSION: u64 = 25;

/// `_sys_process_get_paramsfo`.
pub const PROCESS_GET_PARAMSFO: u64 = 30;

/// `sys_process_get_ppu_guid`.
pub const PROCESS_GET_PPU_GUID: u64 = 31;

/// `sys_timer_create`.
pub const TIMER_CREATE: u64 = 70;
/// `sys_timer_destroy`.
pub const TIMER_DESTROY: u64 = 71;

/// `sys_rwlock_create`.
pub const RWLOCK_CREATE: u64 = 120;
/// `sys_rwlock_destroy`.
pub const RWLOCK_DESTROY: u64 = 121;

/// `sys_event_port_create`.
pub const EVENT_PORT_CREATE: u64 = 134;
/// `sys_event_port_destroy`.
pub const EVENT_PORT_DESTROY: u64 = 135;

/// `sys_ppu_thread_exit`.
pub const PPU_THREAD_EXIT: u64 = 41;
/// `sys_ppu_thread_yield`.
pub const PPU_THREAD_YIELD: u64 = 43;
/// `sys_ppu_thread_join`.
pub const PPU_THREAD_JOIN: u64 = 44;
/// `_sys_ppu_thread_create` (LV2-side; sysPrxForUser wraps it).
pub const PPU_THREAD_CREATE: u64 = 52;

/// `sys_event_flag_create`.
pub const EVENT_FLAG_CREATE: u64 = 82;
/// `sys_event_flag_destroy`.
pub const EVENT_FLAG_DESTROY: u64 = 83;
/// `sys_event_flag_wait`.
pub const EVENT_FLAG_WAIT: u64 = 85;
/// `sys_event_flag_trywait`.
pub const EVENT_FLAG_TRY_WAIT: u64 = 86;
/// `sys_event_flag_set`.
pub const EVENT_FLAG_SET: u64 = 87;

/// `sys_semaphore_create`.
pub const SEMAPHORE_CREATE: u64 = 90;
/// `sys_semaphore_destroy`.
pub const SEMAPHORE_DESTROY: u64 = 91;
/// `sys_semaphore_wait`.
pub const SEMAPHORE_WAIT: u64 = 92;
/// `sys_semaphore_trywait`.
pub const SEMAPHORE_TRY_WAIT: u64 = 93;
/// `sys_semaphore_post`.
pub const SEMAPHORE_POST: u64 = 94;

/// `sys_lwmutex_create`.
pub const LWMUTEX_CREATE: u64 = 95;
/// `sys_lwmutex_destroy`.
pub const LWMUTEX_DESTROY: u64 = 96;

/// `sys_mutex_destroy`.
pub const MUTEX_DESTROY: u64 = 101;
/// `sys_lwmutex_lock`.
pub const LWMUTEX_LOCK: u64 = 97;
/// `sys_lwmutex_unlock`.
pub const LWMUTEX_UNLOCK: u64 = 98;
/// `sys_lwmutex_trylock`.
pub const LWMUTEX_TRYLOCK: u64 = 99;

/// `sys_mutex_create`.
pub const MUTEX_CREATE: u64 = 100;
/// `sys_mutex_lock`.
pub const MUTEX_LOCK: u64 = 102;
/// `sys_mutex_trylock`.
pub const MUTEX_TRYLOCK: u64 = 103;
/// `sys_mutex_unlock`.
pub const MUTEX_UNLOCK: u64 = 104;

/// `sys_cond_create`.
pub const COND_CREATE: u64 = 105;
/// `sys_cond_destroy`.
pub const COND_DESTROY: u64 = 106;
/// `sys_cond_wait`.
pub const COND_WAIT: u64 = 107;
/// `sys_cond_signal`.
pub const COND_SIGNAL: u64 = 108;
/// `sys_cond_signal_all`.
pub const COND_SIGNAL_ALL: u64 = 109;
/// `sys_cond_signal_to`.
pub const COND_SIGNAL_TO: u64 = 110;

/// `sys_semaphore_get_value`.
pub const SEMAPHORE_GET_VALUE: u64 = 114;

/// `sys_event_flag_cancel`.
pub const EVENT_FLAG_CANCEL: u64 = 132;
/// `sys_event_flag_get`.
pub const EVENT_FLAG_GET: u64 = 139;

/// `sys_event_flag_clear`.
pub const EVENT_FLAG_CLEAR: u64 = 118;

/// `sys_event_queue_create`.
pub const EVENT_QUEUE_CREATE: u64 = 128;
/// `sys_event_queue_destroy`.
pub const EVENT_QUEUE_DESTROY: u64 = 129;
/// `sys_event_queue_receive`.
pub const EVENT_QUEUE_RECEIVE: u64 = 130;
/// `sys_event_queue_tryreceive`.
pub const EVENT_QUEUE_TRY_RECEIVE: u64 = 131;
/// `sys_event_port_send`.
pub const EVENT_PORT_SEND: u64 = 138;

/// `sys_time_get_timezone`.
pub const TIME_GET_TIMEZONE: u64 = 144;
/// `sys_time_get_current_time`.
pub const TIME_GET_CURRENT_TIME: u64 = 145;
/// `sys_time_get_timebase_frequency`.
pub const TIME_GET_TIMEBASE_FREQUENCY: u64 = 147;

/// `sys_spu_image_open`.
pub const SPU_IMAGE_OPEN: u64 = 156;
/// `sys_spu_thread_group_create`.
pub const SPU_THREAD_GROUP_CREATE: u64 = 170;
/// `sys_spu_thread_initialize`.
pub const SPU_THREAD_INITIALIZE: u64 = 172;
/// `sys_spu_thread_group_start`.
pub const SPU_THREAD_GROUP_START: u64 = 173;
/// `sys_spu_thread_group_terminate`.
pub const SPU_THREAD_GROUP_TERMINATE: u64 = 177;
/// `sys_spu_thread_group_join`.
pub const SPU_THREAD_GROUP_JOIN: u64 = 178;
/// `sys_spu_thread_write_ls_mb` family entry point.
pub const SPU_THREAD_WRITE_MB: u64 = 190;

/// `sys_memory_container_create`.
pub const MEMORY_CONTAINER_CREATE: u64 = 341;
/// `sys_memory_allocate`.
pub const MEMORY_ALLOCATE: u64 = 348;
/// `sys_memory_free`.
pub const MEMORY_FREE: u64 = 349;
/// `sys_memory_get_user_memory_size`.
pub const MEMORY_GET_USER_MEMORY_SIZE: u64 = 352;

/// `sys_tty_write` (`fd=1` is the TTY guest debug log).
pub const TTY_WRITE: u64 = 403;

/// `sys_fs_open` (path-validating file-open; minimal handler returns
/// CELL_ENOENT for unknown paths).
pub const FS_OPEN: u64 = 801;

/// `sys_fs_read` (read up to `nbytes` from an open fd into a guest
/// buffer; routed through the in-memory FS layer).
pub const FS_READ: u64 = 802;

/// `sys_fs_write`.
pub const FS_WRITE: u64 = 803;

/// `sys_fs_close`.
pub const FS_CLOSE: u64 = 804;

/// `sys_fs_fstat` (populate a `CellFsStat` for an open fd).
pub const FS_FSTAT: u64 = 805;

/// `sys_fs_stat` (populate a `CellFsStat` from a path).
pub const FS_STAT: u64 = 815;

/// `sys_fs_lseek` (move an fd's offset to a new absolute position;
/// SEEK_SET / SEEK_CUR / SEEK_END semantics).
pub const FS_LSEEK: u64 = 818;

/// `sys_rsx_memory_allocate`.
pub const SYS_RSX_MEMORY_ALLOCATE: u64 = 668;
/// `sys_rsx_memory_free`.
pub const SYS_RSX_MEMORY_FREE: u64 = 669;
/// `sys_rsx_context_allocate`.
pub const SYS_RSX_CONTEXT_ALLOCATE: u64 = 670;
/// `sys_rsx_context_free`.
pub const SYS_RSX_CONTEXT_FREE: u64 = 671;
/// `sys_rsx_context_attribute`.
pub const SYS_RSX_CONTEXT_ATTRIBUTE: u64 = 674;
