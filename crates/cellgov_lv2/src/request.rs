//! Typed LV2 syscall requests decoded from PPU `sc` GPR state.
//!
//! Host handlers match exhaustively on [`Lv2Request`]; [`classify`]
//! is total so unknown numbers and malformed arguments surface as
//! [`Lv2Request::Unsupported`] / [`Lv2Request::Malformed`] instead
//! of panicking.

use cellgov_ps3_abi::syscall;

/// Typed LV2 syscall request; host handlers exhaustively match.
///
/// Pointer fields are guest effective addresses (u32 on PS3 despite
/// the 64-bit ELF container). [`classify`] rejects a u32-typed field
/// whose source GPR has non-zero high bits rather than truncating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2Request {
    /// sys_spu_image_open (156).
    SpuImageOpen {
        /// Out-pointer for `sys_spu_image_t`.
        img_ptr: u32,
        /// NUL-terminated path string.
        path_ptr: u32,
    },
    /// sys_spu_thread_group_create (170).
    SpuThreadGroupCreate {
        /// Out-pointer for the group id.
        id_ptr: u32,
        /// Number of SPU threads in the group.
        num_threads: u32,
        /// Priority (not consulted by the scheduler).
        priority: u32,
        /// Attribute struct (opaque).
        attr_ptr: u32,
    },
    /// sys_spu_thread_initialize (172).
    SpuThreadInitialize {
        /// Out-pointer for the thread id.
        thread_ptr: u32,
        /// Thread group id.
        group_id: u32,
        /// Slot index within the group (0-based).
        thread_num: u32,
        /// `sys_spu_image_t` struct.
        img_ptr: u32,
        /// Attribute struct (opaque).
        attr_ptr: u32,
        /// `sys_spu_thread_argument` struct.
        arg_ptr: u32,
    },
    /// sys_spu_thread_group_start (173).
    SpuThreadGroupStart {
        /// Thread group id.
        group_id: u32,
    },
    /// sys_spu_thread_group_join (177).
    SpuThreadGroupJoin {
        /// Thread group id.
        group_id: u32,
        /// Out-pointer for the exit cause.
        cause_ptr: u32,
        /// Out-pointer for the exit status.
        status_ptr: u32,
    },
    /// sys_spu_thread_group_terminate (178).
    ///
    /// Distinct from [`Self::SpuThreadGroupJoin`]; 177 takes two
    /// out-pointers, 178 takes an in-param status.
    SpuThreadGroupTerminate {
        /// Thread group id.
        group_id: u32,
        /// Termination status delivered to any subsequent joiner.
        value: i32,
    },
    /// sys_time_get_current_time (145). Writes `(sec, nsec)` into
    /// two 64-bit out-pointers. `nsec` is `0..=999_999_999`.
    TimeGetCurrentTime {
        /// Out-pointer for seconds since an implementation-defined
        /// origin (CellGov uses runtime start).
        sec_ptr: u32,
        /// Out-pointer for the nanosecond remainder `0..=999_999_999`.
        nsec_ptr: u32,
    },
    /// sys_time_get_timebase_frequency (147). Return-only; no
    /// arguments. The dispatch arm answers with the PPU timebase
    /// register frequency from `cellgov_time::CELL_PPU_TIMEBASE_HZ`.
    TimeGetTimebaseFrequency,
    /// sys_time_get_timezone (144). Writes the timezone offset and
    /// summer-time offset as two 32-bit big-endian integers. CellGov
    /// is a deterministic oracle with no host-time dependency, so
    /// both slots receive zero (UTC, no DST).
    TimeGetTimezone {
        /// Out-pointer for the timezone offset in minutes (`be_t<s32>`).
        timezone_ptr: u32,
        /// Out-pointer for the summer-time offset in minutes (`be_t<s32>`).
        summer_time_ptr: u32,
    },
    /// sys_tty_write (403).
    TtyWrite {
        /// File descriptor.
        fd: u32,
        /// Buffer to write.
        buf_ptr: u32,
        /// Byte count.
        len: u32,
        /// Out-pointer for bytes-written count.
        nwritten_ptr: u32,
    },
    /// sys_spu_thread_write_spu_mb (190).
    SpuThreadWriteMb {
        /// SPU thread id.
        thread_id: u32,
        /// Value deposited into the SPU inbound mailbox.
        value: u32,
    },
    /// sys_mutex_create (100).
    MutexCreate {
        /// Out-pointer for the mutex id.
        id_ptr: u32,
        /// Attribute struct (opaque).
        attr_ptr: u32,
    },
    /// sys_mutex_destroy (101).
    MutexDestroy {
        /// Mutex id.
        mutex_id: u32,
    },
    /// sys_mutex_lock (102).
    MutexLock {
        /// Mutex id.
        mutex_id: u32,
        /// Timeout in microseconds; 0 means infinite. Ignored.
        timeout: u64,
    },
    /// sys_mutex_unlock (104).
    MutexUnlock {
        /// Mutex id.
        mutex_id: u32,
    },
    /// sys_mutex_trylock (103).
    MutexTryLock {
        /// Mutex id.
        mutex_id: u32,
    },
    /// sys_semaphore_create (93).
    SemaphoreCreate {
        /// Out-pointer for the semaphore id.
        id_ptr: u32,
        /// Attribute struct (opaque).
        attr_ptr: u32,
        /// Initial resource count.
        initial: i32,
        /// Maximum resource count.
        max: i32,
    },
    /// sys_semaphore_destroy (94).
    SemaphoreDestroy {
        /// Semaphore id.
        id: u32,
    },
    /// sys_semaphore_wait (114).
    SemaphoreWait {
        /// Semaphore id.
        id: u32,
        /// Timeout in microseconds; 0 means infinite. Ignored.
        timeout: u64,
    },
    /// sys_semaphore_post (115).
    SemaphorePost {
        /// Semaphore id.
        id: u32,
        /// Slots to post; only `val == 1` is accepted.
        val: i32,
    },
    /// sys_semaphore_trywait (116).
    SemaphoreTryWait {
        /// Semaphore id.
        id: u32,
    },
    /// sys_semaphore_get_value (117).
    SemaphoreGetValue {
        /// Semaphore id.
        id: u32,
        /// Out-pointer for the count.
        out_ptr: u32,
    },
    /// sys_event_queue_create (128).
    EventQueueCreate {
        /// Out-pointer for the queue id.
        id_ptr: u32,
        /// Attribute struct (opaque).
        attr_ptr: u32,
        /// Event queue key.
        key: u64,
        /// Maximum queue size.
        size: u32,
    },
    /// sys_event_queue_destroy (129).
    EventQueueDestroy {
        /// Queue id.
        queue_id: u32,
    },
    /// sys_event_queue_receive (130).
    EventQueueReceive {
        /// Queue id.
        queue_id: u32,
        /// Out-pointer for `sys_event_t` (32 bytes: source / data1 /
        /// data2 / data3, each u64 BE).
        out_ptr: u32,
        /// Timeout in microseconds; 0 means infinite. Ignored.
        timeout: u64,
    },
    /// sys_event_port_send (134).
    ///
    /// A port with no binding or a non-1:1 binding routes to ESRCH.
    EventPortSend {
        /// Event port id.
        port_id: u32,
        /// First payload word.
        data1: u64,
        /// Second payload word.
        data2: u64,
        /// Third payload word.
        data3: u64,
    },
    /// sys_event_flag_create (82).
    EventFlagCreate {
        /// Out-pointer for the flag id.
        id_ptr: u32,
        /// Attribute struct (opaque).
        attr_ptr: u32,
        /// Initial bit state.
        init: u64,
    },
    /// sys_event_flag_destroy (83).
    EventFlagDestroy {
        /// Event flag id.
        id: u32,
    },
    /// sys_event_flag_wait (84).
    EventFlagWait {
        /// Event flag id.
        id: u32,
        /// Bit mask to match.
        bits: u64,
        /// Raw ABI wait-mode word; handler maps to `EventFlagWaitMode`.
        mode: u32,
        /// Out-pointer for the observed bit pattern.
        result_ptr: u32,
        /// Timeout in microseconds; 0 means infinite. Ignored.
        timeout: u64,
    },
    /// sys_event_flag_set (86).
    EventFlagSet {
        /// Event flag id.
        id: u32,
        /// Bits to OR into the flag.
        bits: u64,
    },
    /// sys_event_flag_clear (87).
    EventFlagClear {
        /// Event flag id.
        id: u32,
        /// Bits to clear.
        bits: u64,
    },
    /// sys_event_flag_trywait (85).
    EventFlagTryWait {
        /// Event flag id.
        id: u32,
        /// Bit mask to match.
        bits: u64,
        /// Raw ABI wait-mode word.
        mode: u32,
        /// Out-pointer for the observed bit pattern.
        result_ptr: u32,
    },
    /// sys_event_flag_cancel (132).
    EventFlagCancel {
        /// Event flag id.
        id: u32,
        /// Out-pointer for the count of woken waiters; may be 0 (NULL).
        num_ptr: u32,
    },
    /// sys_event_flag_get (139).
    EventFlagGet {
        /// Event flag id.
        id: u32,
        /// Out-pointer for the current bit pattern.
        flags_ptr: u32,
    },
    /// sys_event_queue_tryreceive (133).
    EventQueueTryReceive {
        /// Queue id.
        queue_id: u32,
        /// Output array (32 bytes per entry).
        event_array: u32,
        /// Maximum number of entries to write.
        size: u32,
        /// Out-pointer for the actual count.
        count_out: u32,
    },
    /// sys_memory_allocate (348).
    MemoryAllocate {
        /// Allocation size in bytes.
        size: u64,
        /// Page-size flags: 0x400 = 1MB, 0x200 = 64KB, 0 = 1MB default.
        flags: u64,
        /// Out-pointer for the allocated address.
        alloc_addr_ptr: u32,
    },
    /// sys_memory_free (349).
    MemoryFree {
        /// Guest address to free.
        addr: u32,
    },
    /// sys_memory_get_user_memory_size (352).
    MemoryGetUserMemorySize {
        /// Out-pointer for `sys_memory_info_t`.
        mem_info_ptr: u32,
    },
    /// sys_memory_container_create (341).
    MemoryContainerCreate {
        /// Out-pointer for the container id.
        cid_ptr: u32,
        /// Container size in bytes.
        size: u64,
    },
    /// sys_process_exit (22).
    ProcessExit {
        /// Exit code.
        code: u32,
    },
    /// sys_process_getpid (1).
    ProcessGetPid,
    /// sys_process_get_number_of_object (12).
    ProcessGetNumberOfObject {
        /// Object class id from `sys_process.h`'s `SYS_*_OBJECT` enum.
        class_id: u32,
        /// Out-pointer for the count (size_t, written as 64-bit BE).
        count_out_ptr: u32,
    },
    /// sys_process_getppid (18).
    ProcessGetPpid,
    /// sys_process_get_sdk_version (25).
    ProcessGetSdkVersion {
        /// Target PID (we model a single-process world; ignored).
        pid: u32,
        /// Out-pointer for the SDK version (s32, written as 32-bit BE).
        version_out_ptr: u32,
    },
    /// `_sys_process_get_paramsfo` (30). Writes a 64-byte SFO header
    /// blob to the caller's buffer.
    ProcessGetParamsfo {
        /// Out-pointer for the 64-byte SFO blob.
        buf_ptr: u32,
    },
    /// sys_process_get_ppu_guid (31).
    ProcessGetPpuGuid,
    /// sys_timer_create (70). CellGov tracks the live count for
    /// `sys_process_get_number_of_object` but does not model timer
    /// expiry semantics; the id is allocated and write-back-fired.
    TimerCreate {
        /// Out-pointer for the timer id.
        id_ptr: u32,
    },
    /// sys_timer_destroy (71).
    TimerDestroy {
        /// Timer id (currently unused; just decrements the count).
        id: u32,
    },
    /// sys_rwlock_create (120). Stub: id allocator with live-count
    /// tracking; no read/write contention modeling.
    RwlockCreate {
        /// Out-pointer for the rwlock id.
        id_ptr: u32,
        /// Attribute struct (opaque).
        attr_ptr: u32,
    },
    /// sys_rwlock_destroy (121).
    RwlockDestroy {
        /// Rwlock id.
        id: u32,
    },
    /// sys_event_port_create (134). Stub: id allocator with
    /// live-count tracking.
    EventPortCreate {
        /// Out-pointer for the event-port id.
        id_ptr: u32,
        /// Port type (`SYS_EVENT_PORT_LOCAL` etc; opaque to the stub).
        port_type: u32,
        /// Port name (opaque).
        name: u64,
    },
    /// sys_event_port_destroy (135).
    EventPortDestroy {
        /// Event-port id.
        id: u32,
    },
    /// sys_process_is_stack (NID 0x4f7172c9, sysPrxForUser dispatch).
    /// Listed here for completeness; the NID handler does the
    /// region check rather than going through this variant.
    ProcessIsStack {
        /// Guest pointer to test for membership in any stack region.
        addr: u32,
    },
    /// sys_ppu_thread_yield (43).
    PpuThreadYield,
    /// sys_ppu_thread_exit (41).
    PpuThreadExit {
        /// Exit value passed to joiners' r3 on wake.
        exit_value: u64,
    },
    /// sys_ppu_thread_join (44).
    PpuThreadJoin {
        /// Child thread id.
        target: u64,
        /// Out-pointer for the child's exit value.
        status_out_ptr: u32,
    },
    /// sys_lwmutex_create (95).
    LwMutexCreate {
        /// Out-pointer for the lwmutex id.
        id_ptr: u32,
        /// Attribute struct (opaque).
        attr_ptr: u32,
    },
    /// sys_lwmutex_destroy (96).
    LwMutexDestroy {
        /// Lwmutex id.
        id: u32,
    },
    /// sys_lwmutex_lock (97).
    LwMutexLock {
        /// Lwmutex id (the kernel-side `sleep_queue`).
        id: u32,
        /// User-space `sys_lwmutex_t` address. Captured by the
        /// host so the post-wake handler can update the owner /
        /// recursive_count / waiter fields.
        mutex_ptr: u32,
        /// Timeout in microseconds; 0 means infinite. Ignored.
        timeout: u64,
    },
    /// sys_lwmutex_unlock (98).
    LwMutexUnlock {
        /// Lwmutex id.
        id: u32,
    },
    /// sys_lwmutex_trylock (99).
    LwMutexTryLock {
        /// Lwmutex id.
        id: u32,
    },
    /// sys_fs_open (801). Minimal handler -- whitelist is empty, so
    /// every call returns `CELL_ENOENT` and writes 0 to `fd_out_ptr`.
    FsOpen {
        /// Guest pointer to the path string.
        path_ptr: u32,
        /// Open flags (`O_RDONLY|O_NONBLOCK|O_LARGEFILE` etc).
        flags: u32,
        /// Out-pointer for the file descriptor.
        fd_out_ptr: u32,
        /// Mode bits (only consulted when the path matches the
        /// whitelist; ignored under the minimal handler).
        mode: u32,
    },
    /// sys_fs_close (804). Stub: decrements the live-fd counter for
    /// `sys_process_get_number_of_object` and returns CELL_OK.
    FsClose {
        /// File descriptor.
        fd: u32,
    },
    /// sys_fs_write (803). Reads `size` bytes from `buf_ptr` and
    /// appends them to the host's `tty_log` so the ps3autotests
    /// harness can compare against `<test>.expected` whether the
    /// test wrote via printf (TTY) or fprintf (output.txt).
    FsWrite {
        /// File descriptor (unused; treated as the unified
        /// write-stream).
        fd: u32,
        /// Source buffer.
        buf_ptr: u32,
        /// Byte count.
        size: u32,
        /// Out-pointer for `nwrite` (bytes actually written).
        nwrite_ptr: u32,
    },
    /// sys_cond_create (105).
    CondCreate {
        /// Out-pointer for the cond id.
        id_ptr: u32,
        /// Associated heavy mutex id.
        mutex_id: u32,
        /// Attribute struct (opaque).
        attr_ptr: u32,
    },
    /// sys_cond_destroy (106).
    CondDestroy {
        /// Cond id.
        id: u32,
    },
    /// sys_cond_wait (107).
    CondWait {
        /// Cond id.
        id: u32,
        /// Timeout in microseconds; 0 means infinite. Ignored.
        timeout: u64,
    },
    /// sys_cond_signal (108).
    CondSignal {
        /// Cond id.
        id: u32,
    },
    /// sys_cond_signal_all (109).
    CondSignalAll {
        /// Cond id.
        id: u32,
    },
    /// sys_cond_signal_to (110).
    CondSignalTo {
        /// Cond id.
        id: u32,
        /// Target PPU thread id.
        target_thread: u32,
    },
    /// sys_ppu_thread_create (52).
    PpuThreadCreate {
        /// Out-pointer for the thread id.
        id_ptr: u32,
        /// OPD address: first 8 bytes code, next 8 bytes TOC.
        entry_opd: u32,
        /// Argument passed as the child's r3.
        arg: u64,
        /// Priority (not consulted by the scheduler).
        priority: u32,
        /// Requested child stack size in bytes.
        stacksize: u64,
        /// Flags (not interpreted).
        flags: u64,
    },
    /// sys_rsx_memory_allocate (665).
    SysRsxMemoryAllocate {
        /// Out-pointer for the memory handle.
        mem_handle_ptr: u32,
        /// Out-pointer for the allocated guest address.
        mem_addr_ptr: u32,
        /// Requested size in bytes.
        size: u32,
        /// Allocation flags.
        flags: u64,
        /// Reserved.
        a5: u64,
        /// Reserved.
        a6: u64,
        /// Reserved.
        a7: u64,
    },
    /// sys_rsx_memory_free (667).
    SysRsxMemoryFree {
        /// Handle from a prior `SysRsxMemoryAllocate`.
        mem_handle: u32,
    },
    /// sys_rsx_context_allocate (670).
    SysRsxContextAllocate {
        /// Out-pointer for the context id.
        context_id_ptr: u32,
        /// Out-pointer for the DMA-control base address.
        lpar_dma_control_ptr: u32,
        /// Out-pointer for the driver-info base address.
        lpar_driver_info_ptr: u32,
        /// Out-pointer for the reports base address.
        lpar_reports_ptr: u32,
        /// Memory context handle from `SysRsxMemoryAllocate`.
        mem_ctx: u64,
        /// System-mode flag word.
        system_mode: u64,
    },
    /// sys_rsx_context_free (671).
    SysRsxContextFree {
        /// Context id from a prior `SysRsxContextAllocate`.
        context_id: u32,
    },
    /// sys_rsx_context_attribute (674); `package_id` is the sub-command.
    SysRsxContextAttribute {
        /// Context id from a prior `SysRsxContextAllocate`.
        context_id: u32,
        /// Sub-command selector (FLIP_MODE, FLIP_BUFFER,
        /// SET_DISPLAY_BUFFER, SET_FLIP_HANDLER, SET_VBLANK_HANDLER, ...).
        package_id: u32,
        /// Sub-command argument.
        a3: u64,
        /// Sub-command argument.
        a4: u64,
        /// Sub-command argument.
        a5: u64,
        /// Sub-command argument.
        a6: u64,
    },
    /// Unknown syscall number; raw args preserved for trace.
    Unsupported {
        /// Raw syscall number from GPR 11.
        number: u64,
        /// Raw GPR values from r3..=r10.
        args: [u64; 8],
    },
    /// Recognised syscall whose arguments are out of ABI range; raw
    /// args preserved for trace, dispatcher routes to CELL_EINVAL.
    Malformed {
        /// Raw syscall number.
        number: u64,
        /// Which field failed to decode.
        reason: &'static str,
        /// Raw GPR values from r3..=r10.
        args: [u64; 8],
    },
}

const HIGH_BITS_REASONS: [&str; 8] = [
    "arg 0: non-zero high 32 bits in u32 field",
    "arg 1: non-zero high 32 bits in u32 field",
    "arg 2: non-zero high 32 bits in u32 field",
    "arg 3: non-zero high 32 bits in u32 field",
    "arg 4: non-zero high 32 bits in u32 field",
    "arg 5: non-zero high 32 bits in u32 field",
    "arg 6: non-zero high 32 bits in u32 field",
    "arg 7: non-zero high 32 bits in u32 field",
];

const I32_RANGE_REASONS: [&str; 8] = [
    "arg 0: not representable as i32",
    "arg 1: not representable as i32",
    "arg 2: not representable as i32",
    "arg 3: not representable as i32",
    "arg 4: not representable as i32",
    "arg 5: not representable as i32",
    "arg 6: not representable as i32",
    "arg 7: not representable as i32",
];

/// Build an [`Lv2Request`] from the raw syscall number (r11) and
/// argument GPRs (r3..=r10).
pub fn classify(syscall_num: u64, args: &[u64; 8]) -> Lv2Request {
    // `s!` uses `as i64` to reverse PPC64 sign extension: a guest
    // `int x = -1` arrives as 0xFFFF_FFFF_FFFF_FFFF, decodes to
    // -1i64, and `i32::try_from` rejects anything that isn't a
    // clean sign extension (e.g. 0x1_0000_0001 or 2^31).
    macro_rules! p {
        ($idx:expr) => {
            match u32::try_from(args[$idx]) {
                Ok(v) => v,
                Err(_) => {
                    return Lv2Request::Malformed {
                        number: syscall_num,
                        reason: HIGH_BITS_REASONS[$idx],
                        args: *args,
                    };
                }
            }
        };
    }
    macro_rules! s {
        ($idx:expr) => {
            match i32::try_from(args[$idx] as i64) {
                Ok(v) => v,
                Err(_) => {
                    return Lv2Request::Malformed {
                        number: syscall_num,
                        reason: I32_RANGE_REASONS[$idx],
                        args: *args,
                    };
                }
            }
        };
    }

    match syscall_num {
        syscall::SPU_IMAGE_OPEN => Lv2Request::SpuImageOpen {
            img_ptr: p!(0),
            path_ptr: p!(1),
        },
        syscall::SPU_THREAD_GROUP_CREATE => Lv2Request::SpuThreadGroupCreate {
            id_ptr: p!(0),
            num_threads: p!(1),
            priority: p!(2),
            attr_ptr: p!(3),
        },
        syscall::SPU_THREAD_INITIALIZE => Lv2Request::SpuThreadInitialize {
            thread_ptr: p!(0),
            group_id: p!(1),
            thread_num: p!(2),
            img_ptr: p!(3),
            attr_ptr: p!(4),
            arg_ptr: p!(5),
        },
        syscall::SPU_THREAD_GROUP_START => Lv2Request::SpuThreadGroupStart { group_id: p!(0) },
        syscall::SPU_THREAD_GROUP_JOIN => Lv2Request::SpuThreadGroupJoin {
            group_id: p!(0),
            cause_ptr: p!(1),
            status_ptr: p!(2),
        },
        syscall::SPU_THREAD_GROUP_TERMINATE => Lv2Request::SpuThreadGroupTerminate {
            group_id: p!(0),
            value: s!(1),
        },
        syscall::SPU_THREAD_WRITE_MB => Lv2Request::SpuThreadWriteMb {
            thread_id: p!(0),
            value: p!(1),
        },
        syscall::TIME_GET_TIMEZONE => Lv2Request::TimeGetTimezone {
            timezone_ptr: p!(0),
            summer_time_ptr: p!(1),
        },
        syscall::TIME_GET_CURRENT_TIME => Lv2Request::TimeGetCurrentTime {
            sec_ptr: p!(0),
            nsec_ptr: p!(1),
        },
        syscall::TIME_GET_TIMEBASE_FREQUENCY => Lv2Request::TimeGetTimebaseFrequency,
        syscall::TTY_WRITE => Lv2Request::TtyWrite {
            fd: p!(0),
            buf_ptr: p!(1),
            len: p!(2),
            nwritten_ptr: p!(3),
        },
        syscall::PROCESS_EXIT => Lv2Request::ProcessExit { code: p!(0) },
        syscall::PROCESS_GETPID => Lv2Request::ProcessGetPid,
        syscall::PROCESS_GET_NUMBER_OF_OBJECT => Lv2Request::ProcessGetNumberOfObject {
            class_id: p!(0),
            count_out_ptr: p!(1),
        },
        syscall::PROCESS_GETPPID => Lv2Request::ProcessGetPpid,
        syscall::PROCESS_GET_SDK_VERSION => Lv2Request::ProcessGetSdkVersion {
            pid: p!(0),
            version_out_ptr: p!(1),
        },
        syscall::PROCESS_GET_PARAMSFO => Lv2Request::ProcessGetParamsfo { buf_ptr: p!(0) },
        syscall::PROCESS_GET_PPU_GUID => Lv2Request::ProcessGetPpuGuid,
        syscall::TIMER_CREATE => Lv2Request::TimerCreate { id_ptr: p!(0) },
        syscall::TIMER_DESTROY => Lv2Request::TimerDestroy { id: p!(0) },
        syscall::RWLOCK_CREATE => Lv2Request::RwlockCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
        },
        syscall::RWLOCK_DESTROY => Lv2Request::RwlockDestroy { id: p!(0) },
        syscall::EVENT_PORT_CREATE => Lv2Request::EventPortCreate {
            id_ptr: p!(0),
            port_type: p!(1),
            name: args[2],
        },
        syscall::EVENT_PORT_DESTROY => Lv2Request::EventPortDestroy { id: p!(0) },
        syscall::PPU_THREAD_YIELD => Lv2Request::PpuThreadYield,
        syscall::PPU_THREAD_EXIT => Lv2Request::PpuThreadExit {
            exit_value: args[0],
        },
        syscall::PPU_THREAD_CREATE => Lv2Request::PpuThreadCreate {
            id_ptr: p!(0),
            entry_opd: p!(1),
            arg: args[2],
            priority: p!(3),
            stacksize: args[4],
            flags: args[5],
        },
        syscall::SYS_RSX_MEMORY_ALLOCATE => Lv2Request::SysRsxMemoryAllocate {
            mem_handle_ptr: p!(0),
            mem_addr_ptr: p!(1),
            size: p!(2),
            flags: args[3],
            a5: args[4],
            a6: args[5],
            a7: args[6],
        },
        syscall::SYS_RSX_MEMORY_FREE => Lv2Request::SysRsxMemoryFree { mem_handle: p!(0) },
        syscall::SYS_RSX_CONTEXT_ALLOCATE => Lv2Request::SysRsxContextAllocate {
            context_id_ptr: p!(0),
            lpar_dma_control_ptr: p!(1),
            lpar_driver_info_ptr: p!(2),
            lpar_reports_ptr: p!(3),
            mem_ctx: args[4],
            system_mode: args[5],
        },
        syscall::SYS_RSX_CONTEXT_FREE => Lv2Request::SysRsxContextFree { context_id: p!(0) },
        syscall::SYS_RSX_CONTEXT_ATTRIBUTE => Lv2Request::SysRsxContextAttribute {
            context_id: p!(0),
            package_id: p!(1),
            a3: args[2],
            a4: args[3],
            a5: args[4],
            a6: args[5],
        },
        syscall::PPU_THREAD_JOIN => Lv2Request::PpuThreadJoin {
            target: args[0],
            status_out_ptr: p!(1),
        },
        syscall::LWMUTEX_CREATE => Lv2Request::LwMutexCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
        },
        syscall::LWMUTEX_DESTROY => Lv2Request::LwMutexDestroy { id: p!(0) },
        syscall::LWMUTEX_LOCK => Lv2Request::LwMutexLock {
            id: p!(0),
            // The raw `sys_lwmutex_lock` LV2 syscall does not carry a
            // user-space struct pointer; only the HLE wrapper does.
            mutex_ptr: 0,
            timeout: args[1],
        },
        syscall::LWMUTEX_UNLOCK => Lv2Request::LwMutexUnlock { id: p!(0) },
        syscall::LWMUTEX_TRYLOCK => Lv2Request::LwMutexTryLock { id: p!(0) },
        syscall::FS_CLOSE => Lv2Request::FsClose { fd: p!(0) },
        syscall::FS_WRITE => Lv2Request::FsWrite {
            fd: p!(0),
            buf_ptr: p!(1),
            size: p!(2),
            nwrite_ptr: p!(3),
        },
        syscall::FS_OPEN => Lv2Request::FsOpen {
            path_ptr: p!(0),
            flags: p!(1),
            fd_out_ptr: p!(2),
            mode: p!(3),
        },
        syscall::MUTEX_CREATE => Lv2Request::MutexCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
        },
        syscall::MUTEX_DESTROY => Lv2Request::MutexDestroy { mutex_id: p!(0) },
        syscall::MUTEX_LOCK => Lv2Request::MutexLock {
            mutex_id: p!(0),
            timeout: args[1],
        },
        syscall::MUTEX_UNLOCK => Lv2Request::MutexUnlock { mutex_id: p!(0) },
        syscall::MUTEX_TRYLOCK => Lv2Request::MutexTryLock { mutex_id: p!(0) },
        syscall::SEMAPHORE_CREATE => Lv2Request::SemaphoreCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
            initial: s!(2),
            max: s!(3),
        },
        syscall::SEMAPHORE_DESTROY => Lv2Request::SemaphoreDestroy { id: p!(0) },
        syscall::SEMAPHORE_WAIT => Lv2Request::SemaphoreWait {
            id: p!(0),
            timeout: args[1],
        },
        syscall::SEMAPHORE_POST => Lv2Request::SemaphorePost {
            id: p!(0),
            val: s!(1),
        },
        syscall::SEMAPHORE_TRY_WAIT => Lv2Request::SemaphoreTryWait { id: p!(0) },
        syscall::SEMAPHORE_GET_VALUE => Lv2Request::SemaphoreGetValue {
            id: p!(0),
            out_ptr: p!(1),
        },
        syscall::EVENT_QUEUE_CREATE => Lv2Request::EventQueueCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
            key: args[2],
            size: p!(3),
        },
        syscall::EVENT_QUEUE_DESTROY => Lv2Request::EventQueueDestroy { queue_id: p!(0) },
        syscall::EVENT_QUEUE_RECEIVE => Lv2Request::EventQueueReceive {
            queue_id: p!(0),
            out_ptr: p!(1),
            timeout: args[2],
        },
        syscall::EVENT_FLAG_CREATE => Lv2Request::EventFlagCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
            init: args[2],
        },
        syscall::EVENT_FLAG_DESTROY => Lv2Request::EventFlagDestroy { id: p!(0) },
        syscall::EVENT_FLAG_WAIT => Lv2Request::EventFlagWait {
            id: p!(0),
            bits: args[1],
            mode: p!(2),
            result_ptr: p!(3),
            timeout: args[4],
        },
        syscall::EVENT_FLAG_TRY_WAIT => Lv2Request::EventFlagTryWait {
            id: p!(0),
            bits: args[1],
            mode: p!(2),
            result_ptr: p!(3),
        },
        syscall::EVENT_FLAG_SET => Lv2Request::EventFlagSet {
            id: p!(0),
            bits: args[1],
        },
        syscall::EVENT_FLAG_CLEAR => Lv2Request::EventFlagClear {
            id: p!(0),
            bits: args[1],
        },
        syscall::EVENT_FLAG_CANCEL => Lv2Request::EventFlagCancel {
            id: p!(0),
            num_ptr: p!(1),
        },
        syscall::EVENT_FLAG_GET => Lv2Request::EventFlagGet {
            id: p!(0),
            flags_ptr: p!(1),
        },
        syscall::EVENT_QUEUE_TRY_RECEIVE => Lv2Request::EventQueueTryReceive {
            queue_id: p!(0),
            event_array: p!(1),
            size: p!(2),
            count_out: p!(3),
        },
        syscall::EVENT_PORT_SEND => Lv2Request::EventPortSend {
            port_id: p!(0),
            data1: args[1],
            data2: args[2],
            data3: args[3],
        },
        syscall::COND_CREATE => Lv2Request::CondCreate {
            id_ptr: p!(0),
            mutex_id: p!(1),
            attr_ptr: p!(2),
        },
        syscall::COND_DESTROY => Lv2Request::CondDestroy { id: p!(0) },
        syscall::COND_WAIT => Lv2Request::CondWait {
            id: p!(0),
            timeout: args[1],
        },
        syscall::COND_SIGNAL => Lv2Request::CondSignal { id: p!(0) },
        syscall::COND_SIGNAL_ALL => Lv2Request::CondSignalAll { id: p!(0) },
        syscall::COND_SIGNAL_TO => Lv2Request::CondSignalTo {
            id: p!(0),
            target_thread: p!(1),
        },
        syscall::MEMORY_ALLOCATE => Lv2Request::MemoryAllocate {
            size: args[0],
            flags: args[1],
            alloc_addr_ptr: p!(2),
        },
        syscall::MEMORY_FREE => Lv2Request::MemoryFree { addr: p!(0) },
        syscall::MEMORY_GET_USER_MEMORY_SIZE => Lv2Request::MemoryGetUserMemorySize {
            mem_info_ptr: p!(0),
        },
        syscall::MEMORY_CONTAINER_CREATE => Lv2Request::MemoryContainerCreate {
            cid_ptr: p!(0),
            size: args[1],
        },
        // Known shapes whose effects are not yet modelled; listing
        // them forces a review when a handler is added.
        171 | 174 | 175 | 176 | 179 | 180 | 192 => Lv2Request::Unsupported {
            number: syscall_num,
            args: *args,
        },
        n => Lv2Request::Unsupported {
            number: n,
            args: *args,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_spu_image_open() {
        let args = [0x1000, 0x2000, 0, 0, 0, 0, 0, 0];
        let req = classify(156, &args);
        assert_eq!(
            req,
            Lv2Request::SpuImageOpen {
                img_ptr: 0x1000,
                path_ptr: 0x2000,
            }
        );
    }

    #[test]
    fn classify_thread_group_create() {
        let args = [0x3000, 2, 100, 0x4000, 0, 0, 0, 0];
        let req = classify(170, &args);
        assert_eq!(
            req,
            Lv2Request::SpuThreadGroupCreate {
                id_ptr: 0x3000,
                num_threads: 2,
                priority: 100,
                attr_ptr: 0x4000,
            }
        );
    }

    #[test]
    fn classify_thread_initialize() {
        let args = [0x6000, 1, 0, 0x7000, 0x8000, 0x9000, 0, 0];
        let req = classify(172, &args);
        assert_eq!(
            req,
            Lv2Request::SpuThreadInitialize {
                thread_ptr: 0x6000,
                group_id: 1,
                thread_num: 0,
                img_ptr: 0x7000,
                attr_ptr: 0x8000,
                arg_ptr: 0x9000,
            }
        );
    }

    #[test]
    fn classify_thread_group_start() {
        let args = [7, 0, 0, 0, 0, 0, 0, 0];
        let req = classify(173, &args);
        assert_eq!(req, Lv2Request::SpuThreadGroupStart { group_id: 7 });
    }

    #[test]
    fn classify_thread_group_join() {
        let args = [3, 0x6000, 0x7000, 0, 0, 0, 0, 0];
        let req = classify(178, &args);
        assert_eq!(
            req,
            Lv2Request::SpuThreadGroupJoin {
                group_id: 3,
                cause_ptr: 0x6000,
                status_ptr: 0x7000,
            }
        );
    }

    #[test]
    fn classify_thread_group_terminate_is_separate_from_join() {
        let args = [3, 0xFFFF_FFFF_FFFF_FFFF, 0, 0, 0, 0, 0, 0];
        let req = classify(177, &args);
        assert_eq!(
            req,
            Lv2Request::SpuThreadGroupTerminate {
                group_id: 3,
                value: -1,
            }
        );
    }

    #[test]
    fn classify_tty_write() {
        let args = [0, 0x8000, 64, 0x9000, 0, 0, 0, 0];
        let req = classify(403, &args);
        assert_eq!(
            req,
            Lv2Request::TtyWrite {
                fd: 0,
                buf_ptr: 0x8000,
                len: 64,
                nwritten_ptr: 0x9000,
            }
        );
    }

    #[test]
    fn classify_process_exit() {
        let args = [0, 0, 0, 0, 0, 0, 0, 0];
        let req = classify(22, &args);
        assert_eq!(req, Lv2Request::ProcessExit { code: 0 });
    }

    #[test]
    fn classify_ppu_thread_yield() {
        let args = [0xDEAD, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(classify(43, &args), Lv2Request::PpuThreadYield);
    }

    #[test]
    fn classify_time_get_timebase_frequency_ignores_args() {
        let args = [0xDEAD, 0xBEEF, 0, 0, 0, 0, 0, 0];
        assert_eq!(classify(147, &args), Lv2Request::TimeGetTimebaseFrequency);
    }

    #[test]
    fn classify_time_get_current_time_captures_out_pointers() {
        let args = [0x9000, 0x9008, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(145, &args),
            Lv2Request::TimeGetCurrentTime {
                sec_ptr: 0x9000,
                nsec_ptr: 0x9008,
            }
        );
    }

    #[test]
    fn classify_time_get_timezone_captures_out_pointers() {
        let args = [0xd000_fd10, 0xd000_fd14, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(144, &args),
            Lv2Request::TimeGetTimezone {
                timezone_ptr: 0xd000_fd10,
                summer_time_ptr: 0xd000_fd14,
            }
        );
    }

    #[test]
    fn classify_memory_get_user_memory_size_captures_out_pointer() {
        let args = [0xd000_fdf4, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(352, &args),
            Lv2Request::MemoryGetUserMemorySize {
                mem_info_ptr: 0xd000_fdf4,
            }
        );
    }

    #[test]
    fn classify_ppu_thread_exit_captures_exit_value() {
        let args = [0xDEAD_BEEF, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(41, &args),
            Lv2Request::PpuThreadExit {
                exit_value: 0xDEAD_BEEF
            },
        );
    }

    #[test]
    fn classify_ppu_thread_join_captures_target_and_out_ptr() {
        let args = [0x0100_0003, 0x5000, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(44, &args),
            Lv2Request::PpuThreadJoin {
                target: 0x0100_0003,
                status_out_ptr: 0x5000,
            },
        );
    }

    #[test]
    fn classify_ppu_thread_create_captures_all_fields() {
        let args = [0x3000, 0x2_0000, 0xCAFE_BABE, 1500, 0x10_000, 0, 0, 0];
        assert_eq!(
            classify(52, &args),
            Lv2Request::PpuThreadCreate {
                id_ptr: 0x3000,
                entry_opd: 0x2_0000,
                arg: 0xCAFE_BABE,
                priority: 1500,
                stacksize: 0x10_000,
                flags: 0,
            },
        );
    }

    #[test]
    fn classify_unknown_syscall() {
        let args = [0xAA, 0xBB, 0xCC, 0, 0, 0, 0, 0];
        let req = classify(999, &args);
        assert_eq!(
            req,
            Lv2Request::Unsupported {
                number: 999,
                args: [0xAA, 0xBB, 0xCC, 0, 0, 0, 0, 0],
            }
        );
    }

    #[test]
    fn unsupported_preserves_args_for_diagnosis() {
        let args = [0x1234, 0x5678, 0x9ABC, 0xDEF0, 1, 2, 3, 4];
        let req = classify(600, &args);
        match req {
            Lv2Request::Unsupported { number, args: a } => {
                assert_eq!(number, 600);
                assert_eq!(a, args);
            }
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn spu_thread_group_range_stubs_classify_as_unsupported() {
        let args = [0; 8];
        for n in [171, 174, 175, 176, 179, 180, 192] {
            let req = classify(n, &args);
            assert!(
                matches!(req, Lv2Request::Unsupported { number, .. } if number == n),
                "syscall {n} should be Unsupported",
            );
        }
    }

    #[test]
    fn classify_mutex_create() {
        let args = [0x5000, 0x6000, 0, 0, 0, 0, 0, 0];
        let req = classify(100, &args);
        assert_eq!(
            req,
            Lv2Request::MutexCreate {
                id_ptr: 0x5000,
                attr_ptr: 0x6000,
            }
        );
    }

    #[test]
    fn classify_lwmutex_create_destroy() {
        let create_args = [0x5000, 0x6000, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(95, &create_args),
            Lv2Request::LwMutexCreate {
                id_ptr: 0x5000,
                attr_ptr: 0x6000,
            }
        );
        let destroy_args = [7, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(96, &destroy_args),
            Lv2Request::LwMutexDestroy { id: 7 }
        );
    }

    #[test]
    fn classify_lwmutex_lock() {
        let args = [7, 100, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(97, &args),
            Lv2Request::LwMutexLock {
                id: 7,
                mutex_ptr: 0,
                timeout: 100
            }
        );
    }

    #[test]
    fn classify_lwmutex_unlock() {
        let args = [9, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(classify(98, &args), Lv2Request::LwMutexUnlock { id: 9 });
    }

    #[test]
    fn classify_lwmutex_trylock() {
        let args = [11, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(classify(99, &args), Lv2Request::LwMutexTryLock { id: 11 });
    }

    #[test]
    fn classify_mutex_trylock() {
        let args = [42, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(103, &args),
            Lv2Request::MutexTryLock { mutex_id: 42 }
        );
    }

    #[test]
    fn classify_event_queue_receive_send() {
        assert_eq!(
            classify(130, &[7, 0x1000, 500, 0, 0, 0, 0, 0]),
            Lv2Request::EventQueueReceive {
                queue_id: 7,
                out_ptr: 0x1000,
                timeout: 500,
            }
        );
        assert_eq!(
            classify(138, &[7, 0xaa, 0xbb, 0xcc, 0, 0, 0, 0]),
            Lv2Request::EventPortSend {
                port_id: 7,
                data1: 0xaa,
                data2: 0xbb,
                data3: 0xcc,
            }
        );
        assert_eq!(
            classify(131, &[7, 0x2000, 4, 0x3000, 0, 0, 0, 0]),
            Lv2Request::EventQueueTryReceive {
                queue_id: 7,
                event_array: 0x2000,
                size: 4,
                count_out: 0x3000,
            }
        );
    }

    #[test]
    fn classify_semaphore_trywait_and_get_value() {
        assert_eq!(
            classify(93, &[7, 0, 0, 0, 0, 0, 0, 0]),
            Lv2Request::SemaphoreTryWait { id: 7 }
        );
        assert_eq!(
            classify(114, &[7, 0x1000, 0, 0, 0, 0, 0, 0]),
            Lv2Request::SemaphoreGetValue {
                id: 7,
                out_ptr: 0x1000
            }
        );
    }

    #[test]
    fn classify_semaphore_post() {
        let args = [7, 1, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(94, &args),
            Lv2Request::SemaphorePost { id: 7, val: 1 }
        );
    }

    #[test]
    fn classify_semaphore_create_destroy_wait() {
        let create_args = [0x5000, 0x6000, 2, 10, 0, 0, 0, 0];
        assert_eq!(
            classify(90, &create_args),
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x5000,
                attr_ptr: 0x6000,
                initial: 2,
                max: 10,
            }
        );
        let destroy_args = [7, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(91, &destroy_args),
            Lv2Request::SemaphoreDestroy { id: 7 }
        );
        let wait_args = [7, 100, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(92, &wait_args),
            Lv2Request::SemaphoreWait {
                id: 7,
                timeout: 100
            }
        );
    }

    #[test]
    fn classify_mutex_lock_unlock() {
        let args = [42, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(102, &args),
            Lv2Request::MutexLock {
                mutex_id: 42,
                timeout: 0,
            }
        );
        assert_eq!(
            classify(104, &args),
            Lv2Request::MutexUnlock { mutex_id: 42 }
        );
    }

    #[test]
    fn classify_event_queue_create_destroy() {
        let args = [0x7000, 0x8000, 0x100, 64, 0, 0, 0, 0];
        assert_eq!(
            classify(128, &args),
            Lv2Request::EventQueueCreate {
                id_ptr: 0x7000,
                attr_ptr: 0x8000,
                key: 0x100,
                size: 64,
            }
        );
        let args2 = [99, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(129, &args2),
            Lv2Request::EventQueueDestroy { queue_id: 99 }
        );
    }

    #[test]
    fn classify_cond_create_destroy_wait() {
        assert_eq!(
            classify(105, &[0x5000, 7, 0x6000, 0, 0, 0, 0, 0]),
            Lv2Request::CondCreate {
                id_ptr: 0x5000,
                mutex_id: 7,
                attr_ptr: 0x6000,
            }
        );
        assert_eq!(
            classify(106, &[9, 0, 0, 0, 0, 0, 0, 0]),
            Lv2Request::CondDestroy { id: 9 }
        );
        assert_eq!(
            classify(107, &[9, 500, 0, 0, 0, 0, 0, 0]),
            Lv2Request::CondWait {
                id: 9,
                timeout: 500,
            }
        );
    }

    #[test]
    fn classify_cond_signal_variants() {
        assert_eq!(
            classify(108, &[9, 0, 0, 0, 0, 0, 0, 0]),
            Lv2Request::CondSignal { id: 9 }
        );
        assert_eq!(
            classify(109, &[9, 0, 0, 0, 0, 0, 0, 0]),
            Lv2Request::CondSignalAll { id: 9 }
        );
        assert_eq!(
            classify(110, &[9, 0x0100_0005, 0, 0, 0, 0, 0, 0]),
            Lv2Request::CondSignalTo {
                id: 9,
                target_thread: 0x0100_0005,
            }
        );
    }

    #[test]
    fn classify_memory_allocate_free() {
        let args = [0x10000, 0x200, 0x9000, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(348, &args),
            Lv2Request::MemoryAllocate {
                size: 0x10000,
                flags: 0x200,
                alloc_addr_ptr: 0x9000,
            }
        );
        let args2 = [0x0001_0000, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(349, &args2),
            Lv2Request::MemoryFree { addr: 0x0001_0000 }
        );
    }

    #[test]
    fn narrow_ptr_rejects_high_bits_in_u32_field() {
        let args = [0x1_0000_1000, 0x2000, 0, 0, 0, 0, 0, 0];
        match classify(100, &args) {
            Lv2Request::Malformed {
                number,
                reason,
                args: a,
            } => {
                assert_eq!(number, 100);
                assert!(
                    reason.contains("arg 0") && reason.contains("high"),
                    "unexpected reason: {reason}",
                );
                assert_eq!(a, args);
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn narrow_i32_accepts_sign_extended_negatives() {
        let args = [
            0x5000,
            0x6000,
            0xFFFF_FFFF_FFFF_FFFF,
            0xFFFF_FFFF_FFFF_FFFE,
            0,
            0,
            0,
            0,
        ];
        assert_eq!(
            classify(90, &args),
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x5000,
                attr_ptr: 0x6000,
                initial: -1,
                max: -2,
            }
        );
    }

    #[test]
    fn narrow_i32_rejects_values_outside_i32_range() {
        let args = [0x5000, 0x6000, 0x1_0000_0001, 10, 0, 0, 0, 0];
        match classify(90, &args) {
            Lv2Request::Malformed { number, reason, .. } => {
                assert_eq!(number, 90);
                assert!(
                    reason.contains("arg 2") && reason.contains("i32"),
                    "unexpected reason: {reason}",
                );
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn narrow_i32_rejects_large_positive() {
        // 2^31 fits in u32 but not in i32; old cast wrapped to i32::MIN.
        let args = [0x5000, 0x6000, 0x8000_0000, 10, 0, 0, 0, 0];
        assert!(matches!(
            classify(90, &args),
            Lv2Request::Malformed { number: 90, .. }
        ));
    }

    /// Syscalls and GPR slots that must reject high bits; regression
    /// fence against `args[N] as u32` slipping in for a new arm.
    const U32_SLOTS_BY_SYSCALL: &[(u64, &[usize])] = &[
        (22, &[0]),
        (44, &[1]),
        (52, &[0, 1, 3]),
        (82, &[0, 1]),
        (83, &[0]),
        (85, &[0, 2, 3]),
        (86, &[0, 2, 3]),
        (87, &[0]),
        (90, &[0, 1]),
        (91, &[0]),
        (92, &[0]),
        (93, &[0]),
        (94, &[0]),
        (95, &[0, 1]),
        (96, &[0]),
        (97, &[0]),
        (98, &[0]),
        (99, &[0]),
        (100, &[0, 1]),
        (102, &[0]),
        (103, &[0]),
        (104, &[0]),
        (105, &[0, 1, 2]),
        (106, &[0]),
        (107, &[0]),
        (108, &[0]),
        (109, &[0]),
        (110, &[0, 1]),
        (114, &[0, 1]),
        (118, &[0]),
        (128, &[0, 1, 3]),
        (129, &[0]),
        (130, &[0, 1]),
        (131, &[0, 1, 2, 3]),
        (138, &[0]),
        (145, &[0, 1]),
        (156, &[0, 1]),
        (170, &[0, 1, 2, 3]),
        (172, &[0, 1, 2, 3, 4, 5]),
        (173, &[0]),
        (177, &[0]),
        (178, &[0, 1, 2]),
        (190, &[0, 1]),
        (341, &[0]),
        (348, &[2]),
        (349, &[0]),
        (352, &[0]),
        (403, &[0, 1, 2, 3]),
        (668, &[0, 1, 2]),
        (669, &[0]),
        (670, &[0, 1, 2, 3]),
        (671, &[0]),
        (674, &[0, 1]),
    ];

    #[test]
    fn every_u32_slot_rejects_high_bits() {
        for &(num, slots) in U32_SLOTS_BY_SYSCALL {
            for &slot in slots {
                let mut args = [0u64; 8];
                args[slot] = 0x1_0000_0000;
                match classify(num, &args) {
                    Lv2Request::Malformed {
                        number,
                        reason,
                        args: a,
                    } => {
                        assert_eq!(number, num, "syscall {num} slot {slot}");
                        assert_eq!(a, args, "syscall {num} slot {slot}");
                        let tag = format!("arg {slot}");
                        assert!(
                            reason.contains(&tag),
                            "syscall {num} slot {slot}: reason {reason:?} did not name {tag:?}",
                        );
                    }
                    other => {
                        panic!("syscall {num} slot {slot}: expected Malformed, got {other:?}",)
                    }
                }
            }
        }
    }

    #[test]
    fn semaphore_post_val_narrowing() {
        let ok = [7u64, 0xFFFF_FFFF_FFFF_FFFF, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(94, &ok),
            Lv2Request::SemaphorePost { id: 7, val: -1 }
        );
        let bad = [7u64, 0x1_0000_0001, 0, 0, 0, 0, 0, 0];
        assert!(matches!(
            classify(94, &bad),
            Lv2Request::Malformed { number: 94, .. }
        ));
    }

    #[test]
    fn classify_sys_rsx_memory_allocate() {
        let args = [0x1000, 0x1008, 0x0010_0000, 0x400, 0, 0, 0, 0];
        assert_eq!(
            classify(668, &args),
            Lv2Request::SysRsxMemoryAllocate {
                mem_handle_ptr: 0x1000,
                mem_addr_ptr: 0x1008,
                size: 0x0010_0000,
                flags: 0x400,
                a5: 0,
                a6: 0,
                a7: 0,
            }
        );
    }

    #[test]
    fn classify_sys_rsx_memory_free() {
        let args = [0xA001, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(669, &args),
            Lv2Request::SysRsxMemoryFree { mem_handle: 0xA001 }
        );
    }

    #[test]
    fn classify_sys_rsx_context_allocate() {
        let args = [0x2000, 0x2008, 0x2010, 0x2018, 0xA001, 0, 0, 0];
        assert_eq!(
            classify(670, &args),
            Lv2Request::SysRsxContextAllocate {
                context_id_ptr: 0x2000,
                lpar_dma_control_ptr: 0x2008,
                lpar_driver_info_ptr: 0x2010,
                lpar_reports_ptr: 0x2018,
                mem_ctx: 0xA001,
                system_mode: 0,
            }
        );
    }

    #[test]
    fn classify_sys_rsx_context_free() {
        let args = [0x5555_5555, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(671, &args),
            Lv2Request::SysRsxContextFree {
                context_id: 0x5555_5555,
            }
        );
    }

    #[test]
    fn classify_sys_rsx_context_attribute() {
        let args = [0x5555_5555, 0x102, 0xAA, 0xBB, 0xCC, 0xDD, 0, 0];
        assert_eq!(
            classify(674, &args),
            Lv2Request::SysRsxContextAttribute {
                context_id: 0x5555_5555,
                package_id: 0x102,
                a3: 0xAA,
                a4: 0xBB,
                a5: 0xCC,
                a6: 0xDD,
            }
        );
    }
}
