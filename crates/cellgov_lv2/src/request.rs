//! Typed LV2 syscall requests decoded from PPU `sc` GPR state.
//!
//! [`classify`] is total: unknown numbers and malformed arguments
//! surface as [`Lv2Request::Unsupported`] / [`Lv2Request::Malformed`]
//! rather than panicking, so host dispatch can match exhaustively.
//!
//! # Cross-crate contract
//!
//! Pointer fields are guest effective addresses (u32, big-endian on
//! the bus). The classifier rejects a u32-typed slot whose source
//! GPR has non-zero high 32 bits rather than truncating; downstream
//! `Lv2Host` handlers therefore never receive a silently-narrowed
//! pointer. Out-pointers carry the convention that the kernel must
//! commit `*out = id` before returning OK -- the runtime emits the
//! write and the OK return as a single atomic effect batch so guests
//! that race a sibling thread on the id never observe a stale slot.

use cellgov_ps3_abi::callback_dispatch::CB_RETURN_SYSCALL;
use cellgov_ps3_abi::syscall;
use cellgov_ps3_abi::syscall_namespace::SyscallNamespace;

/// Typed LV2 syscall request decoded from PPU `sc` GPR state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2Request {
    /// sys_spu_image_open syscall.
    SpuImageOpen {
        /// In: SPU image struct pointer.
        img_ptr: u32,
        /// In: path string pointer.
        path_ptr: u32,
    },
    /// sys_spu_thread_group_create syscall.
    SpuThreadGroupCreate {
        /// Out: group id.
        id_ptr: u32,
        /// In: thread count.
        num_threads: u32,
        /// In: group priority.
        priority: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
    },
    /// sys_spu_thread_initialize syscall.
    SpuThreadInitialize {
        /// Out: thread id.
        thread_ptr: u32,
        /// In: parent group id.
        group_id: u32,
        /// In: index within the group.
        thread_num: u32,
        /// In: SPU image pointer.
        img_ptr: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
        /// In: argument struct pointer.
        arg_ptr: u32,
    },
    /// sys_spu_thread_group_start syscall.
    SpuThreadGroupStart {
        /// In: group id.
        group_id: u32,
    },
    /// sys_spu_thread_group_join syscall.
    SpuThreadGroupJoin {
        /// In: group id.
        group_id: u32,
        /// Out: cause word.
        cause_ptr: u32,
        /// Out: status word.
        status_ptr: u32,
    },
    /// Distinct from [`Self::SpuThreadGroupJoin`]: 178 takes an
    /// in-param status, 177 takes two out-pointers.
    SpuThreadGroupTerminate {
        /// In: group id.
        group_id: u32,
        /// In: termination value.
        value: i32,
    },
    /// `nsec` lies in `0..=999_999_999`.
    TimeGetCurrentTime {
        /// Out: seconds.
        sec_ptr: u32,
        /// Out: nanoseconds.
        nsec_ptr: u32,
    },
    /// sys_time_get_timebase_frequency syscall.
    TimeGetTimebaseFrequency,
    /// CellGov is host-time-free, so both slots receive zero
    /// (UTC, no DST).
    TimeGetTimezone {
        /// Out: timezone offset.
        timezone_ptr: u32,
        /// Out: summer-time flag.
        summer_time_ptr: u32,
    },
    /// sys_tty_write syscall.
    TtyWrite {
        /// In: tty fd.
        fd: u32,
        /// In: buffer pointer.
        buf_ptr: u32,
        /// In: byte count.
        len: u32,
        /// Out: bytes written.
        nwritten_ptr: u32,
    },
    /// sys_spu_thread_write_in_mbox syscall.
    SpuThreadWriteMb {
        /// In: target SPU thread id.
        thread_id: u32,
        /// In: mailbox value.
        value: u32,
    },
    /// sys_mutex_create syscall.
    MutexCreate {
        /// Out: mutex id.
        id_ptr: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
    },
    /// sys_mutex_destroy syscall.
    MutexDestroy {
        /// In: mutex id to free.
        mutex_id: u32,
    },
    /// `timeout == 0` means infinite; the field is currently ignored.
    MutexLock {
        /// In: mutex id.
        mutex_id: u32,
        /// In: timeout in microseconds.
        timeout: u64,
    },
    /// sys_mutex_unlock syscall.
    MutexUnlock {
        /// In: mutex id.
        mutex_id: u32,
    },
    /// sys_mutex_trylock syscall.
    MutexTryLock {
        /// In: mutex id.
        mutex_id: u32,
    },
    /// sys_semaphore_create syscall.
    SemaphoreCreate {
        /// Out: semaphore id.
        id_ptr: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
        /// In: initial value.
        initial: i32,
        /// In: max value.
        max: i32,
    },
    /// sys_semaphore_destroy syscall.
    SemaphoreDestroy {
        /// In: semaphore id.
        id: u32,
    },
    /// `timeout == 0` means infinite; the field is currently ignored.
    SemaphoreWait {
        /// In: semaphore id.
        id: u32,
        /// In: timeout in microseconds.
        timeout: u64,
    },
    /// Only `val == 1` is accepted by the handler.
    SemaphorePost {
        /// In: semaphore id.
        id: u32,
        /// In: post count.
        val: i32,
    },
    /// sys_semaphore_trywait syscall.
    SemaphoreTryWait {
        /// In: semaphore id.
        id: u32,
    },
    /// sys_semaphore_get_value syscall.
    SemaphoreGetValue {
        /// In: semaphore id.
        id: u32,
        /// Out: current value.
        out_ptr: u32,
    },
    /// sys_event_queue_create syscall.
    EventQueueCreate {
        /// Out: queue id.
        id_ptr: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
        /// In: IPC key.
        key: u64,
        /// In: queue depth.
        size: u32,
    },
    /// sys_event_queue_destroy syscall.
    EventQueueDestroy {
        /// In: queue id.
        queue_id: u32,
    },
    /// `out_ptr` receives 32 bytes: source / data1 / data2 / data3,
    /// each u64 BE. `timeout == 0` means infinite (currently ignored).
    EventQueueReceive {
        /// In: queue id.
        queue_id: u32,
        /// Out: event packet.
        out_ptr: u32,
        /// In: timeout in microseconds.
        timeout: u64,
    },
    /// A port with no binding or a non-1:1 binding routes to ESRCH.
    EventPortSend {
        /// In: port id.
        port_id: u32,
        /// In: event payload data1.
        data1: u64,
        /// In: event payload data2.
        data2: u64,
        /// In: event payload data3.
        data3: u64,
    },
    /// sys_event_flag_create syscall.
    EventFlagCreate {
        /// Out: event flag id.
        id_ptr: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
        /// In: initial bit pattern.
        init: u64,
    },
    /// sys_event_flag_destroy syscall.
    EventFlagDestroy {
        /// In: event flag id.
        id: u32,
    },
    /// `mode` is the raw ABI wait-mode word; the handler maps to
    /// `EventFlagWaitMode`. `timeout == 0` means infinite (ignored).
    EventFlagWait {
        /// In: event flag id.
        id: u32,
        /// In: bit pattern to wait on.
        bits: u64,
        /// In: raw wait-mode word.
        mode: u32,
        /// Out: matched bit pattern.
        result_ptr: u32,
        /// In: timeout in microseconds.
        timeout: u64,
    },
    /// sys_event_flag_set syscall.
    EventFlagSet {
        /// In: event flag id.
        id: u32,
        /// In: bits to set.
        bits: u64,
    },
    /// sys_event_flag_clear syscall.
    EventFlagClear {
        /// In: event flag id.
        id: u32,
        /// In: bits to clear.
        bits: u64,
    },
    /// sys_event_flag_trywait syscall.
    EventFlagTryWait {
        /// In: event flag id.
        id: u32,
        /// In: bit pattern to wait on.
        bits: u64,
        /// In: raw wait-mode word.
        mode: u32,
        /// Out: matched bit pattern.
        result_ptr: u32,
    },
    /// `num_ptr` may be 0 (NULL) -- the handler treats that as
    /// "discard the count" rather than EFAULT.
    EventFlagCancel {
        /// In: event flag id.
        id: u32,
        /// Out: cancelled-waiter count.
        num_ptr: u32,
    },
    /// sys_event_flag_get syscall.
    EventFlagGet {
        /// In: event flag id.
        id: u32,
        /// Out: current bit pattern.
        flags_ptr: u32,
    },
    /// sys_event_queue_tryreceive syscall.
    EventQueueTryReceive {
        /// In: queue id.
        queue_id: u32,
        /// Out: event array buffer.
        event_array: u32,
        /// In: array element count.
        size: u32,
        /// Out: events received.
        count_out: u32,
    },
    /// `flags`: 0x400 = 1MB pages, 0x200 = 64KB pages, 0 = 1MB default.
    MemoryAllocate {
        /// In: allocation size in bytes.
        size: u64,
        /// In: page-size flags.
        flags: u64,
        /// Out: allocated address.
        alloc_addr_ptr: u32,
    },
    /// sys_memory_free syscall.
    MemoryFree {
        /// In: address to free.
        addr: u32,
    },
    /// sys_memory_get_user_memory_size syscall.
    MemoryGetUserMemorySize {
        /// Out: sys_memory_info_t struct.
        mem_info_ptr: u32,
    },
    /// sys_memory_container_create syscall.
    MemoryContainerCreate {
        /// Out: container id.
        cid_ptr: u32,
        /// In: container size in bytes.
        size: u64,
    },
    /// sys_process_exit syscall.
    ProcessExit {
        /// In: exit code.
        code: u32,
    },
    /// sys_process_getpid syscall.
    ProcessGetPid,
    /// `class_id` is from `sys_process.h`'s `SYS_*_OBJECT` enum;
    /// `count_out_ptr` receives a size_t written as 64-bit BE.
    ProcessGetNumberOfObject {
        /// In: object class id.
        class_id: u32,
        /// Out: object count.
        count_out_ptr: u32,
    },
    /// sys_process_getppid syscall.
    ProcessGetPpid,
    /// CellGov models a single-process world; `pid` is ignored.
    ProcessGetSdkVersion {
        /// In: target pid (ignored).
        pid: u32,
        /// Out: SDK version word.
        version_out_ptr: u32,
    },
    /// Writes a 64-byte SFO header blob to `buf_ptr`.
    ProcessGetParamsfo {
        /// Out: SFO header buffer.
        buf_ptr: u32,
    },
    /// sys_process_get_ppu_guid syscall.
    ProcessGetPpuGuid,
    /// Stub: tracks live count for `sys_process_get_number_of_object`;
    /// no expiry semantics.
    TimerCreate {
        /// Out: timer id.
        id_ptr: u32,
    },
    /// sys_timer_destroy syscall.
    TimerDestroy {
        /// In: timer id.
        id: u32,
    },
    /// Stub: id allocator + live-count; no read/write contention.
    RwlockCreate {
        /// Out: rwlock id.
        id_ptr: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
    },
    /// sys_rwlock_destroy syscall.
    RwlockDestroy {
        /// In: rwlock id.
        id: u32,
    },
    /// Stub: id allocator with live-count tracking.
    EventPortCreate {
        /// Out: port id.
        id_ptr: u32,
        /// In: port type.
        port_type: u32,
        /// In: port name.
        name: u64,
    },
    /// sys_event_port_destroy syscall.
    EventPortDestroy {
        /// In: port id.
        id: u32,
    },
    /// Listed for completeness; the sysPrxForUser NID handler does
    /// the region check rather than routing through this variant.
    ProcessIsStack {
        /// In: address to query.
        addr: u32,
    },
    /// sys_ppu_thread_yield syscall.
    PpuThreadYield,
    /// sys_ppu_thread_exit syscall.
    PpuThreadExit {
        /// In: exit value.
        exit_value: u64,
    },
    /// sys_ppu_thread_join syscall.
    PpuThreadJoin {
        /// In: target thread id.
        target: u64,
        /// Out: thread status.
        status_out_ptr: u32,
    },
    /// sys_lwmutex_create syscall.
    LwMutexCreate {
        /// Out: lwmutex id.
        id_ptr: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
    },
    /// sys_lwmutex_destroy syscall.
    LwMutexDestroy {
        /// In: lwmutex id.
        id: u32,
    },
    /// `mutex_ptr` is the user-space `sys_lwmutex_t` address. The
    /// raw LV2 syscall does not carry it -- the HLE wrapper does --
    /// so the post-wake handler can update owner / recursive_count /
    /// waiter fields. The raw-syscall path arrives with
    /// `mutex_ptr == 0` and the handler skips the user-struct write.
    /// `timeout == 0` means infinite (ignored).
    LwMutexLock {
        /// In: lwmutex id.
        id: u32,
        /// In: user-space sys_lwmutex_t address (HLE only; 0 on raw syscall).
        mutex_ptr: u32,
        /// In: timeout in microseconds.
        timeout: u64,
    },
    /// sys_lwmutex_unlock syscall.
    LwMutexUnlock {
        /// In: lwmutex id.
        id: u32,
    },
    /// sys_lwmutex_trylock syscall.
    LwMutexTryLock {
        /// In: lwmutex id.
        id: u32,
    },
    /// Minimal handler: whitelist is empty, so every call returns
    /// `CELL_ENOENT` and writes 0 to `fd_out_ptr`.
    FsOpen {
        /// In: path string pointer.
        path_ptr: u32,
        /// In: open flags.
        flags: u32,
        /// Out: fd.
        fd_out_ptr: u32,
        /// In: open mode.
        mode: u32,
    },
    /// cellFsClose syscall.
    FsClose {
        /// In: fd to close.
        fd: u32,
    },
    /// Reads up to `nbytes` from the fd's offset, advancing it by
    /// the count actually returned. `nread_out_ptr` is u64,
    /// 8-byte-aligned.
    FsRead {
        /// In: fd.
        fd: u32,
        /// Out: read buffer.
        buf_ptr: u32,
        /// In: requested byte count.
        nbytes: u64,
        /// Out: bytes actually read.
        nread_out_ptr: u32,
    },
    /// `whence`: 0 = SEEK_SET, 1 = SEEK_CUR, 2 = SEEK_END; anything
    /// else surfaces as CELL_EINVAL. `pos_out_ptr` is u64, 8-aligned.
    FsLseek {
        /// In: fd.
        fd: u32,
        /// In: signed offset.
        offset: i64,
        /// In: whence selector.
        whence: u32,
        /// Out: resulting position.
        pos_out_ptr: u32,
    },
    /// `stat_out_ptr` receives a 56-byte `CellFsStat`, 8-byte aligned.
    FsFstat {
        /// In: fd.
        fd: u32,
        /// Out: CellFsStat buffer.
        stat_out_ptr: u32,
    },
    /// Path-keyed variant of `FsFstat`; same struct layout.
    FsStat {
        /// In: path string pointer.
        path_ptr: u32,
        /// Out: CellFsStat buffer.
        stat_out_ptr: u32,
    },
    /// `sys_fs_opendir` -- snapshot a host directory's entries
    /// (sorted lexicographically) and allocate a directory fd.
    FsOpendir {
        /// In: path string pointer (NUL-terminated guest UTF-8).
        path_ptr: u32,
        /// Out: u32 directory fd, 4-byte aligned.
        fd_out_ptr: u32,
    },
    /// `sys_fs_readdir` -- copy the next snapshot entry into a
    /// 258-byte `CellFsDirent`, write the byte count to
    /// `nread_out_ptr` (`sizeof(CellFsDirent) = 258` on success;
    /// `0` at EOF).
    FsReaddir {
        /// In: directory fd.
        fd: u32,
        /// Out: CellFsDirent buffer (258 bytes, no required alignment).
        dirent_out_ptr: u32,
        /// Out: u64 byte count, 8-byte aligned.
        nread_out_ptr: u32,
    },
    /// `sys_fs_closedir` -- release a directory fd.
    FsClosedir {
        /// In: directory fd to close.
        fd: u32,
    },
    /// `fd` is unused; bytes are appended to the host's unified
    /// `tty_log` so the ps3autotests harness can match either printf
    /// or fprintf output against `<test>.expected`.
    FsWrite {
        /// In: fd (unused).
        fd: u32,
        /// In: buffer pointer.
        buf_ptr: u32,
        /// In: byte count.
        size: u32,
        /// Out: bytes written.
        nwrite_ptr: u32,
    },
    /// sys_cond_create syscall.
    CondCreate {
        /// Out: cond id.
        id_ptr: u32,
        /// In: associated mutex id.
        mutex_id: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
    },
    /// sys_cond_destroy syscall.
    CondDestroy {
        /// In: cond id.
        id: u32,
    },
    /// `timeout == 0` means infinite (currently ignored).
    CondWait {
        /// In: cond id.
        id: u32,
        /// In: timeout in microseconds.
        timeout: u64,
    },
    /// sys_cond_signal syscall.
    CondSignal {
        /// In: cond id.
        id: u32,
    },
    /// sys_cond_signal_all syscall.
    CondSignalAll {
        /// In: cond id.
        id: u32,
    },
    /// sys_cond_signal_to syscall.
    CondSignalTo {
        /// In: cond id.
        id: u32,
        /// In: target thread id.
        target_thread: u32,
    },
    /// `entry_opd` points to a 16-byte OPD: code || toc.
    PpuThreadCreate {
        /// Out: thread id.
        id_ptr: u32,
        /// In: entry OPD pointer.
        entry_opd: u32,
        /// In: thread argument.
        arg: u64,
        /// In: priority.
        priority: u32,
        /// In: stack size in bytes.
        stacksize: u64,
        /// In: creation flags.
        flags: u64,
    },
    /// sys_rsx_memory_allocate syscall.
    SysRsxMemoryAllocate {
        /// Out: RSX memory handle.
        mem_handle_ptr: u32,
        /// Out: mapped address.
        mem_addr_ptr: u32,
        /// In: allocation size.
        size: u32,
        /// In: allocation flags.
        flags: u64,
        /// In: raw arg5.
        a5: u64,
        /// In: raw arg6.
        a6: u64,
        /// In: raw arg7.
        a7: u64,
    },
    /// sys_rsx_memory_free syscall.
    SysRsxMemoryFree {
        /// In: RSX memory handle.
        mem_handle: u32,
    },
    /// sys_rsx_context_allocate syscall.
    SysRsxContextAllocate {
        /// Out: context id.
        context_id_ptr: u32,
        /// Out: lpar DMA control address.
        lpar_dma_control_ptr: u32,
        /// Out: lpar driver-info address.
        lpar_driver_info_ptr: u32,
        /// Out: lpar reports address.
        lpar_reports_ptr: u32,
        /// In: parent memory context.
        mem_ctx: u64,
        /// In: system mode flags.
        system_mode: u64,
    },
    /// sys_rsx_context_free syscall.
    SysRsxContextFree {
        /// In: RSX context id.
        context_id: u32,
    },
    /// `package_id` selects the sub-command (FLIP_MODE, FLIP_BUFFER,
    /// SET_DISPLAY_BUFFER, SET_FLIP_HANDLER, SET_VBLANK_HANDLER, ...).
    SysRsxContextAttribute {
        /// In: RSX context id.
        context_id: u32,
        /// In: sub-command selector.
        package_id: u32,
        /// In: raw arg3.
        a3: u64,
        /// In: raw arg4.
        a4: u64,
        /// In: raw arg5.
        a5: u64,
        /// In: raw arg6.
        a6: u64,
    },
    /// Internal worker-spawn -- not guest-issued, never produced by
    /// [`classify`]. Fabricated by HLE handlers via
    /// `Lv2Host::call_guest_callback_sync`; the host materializes a
    /// fresh worker PPU thread with the title-supplied OPD and parks
    /// `parent` until the worker returns.
    CallbackDispatchSpawn {
        /// In: title-supplied OPD pointer.
        opd: u32,
        /// In: worker register arguments.
        args: [u64; 8],
        /// In: parent unit to park until return.
        parent: cellgov_event::UnitId,
    },
    /// Issued by the CellGov-private trampoline in
    /// `cellgov_ps3_abi::callback_dispatch` when the worker's
    /// terminal `blr` lands on it. [`classify`] decodes this from
    /// `r11 = CB_RETURN_SYSCALL` (bit 19 set). `args` are forwarded
    /// to the parent via
    /// [`crate::dispatch::PendingResponse::CallbackReturn`].
    CallbackDispatchReturn {
        /// In: worker return registers.
        args: [u64; 8],
    },
    /// `sc` with non-zero LEV. PS3 usermode must never issue this;
    /// the runtime rejects rather than letting the call reach LV2
    /// dispatch unflagged.
    Hypercall {
        /// In: privilege level.
        lev: u8,
        /// In: r11 (syscall-number register).
        r11: u64,
        /// In: r3..=r10 arguments.
        args: [u64; 8],
    },
    /// Unknown syscall number; raw args preserved for trace.
    Unsupported {
        /// In: syscall number.
        number: u64,
        /// In: raw arguments.
        args: [u64; 8],
    },
    /// Recognised syscall whose arguments are out of ABI range;
    /// `reason` names the failing field, dispatcher routes to
    /// CELL_EINVAL.
    Malformed {
        /// In: syscall number.
        number: u64,
        /// In: failing-field description.
        reason: &'static str,
        /// In: raw arguments.
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

/// LEV=0 wrapper around [`classify_with_lev`] for synthetic / fake-ISA
/// test paths. Real PPU `sc` decode goes through `classify_with_lev`.
#[inline]
pub fn classify(syscall_num: u64, args: &[u64; 8]) -> Lv2Request {
    classify_with_lev(0, syscall_num, args)
}

/// Build an [`Lv2Request`] from the `sc` LEV field, r11, and r3..=r10.
///
/// Non-zero `lev` routes to [`Lv2Request::Hypercall`]; the LV2
/// dispatcher must not see it.
pub fn classify_with_lev(lev: u8, syscall_num: u64, args: &[u64; 8]) -> Lv2Request {
    if lev != 0 {
        return Lv2Request::Hypercall {
            lev,
            r11: syscall_num,
            args: *args,
        };
    }
    // `s!` reverses PPC64 sign extension via `as i64` then
    // `i32::try_from`: a guest `int x = -1` arrives as
    // 0xFFFF_FFFF_FFFF_FFFF, decodes to -1i64, and any value that
    // isn't a clean sign extension (e.g. 0x1_0000_0001 or 2^31) is
    // rejected as Malformed rather than wrapped.
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

    // Match on every namespace explicitly so a new variant in
    // SyscallNamespace forces a compile error here. HleImport is
    // consumed upstream by NID lookup; surfacing it as Unsupported
    // keeps the total-classification contract.
    match SyscallNamespace::of(syscall_num) {
        Some(SyscallNamespace::CellGovPrivate) => {
            return match syscall_num {
                CB_RETURN_SYSCALL => Lv2Request::CallbackDispatchReturn { args: *args },
                n => Lv2Request::Unsupported {
                    number: n,
                    args: *args,
                },
            };
        }
        Some(SyscallNamespace::HleImport) => {
            return Lv2Request::Unsupported {
                number: syscall_num,
                args: *args,
            };
        }
        Some(SyscallNamespace::Lv2) | None => {}
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
            // Raw syscall path: no user-space struct pointer (only
            // the HLE wrapper carries one). See LwMutexLock docs.
            mutex_ptr: 0,
            timeout: args[1],
        },
        syscall::LWMUTEX_UNLOCK => Lv2Request::LwMutexUnlock { id: p!(0) },
        syscall::LWMUTEX_TRYLOCK => Lv2Request::LwMutexTryLock { id: p!(0) },
        syscall::FS_CLOSE => Lv2Request::FsClose { fd: p!(0) },
        syscall::FS_READ => Lv2Request::FsRead {
            fd: p!(0),
            buf_ptr: p!(1),
            nbytes: args[2],
            nread_out_ptr: p!(3),
        },
        syscall::FS_LSEEK => Lv2Request::FsLseek {
            fd: p!(0),
            offset: args[1] as i64,
            whence: p!(2),
            pos_out_ptr: p!(3),
        },
        syscall::FS_FSTAT => Lv2Request::FsFstat {
            fd: p!(0),
            stat_out_ptr: p!(1),
        },
        syscall::FS_STAT => Lv2Request::FsStat {
            path_ptr: p!(0),
            stat_out_ptr: p!(1),
        },
        syscall::FS_OPENDIR => Lv2Request::FsOpendir {
            path_ptr: p!(0),
            fd_out_ptr: p!(1),
        },
        syscall::FS_READDIR => Lv2Request::FsReaddir {
            fd: p!(0),
            dirent_out_ptr: p!(1),
            nread_out_ptr: p!(2),
        },
        syscall::FS_CLOSEDIR => Lv2Request::FsClosedir { fd: p!(0) },
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
        // SPU-thread syscalls with a known number but no modelled
        // effect; listed here so adding a handler is a one-line edit
        // rather than a search.
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
        // 2^31 fits u32 but not i32; the prior `as i32` cast wrapped
        // to i32::MIN -- verify the typed path now rejects.
        let args = [0x5000, 0x6000, 0x8000_0000, 10, 0, 0, 0, 0];
        assert!(matches!(
            classify(90, &args),
            Lv2Request::Malformed { number: 90, .. }
        ));
    }

    /// Regression fence: every (syscall, slot) pair the table
    /// covers must reject a non-zero high half rather than wrap.
    /// Catches `args[N] as u32` slipping in for a new arm.
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
    fn classify_callback_return_syscall_routes_via_bit19() {
        let args = [
            0x1111, 0x2222, 0x3333, 0x4444, 0x5555, 0x6666, 0x7777, 0x8888,
        ];
        let req = classify(CB_RETURN_SYSCALL, &args);
        assert_eq!(req, Lv2Request::CallbackDispatchReturn { args });
    }

    #[test]
    fn classify_unknown_private_syscall_falls_through_to_unsupported() {
        let args = [0; 8];
        let bogus = CB_RETURN_SYSCALL | 0x1000;
        let req = classify(bogus, &args);
        match req {
            Lv2Request::Unsupported { number, .. } => assert_eq!(number, bogus),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn real_lv2_syscalls_classify_into_lv2_namespace() {
        for n in [
            syscall::PROCESS_EXIT,
            syscall::PPU_THREAD_CREATE,
            syscall::FS_OPEN,
            syscall::TTY_WRITE,
        ] {
            assert_eq!(
                SyscallNamespace::of(n),
                Some(SyscallNamespace::Lv2),
                "syscall {n:#x} must classify into Lv2",
            );
        }
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
