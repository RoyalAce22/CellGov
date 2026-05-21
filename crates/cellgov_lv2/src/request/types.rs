use std::num::NonZeroU8;

/// Typed LV2 syscall request decoded from PPU `sc` GPR state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2Request {
    /// `sys_spu_image_open`.
    SpuImageOpen {
        /// In: SPU image struct pointer.
        img_ptr: u32,
        /// In: path string pointer.
        path_ptr: u32,
    },
    /// `sys_spu_image_import`. Register the `size` bytes at `img_ptr`
    /// in [`crate::image::ContentStore`] under a synthetic
    /// path-string and write the resulting handle into the SPU image
    /// struct at `handle_out`.
    SpuImageImport {
        /// Out: SPU image struct pointer (16 bytes, BE).
        handle_out: u32,
        /// In: raw SPU ELF bytes pointer.
        img_ptr: u32,
        /// In: byte length of the raw SPU ELF. `sys_spu_image_import`
        /// takes `size_t size`; the full u64 is preserved at
        /// classification, dispatcher caps against the 256 KiB SPU
        /// local-store limit.
        size: u64,
        /// In: image type tag; recorded in the synthetic path.
        type_id: u32,
    },
    /// `sys_spu_thread_group_create`.
    SpuThreadGroupCreate {
        /// Out: group id.
        id_ptr: u32,
        /// In: thread count.
        num_threads: u32,
        /// In: group priority. LV2 ABI is signed `int priority`;
        /// classify validates sign extension before constructing the
        /// variant.
        priority: i32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
    },
    /// `sys_spu_thread_initialize`.
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
    /// `sys_spu_thread_group_start`.
    SpuThreadGroupStart {
        /// In: group id.
        group_id: u32,
    },
    /// `sys_spu_thread_group_destroy`. LV2 rejects with CELL_EBUSY
    /// while the group is still running; the title must terminate or
    /// join before destroying.
    SpuThreadGroupDestroy {
        /// In: group id.
        id: u32,
    },
    /// `sys_spu_thread_group_join`.
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
    /// `sys_time_get_timebase_frequency`.
    TimeGetTimebaseFrequency,
    /// CellGov is host-time-free, so both slots receive zero
    /// (UTC, no DST).
    TimeGetTimezone {
        /// Out: timezone offset.
        timezone_ptr: u32,
        /// Out: summer-time flag.
        summer_time_ptr: u32,
    },
    /// `sys_tty_write`.
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
    /// `sys_spu_thread_write_in_mbox`.
    SpuThreadWriteMb {
        /// In: target SPU thread id.
        thread_id: u32,
        /// In: mailbox value.
        value: u32,
    },
    /// `sys_mutex_create`.
    MutexCreate {
        /// Out: mutex id.
        id_ptr: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
    },
    /// `sys_mutex_destroy`.
    MutexDestroy {
        /// In: mutex id to free.
        mutex_id: u32,
    },
    /// `timeout == 0` means infinite; the field is ignored.
    MutexLock {
        /// In: mutex id.
        mutex_id: u32,
        /// In: timeout in microseconds.
        timeout: u64,
    },
    /// `sys_mutex_unlock`.
    MutexUnlock {
        /// In: mutex id.
        mutex_id: u32,
    },
    /// `sys_mutex_trylock`.
    MutexTryLock {
        /// In: mutex id.
        mutex_id: u32,
    },
    /// `sys_semaphore_create`.
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
    /// `sys_semaphore_destroy`.
    SemaphoreDestroy {
        /// In: semaphore id.
        id: u32,
    },
    /// `timeout == 0` means infinite; the field is ignored.
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
    /// `sys_semaphore_trywait`.
    SemaphoreTryWait {
        /// In: semaphore id.
        id: u32,
    },
    /// `sys_semaphore_get_value`.
    SemaphoreGetValue {
        /// In: semaphore id.
        id: u32,
        /// Out: current value.
        out_ptr: u32,
    },
    /// `sys_event_queue_create`.
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
    /// `sys_event_queue_destroy`.
    EventQueueDestroy {
        /// In: queue id.
        queue_id: u32,
    },
    /// `out_ptr` receives 32 bytes: source / data1 / data2 / data3,
    /// each u64 BE. `timeout == 0` means infinite (ignored).
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
    /// `sys_event_flag_create`.
    EventFlagCreate {
        /// Out: event flag id.
        id_ptr: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
        /// In: initial bit pattern.
        init: u64,
    },
    /// `sys_event_flag_destroy`.
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
    /// `sys_event_flag_set`.
    EventFlagSet {
        /// In: event flag id.
        id: u32,
        /// In: bits to set.
        bits: u64,
    },
    /// `sys_event_flag_clear`.
    EventFlagClear {
        /// In: event flag id.
        id: u32,
        /// In: bits to clear.
        bits: u64,
    },
    /// `sys_event_flag_trywait`.
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
    /// `sys_event_flag_get`.
    EventFlagGet {
        /// In: event flag id.
        id: u32,
        /// Out: current bit pattern.
        flags_ptr: u32,
    },
    /// `sys_event_queue_tryreceive`.
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
    /// `sys_memory_free`.
    MemoryFree {
        /// In: address to free.
        addr: u32,
    },
    /// `sys_memory_get_user_memory_size`.
    MemoryGetUserMemorySize {
        /// Out: sys_memory_info_t struct.
        mem_info_ptr: u32,
    },
    /// `sys_memory_container_create`.
    MemoryContainerCreate {
        /// Out: container id.
        cid_ptr: u32,
        /// In: container size in bytes.
        size: u64,
    },
    /// `sys_process_exit`.
    ProcessExit {
        /// In: exit code. `sys_process_exit_with_status` takes
        /// `int32_t status`, so a guest `exit(-1)` arrives as a
        /// sign-extended `0xFFFF_FFFF_FFFF_FFFF` and decodes to
        /// `-1`, not Malformed.
        code: i32,
    },
    /// `sys_process_getpid`.
    ProcessGetPid,
    /// `class_id` is from `sys_process.h`'s `SYS_*_OBJECT` enum;
    /// `count_out_ptr` receives a size_t written as 64-bit BE.
    ProcessGetNumberOfObject {
        /// In: object class id.
        class_id: u32,
        /// Out: object count.
        count_out_ptr: u32,
    },
    /// `sys_process_getppid`.
    ProcessGetPpid,
    /// `sys_process_get_sdk_version`.
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
    /// `sys_process_get_ppu_guid`.
    ProcessGetPpuGuid,
    /// `sys_timer_create`.
    TimerCreate {
        /// Out: timer id.
        id_ptr: u32,
    },
    /// `sys_timer_destroy`.
    TimerDestroy {
        /// In: timer id.
        id: u32,
    },
    /// `sys_rwlock_create`.
    RwlockCreate {
        /// Out: rwlock id.
        id_ptr: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
    },
    /// `sys_rwlock_destroy`.
    RwlockDestroy {
        /// In: rwlock id.
        id: u32,
    },
    /// `sys_event_port_create`.
    EventPortCreate {
        /// Out: port id.
        id_ptr: u32,
        /// In: port type.
        port_type: u32,
        /// In: port name.
        name: u64,
    },
    /// `sys_event_port_destroy`.
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
    /// `sys_process_is_spu_lock_line_reservation_address`. Asks LV2
    /// whether `addr` falls inside the SPU lock-line reservation range
    /// (`0xE000_0000..=0xFFFF_FFFF`). `flags` is the optional second
    /// argument; LV2 rejects unknown bits.
    ProcessIsSpuLockLineReservationAddress {
        /// In: address to query.
        addr: u32,
        /// In: reservation flags.
        flags: u64,
    },
    /// `sys_spu_initialize`. Announces per-process SPU resource caps
    /// (max usable / max raw). CellGov does not model VSH-side SPU
    /// resource accounting; the dispatcher only validates the cap.
    SpuInitialize {
        /// In: maximum number of usable SPUs.
        max_usable_spu: u32,
        /// In: maximum number of raw SPUs.
        max_raw_spu: u32,
    },
    /// `sys_ppu_thread_yield`.
    PpuThreadYield,
    /// `sys_ppu_thread_exit`.
    PpuThreadExit {
        /// In: exit value.
        exit_value: u64,
    },
    /// `sys_ppu_thread_join`.
    PpuThreadJoin {
        /// In: target thread id.
        target: u64,
        /// Out: thread status.
        status_out_ptr: u32,
    },
    /// `sys_lwmutex_create`.
    LwMutexCreate {
        /// Out: lwmutex id.
        id_ptr: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
    },
    /// `sys_lwmutex_destroy`.
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
    /// `sys_lwmutex_unlock`.
    LwMutexUnlock {
        /// In: lwmutex id.
        id: u32,
    },
    /// `sys_lwmutex_trylock`.
    LwMutexTryLock {
        /// In: lwmutex id.
        id: u32,
    },
    /// `cellFsOpen`.
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
    /// `cellFsClose`.
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
        /// In: byte count. `sys_fs_write` takes `uint64_t size`,
        /// so the full u64 is preserved at classification; the
        /// dispatcher narrows for its tty-append fast path.
        size: u64,
        /// Out: bytes written.
        nwrite_ptr: u32,
    },
    /// `sys_cond_create`.
    CondCreate {
        /// Out: cond id.
        id_ptr: u32,
        /// In: associated mutex id.
        mutex_id: u32,
        /// In: attribute struct pointer.
        attr_ptr: u32,
    },
    /// `sys_cond_destroy`.
    CondDestroy {
        /// In: cond id.
        id: u32,
    },
    /// `timeout == 0` means infinite (ignored).
    CondWait {
        /// In: cond id.
        id: u32,
        /// In: timeout in microseconds.
        timeout: u64,
    },
    /// `sys_cond_signal`.
    CondSignal {
        /// In: cond id.
        id: u32,
    },
    /// `sys_cond_signal_all`.
    CondSignalAll {
        /// In: cond id.
        id: u32,
    },
    /// `sys_cond_signal_to`.
    CondSignalTo {
        /// In: cond id.
        id: u32,
        /// In: target thread id.
        target_thread: u32,
    },
    /// `_sys_ppu_thread_create` (syscall 52). `param_ptr` is the
    /// `ppu_thread_param_t` pointer: `{ u32 entry_opd_ptr; u32 tls; }`.
    /// The dispatcher reads `param_ptr` for the OPD pointer, then
    /// reads that OPD's 8 bytes to recover `{ entry_code, entry_toc }`.
    PpuThreadCreate {
        /// Out: thread id.
        id_ptr: u32,
        /// In: `ppu_thread_param_t` pointer.
        param_ptr: u32,
        /// In: thread argument.
        arg: u64,
        /// In: priority. LV2 ABI is signed `int priority`; classify
        /// validates sign extension before constructing the variant.
        priority: i32,
        /// In: stack size in bytes.
        stacksize: u64,
        /// In: creation flags.
        flags: u64,
    },
    /// `sys_rsx_memory_allocate`.
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
    /// `sys_rsx_memory_free`.
    SysRsxMemoryFree {
        /// In: RSX memory handle.
        mem_handle: u32,
    },
    /// `sys_rsx_context_allocate`.
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
    /// `sys_rsx_context_free`.
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
    /// `sys_ss_access_control_engine`. Three useful pkg_ids:
    /// `1` writes the caller's authority-id to `*a3` (debug/root only;
    /// user-perm returns CELL_ENOSYS). `2` writes authority-id to
    /// `*a2` for any caller. `3` is a debug/root no-op. Anything else
    /// is the SS-domain "invalid pkg_id" status.
    SsAccessControlEngine {
        /// In: subcommand selector (r3).
        pkg_id: u64,
        /// In/Out: pkg-dependent pointer; for pkg_id=2 this is the
        /// out-pointer for the 64-bit authority-id.
        a2: u64,
        /// In/Out: pkg-dependent pointer; for pkg_id=1 this is the
        /// out-pointer for the 64-bit authority-id.
        a3: u64,
    },
    /// `sc` with non-zero LEV. PS3 usermode must never issue this;
    /// the runtime rejects rather than letting the call reach LV2
    /// dispatch unflagged.
    Hypercall {
        /// In: privilege level. `NonZeroU8` because LEV=0 is the
        /// normal-syscall path and never produces a `Hypercall`
        /// variant; the type makes that contract unrepresentable.
        lev: NonZeroU8,
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
    /// Fired by the unresolved-import trampoline planted in
    /// unpatched GOT slots. The NID rides in r4 (`args[1]`) and
    /// names the missing import. Dispatcher surfaces a structured
    /// diagnostic and fails the unit.
    UnresolvedImport {
        /// In: NID of the unresolved import.
        nid: u32,
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
