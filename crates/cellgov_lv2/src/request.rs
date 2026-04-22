//! Typed LV2 syscall requests produced by the PPU at `sc` yield.
//!
//! The runtime decodes r3..=r10 / r11 into one of these variants
//! and hands it to `Lv2Host::dispatch`. Actual syscall semantics
//! live in the host dispatch handlers; this module owns only the
//! request vocabulary.
//!
//! # Escape hatches
//! `classify` is total: an unrecognised number surfaces as
//! [`Lv2Request::Unsupported`], and a recognised number whose u32
//! or i32 arguments are out of ABI range surfaces as
//! [`Lv2Request::Malformed`]. Both carry the raw GPR values so the
//! dispatcher log can show what the caller attempted.

/// A typed LV2 syscall request.
///
/// Pointer fields are guest effective addresses (u32 on PS3
/// despite the 64-bit ELF container). `classify` rejects any
/// u32-typed field whose source GPR has non-zero high bits
/// rather than silently truncating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2Request {
    /// sys_spu_image_open (156).
    SpuImageOpen {
        /// Guest address to populate with the `sys_spu_image_t` struct.
        img_ptr: u32,
        /// Guest address of the NUL-terminated path string.
        path_ptr: u32,
    },
    /// sys_spu_thread_group_create (170).
    SpuThreadGroupCreate {
        /// Guest address to write the allocated group id into.
        id_ptr: u32,
        /// Number of SPU threads in the group.
        num_threads: u32,
        /// Priority (not used by CellGov).
        priority: u32,
        /// Guest address of the attribute struct (opaque).
        attr_ptr: u32,
    },
    /// sys_spu_thread_initialize (172).
    ///
    /// ABI: r3=thread_ptr, r4=group, r5=spu_num, r6=img_ptr,
    /// r7=attr_ptr, r8=arg_ptr.
    SpuThreadInitialize {
        /// Guest address to write the allocated thread id into.
        thread_ptr: u32,
        /// Thread group id returned by a previous create call.
        group_id: u32,
        /// Slot index within the group (0-based).
        thread_num: u32,
        /// Guest address of the sys_spu_image_t struct.
        img_ptr: u32,
        /// Guest address of the attribute struct (opaque).
        attr_ptr: u32,
        /// Guest address of `sys_spu_thread_argument`.
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
        /// Guest address to write the exit cause into.
        cause_ptr: u32,
        /// Guest address to write the exit status into.
        status_ptr: u32,
    },
    /// sys_spu_thread_group_terminate (178).
    ///
    /// Distinct from [`Self::SpuThreadGroupJoin`] so the dispatch
    /// cannot conflate the two ABI shapes (177 has two out-pointers;
    /// 178 takes an in-param status).
    SpuThreadGroupTerminate {
        /// Thread group id.
        group_id: u32,
        /// Termination status delivered to any subsequent joiner.
        value: i32,
    },
    /// sys_tty_write (403).
    TtyWrite {
        /// File descriptor (typically 0 for stdout).
        fd: u32,
        /// Guest address of the buffer to write.
        buf_ptr: u32,
        /// Number of bytes to write.
        len: u32,
        /// Guest address to store the number of bytes written.
        nwritten_ptr: u32,
    },
    /// sys_spu_thread_write_spu_mb (190).
    SpuThreadWriteMb {
        /// SPU thread id.
        thread_id: u32,
        /// Value to deposit into the SPU's inbound mailbox.
        value: u32,
    },
    /// sys_mutex_create (100).
    MutexCreate {
        /// Guest address to write the allocated mutex id into.
        id_ptr: u32,
        /// Guest address of the attribute struct (opaque).
        attr_ptr: u32,
    },
    /// sys_mutex_lock (102).
    MutexLock {
        /// Mutex id.
        mutex_id: u32,
        /// Timeout in microseconds (0 = infinite). Captured and ignored.
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
        /// Guest address to receive the minted semaphore id (u32 BE).
        id_ptr: u32,
        /// Guest address of the attribute struct (opaque).
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
        /// Timeout in microseconds (0 = infinite). Captured and ignored.
        timeout: u64,
    },
    /// sys_semaphore_post (115).
    SemaphorePost {
        /// Semaphore id.
        id: u32,
        /// Number of slots to post. Only `val == 1` is accepted.
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
        /// Guest address to receive the count (u32 BE).
        out_ptr: u32,
    },
    /// sys_event_queue_create (128).
    EventQueueCreate {
        /// Guest address to write the allocated queue id into.
        id_ptr: u32,
        /// Guest address of the attribute struct (opaque).
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
        /// Guest address of the `sys_event_t` output buffer (32
        /// bytes: source / data1 / data2 / data3, each u64 BE).
        out_ptr: u32,
        /// Timeout in microseconds (0 = infinite). Captured and ignored.
        timeout: u64,
    },
    /// sys_event_port_send (134).
    ///
    /// The dispatcher looks up the port and validates the binding;
    /// a port with no binding or a non-1:1 binding routes to ESRCH
    /// rather than being silently mis-delivered.
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
        /// Guest address to receive the minted id (u32 BE).
        id_ptr: u32,
        /// Guest address of the attribute struct (opaque).
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
        /// Raw ABI wait-mode word; the handler maps to `EventFlagWaitMode`.
        mode: u32,
        /// Guest address to write the observed bit pattern (u64 BE).
        result_ptr: u32,
        /// Timeout in microseconds (0 = infinite). Captured and ignored.
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
        /// Guest address to write the observed bit pattern.
        result_ptr: u32,
    },
    /// sys_event_queue_tryreceive (133).
    EventQueueTryReceive {
        /// Queue id.
        queue_id: u32,
        /// Guest address of the output array (32 bytes per entry).
        event_array: u32,
        /// Maximum number of entries to write.
        size: u32,
        /// Guest address to receive the actual count (u32 BE).
        count_out: u32,
    },
    /// sys_memory_allocate (348).
    MemoryAllocate {
        /// Allocation size in bytes.
        size: u64,
        /// Page size flags: 0x400 = 1MB, 0x200 = 64KB, 0 = 1MB default.
        flags: u64,
        /// Guest address to write the allocated address into.
        alloc_addr_ptr: u32,
    },
    /// sys_memory_free (349).
    MemoryFree {
        /// Guest address to free.
        addr: u32,
    },
    /// sys_memory_get_user_memory_size (352).
    MemoryGetUserMemorySize {
        /// Guest address of `sys_memory_info_t` output struct.
        mem_info_ptr: u32,
    },
    /// sys_memory_container_create (341).
    MemoryContainerCreate {
        /// Guest address to write the allocated container id into.
        cid_ptr: u32,
        /// Container size in bytes.
        size: u64,
    },
    /// sys_process_exit (22).
    ProcessExit {
        /// Exit code.
        code: u32,
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
        /// Guest thread id of the child to join on.
        target: u64,
        /// Guest address to receive the child's exit value (u64 BE).
        status_out_ptr: u32,
    },
    /// sys_lwmutex_create (95).
    LwMutexCreate {
        /// Guest address to receive the minted lwmutex id (u32 BE).
        id_ptr: u32,
        /// Guest address of the attribute struct (opaque).
        attr_ptr: u32,
    },
    /// sys_lwmutex_destroy (96).
    LwMutexDestroy {
        /// Lwmutex id.
        id: u32,
    },
    /// sys_lwmutex_lock (97).
    LwMutexLock {
        /// Lwmutex id.
        id: u32,
        /// Timeout in microseconds (0 = infinite). Captured and ignored.
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
    /// sys_cond_create (105).
    CondCreate {
        /// Guest address to receive the minted cond id (u32 BE).
        id_ptr: u32,
        /// Guest id of the associated heavy mutex.
        mutex_id: u32,
        /// Guest address of the attribute struct (opaque).
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
        /// Timeout in microseconds (0 = infinite). Captured and ignored.
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
        /// Guest PPU thread id of the target.
        target_thread: u32,
    },
    /// sys_ppu_thread_create (52).
    PpuThreadCreate {
        /// Guest address to receive the minted thread id (u64 BE).
        id_ptr: u32,
        /// OPD address: first 8 bytes code, next 8 bytes TOC.
        entry_opd: u32,
        /// Argument passed as the child's r3 on first execution.
        arg: u64,
        /// Priority (captured but not consulted by the scheduler).
        priority: u32,
        /// Requested child stack size in bytes.
        stacksize: u64,
        /// Flags (captured but not interpreted).
        flags: u64,
    },
    /// A syscall number that does not map to any known request.
    Unsupported {
        /// The raw syscall number from GPR 11.
        number: u64,
        /// Raw GPR values from r3..=r10.
        args: [u64; 8],
    },
    /// A recognised syscall whose arguments are out of ABI range.
    ///
    /// A u32-typed field arriving with non-zero high bits, or an
    /// i32-typed field that is not a clean sign extension (PPC64
    /// promotes `int x = -1` to `0xFFFF_FFFF_FFFF_FFFF`, which
    /// decodes to `-1`; `0x1_0000_0001` is neither a zero-extended
    /// u32 nor a clean sign extension and is rejected). The
    /// dispatcher routes this to CELL_EINVAL.
    Malformed {
        /// The raw syscall number.
        number: u64,
        /// Short description of which field failed to decode.
        reason: &'static str,
        /// Raw GPR values from r3..=r10.
        args: [u64; 8],
    },
}

/// Per-arg reason strings used by [`classify`] on u32 high-bit
/// rejection.
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

/// Per-arg reason strings used by [`classify`] on i32
/// out-of-range rejection.
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

/// Build an `Lv2Request` from the raw syscall number (r11) and
/// argument GPRs (r3..=r10).
///
/// Unknown numbers produce [`Lv2Request::Unsupported`]; recognised
/// numbers whose u32 or i32 arguments are out of ABI range produce
/// [`Lv2Request::Malformed`]. Both variants preserve the raw args.
pub fn classify(syscall_num: u64, args: &[u64; 8]) -> Lv2Request {
    // `s!(i)` relies on `args[i] as i64` to reverse PPC64's sign
    // extension: a guest-side `int x = -1` arrives in the GPR as
    // 0xFFFF_FFFF_FFFF_FFFF, which `as i64` turns into -1i64, and
    // `i32::try_from` then rejects anything that isn't a clean
    // sign extension (e.g. 0x1_0000_0001, or 2^31).
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
        156 => Lv2Request::SpuImageOpen {
            img_ptr: p!(0),
            path_ptr: p!(1),
        },
        170 => Lv2Request::SpuThreadGroupCreate {
            id_ptr: p!(0),
            num_threads: p!(1),
            priority: p!(2),
            attr_ptr: p!(3),
        },
        172 => Lv2Request::SpuThreadInitialize {
            thread_ptr: p!(0),
            group_id: p!(1),
            thread_num: p!(2),
            img_ptr: p!(3),
            attr_ptr: p!(4),
            arg_ptr: p!(5),
        },
        173 => Lv2Request::SpuThreadGroupStart { group_id: p!(0) },
        177 => Lv2Request::SpuThreadGroupJoin {
            group_id: p!(0),
            cause_ptr: p!(1),
            status_ptr: p!(2),
        },
        178 => Lv2Request::SpuThreadGroupTerminate {
            group_id: p!(0),
            value: s!(1),
        },
        190 => Lv2Request::SpuThreadWriteMb {
            thread_id: p!(0),
            value: p!(1),
        },
        403 => Lv2Request::TtyWrite {
            fd: p!(0),
            buf_ptr: p!(1),
            len: p!(2),
            nwritten_ptr: p!(3),
        },
        22 => Lv2Request::ProcessExit { code: p!(0) },
        43 => Lv2Request::PpuThreadYield,
        41 => Lv2Request::PpuThreadExit {
            exit_value: args[0],
        },
        52 => Lv2Request::PpuThreadCreate {
            id_ptr: p!(0),
            entry_opd: p!(1),
            arg: args[2],
            priority: p!(3),
            stacksize: args[4],
            flags: args[5],
        },
        44 => Lv2Request::PpuThreadJoin {
            target: args[0],
            status_out_ptr: p!(1),
        },
        95 => Lv2Request::LwMutexCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
        },
        96 => Lv2Request::LwMutexDestroy { id: p!(0) },
        97 => Lv2Request::LwMutexLock {
            id: p!(0),
            timeout: args[1],
        },
        98 => Lv2Request::LwMutexUnlock { id: p!(0) },
        99 => Lv2Request::LwMutexTryLock { id: p!(0) },
        100 => Lv2Request::MutexCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
        },
        102 => Lv2Request::MutexLock {
            mutex_id: p!(0),
            timeout: args[1],
        },
        104 => Lv2Request::MutexUnlock { mutex_id: p!(0) },
        103 => Lv2Request::MutexTryLock { mutex_id: p!(0) },
        93 => Lv2Request::SemaphoreCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
            initial: s!(2),
            max: s!(3),
        },
        94 => Lv2Request::SemaphoreDestroy { id: p!(0) },
        114 => Lv2Request::SemaphoreWait {
            id: p!(0),
            timeout: args[1],
        },
        115 => Lv2Request::SemaphorePost {
            id: p!(0),
            val: s!(1),
        },
        116 => Lv2Request::SemaphoreTryWait { id: p!(0) },
        117 => Lv2Request::SemaphoreGetValue {
            id: p!(0),
            out_ptr: p!(1),
        },
        128 => Lv2Request::EventQueueCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
            key: args[2],
            size: p!(3),
        },
        129 => Lv2Request::EventQueueDestroy { queue_id: p!(0) },
        130 => Lv2Request::EventQueueReceive {
            queue_id: p!(0),
            out_ptr: p!(1),
            timeout: args[2],
        },
        82 => Lv2Request::EventFlagCreate {
            id_ptr: p!(0),
            attr_ptr: p!(1),
            init: args[2],
        },
        83 => Lv2Request::EventFlagDestroy { id: p!(0) },
        84 => Lv2Request::EventFlagWait {
            id: p!(0),
            bits: args[1],
            mode: p!(2),
            result_ptr: p!(3),
            timeout: args[4],
        },
        85 => Lv2Request::EventFlagTryWait {
            id: p!(0),
            bits: args[1],
            mode: p!(2),
            result_ptr: p!(3),
        },
        86 => Lv2Request::EventFlagSet {
            id: p!(0),
            bits: args[1],
        },
        87 => Lv2Request::EventFlagClear {
            id: p!(0),
            bits: args[1],
        },
        133 => Lv2Request::EventQueueTryReceive {
            queue_id: p!(0),
            event_array: p!(1),
            size: p!(2),
            count_out: p!(3),
        },
        134 => Lv2Request::EventPortSend {
            port_id: p!(0),
            data1: args[1],
            data2: args[2],
            data3: args[3],
        },
        105 => Lv2Request::CondCreate {
            id_ptr: p!(0),
            mutex_id: p!(1),
            attr_ptr: p!(2),
        },
        106 => Lv2Request::CondDestroy { id: p!(0) },
        107 => Lv2Request::CondWait {
            id: p!(0),
            timeout: args[1],
        },
        108 => Lv2Request::CondSignal { id: p!(0) },
        109 => Lv2Request::CondSignalAll { id: p!(0) },
        110 => Lv2Request::CondSignalTo {
            id: p!(0),
            target_thread: p!(1),
        },
        348 => Lv2Request::MemoryAllocate {
            size: args[0],
            flags: args[1],
            alloc_addr_ptr: p!(2),
        },
        349 => Lv2Request::MemoryFree { addr: p!(0) },
        352 => Lv2Request::MemoryGetUserMemorySize {
            mem_info_ptr: p!(0),
        },
        341 => Lv2Request::MemoryContainerCreate {
            cid_ptr: p!(0),
            size: args[1],
        },
        // Explicit stub list: known shapes whose effects are not
        // yet modelled. Listed here so adding a handler later
        // forces a review rather than silently falling through
        // the default arm.
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
        let req = classify(177, &args);
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
        // Regression fence: 177 and 178 have different ABI
        // shapes; conflating them routes r4 through the wrong
        // interpretation.
        let args = [3, 0xFFFF_FFFF_FFFF_FFFF, 0, 0, 0, 0, 0, 0];
        let req = classify(178, &args);
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
            classify(134, &[7, 0xaa, 0xbb, 0xcc, 0, 0, 0, 0]),
            Lv2Request::EventPortSend {
                port_id: 7,
                data1: 0xaa,
                data2: 0xbb,
                data3: 0xcc,
            }
        );
        assert_eq!(
            classify(133, &[7, 0x2000, 4, 0x3000, 0, 0, 0, 0]),
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
            classify(116, &[7, 0, 0, 0, 0, 0, 0, 0]),
            Lv2Request::SemaphoreTryWait { id: 7 }
        );
        assert_eq!(
            classify(117, &[7, 0x1000, 0, 0, 0, 0, 0, 0]),
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
            classify(115, &args),
            Lv2Request::SemaphorePost { id: 7, val: 1 }
        );
    }

    #[test]
    fn classify_semaphore_create_destroy_wait() {
        let create_args = [0x5000, 0x6000, 2, 10, 0, 0, 0, 0];
        assert_eq!(
            classify(93, &create_args),
            Lv2Request::SemaphoreCreate {
                id_ptr: 0x5000,
                attr_ptr: 0x6000,
                initial: 2,
                max: 10,
            }
        );
        let destroy_args = [7, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(94, &destroy_args),
            Lv2Request::SemaphoreDestroy { id: 7 }
        );
        let wait_args = [7, 100, 0, 0, 0, 0, 0, 0];
        assert_eq!(
            classify(114, &wait_args),
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
        // PPC64 sign-extends negative ints; -1 must decode to
        // i32 -1, not be rejected. The handler's existing
        // "negative -> EINVAL" logic is the final gate.
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
            classify(93, &args),
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
        match classify(93, &args) {
            Lv2Request::Malformed { number, reason, .. } => {
                assert_eq!(number, 93);
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
        // 2^31 fits in u32 but not in i32; the old cast wrapped
        // to i32::MIN.
        let args = [0x5000, 0x6000, 0x8000_0000, 10, 0, 0, 0, 0];
        assert!(matches!(
            classify(93, &args),
            Lv2Request::Malformed { number: 93, .. }
        ));
    }

    /// Every syscall whose decode uses `p!(N)` for a u32 field,
    /// listed by the GPR slot indices that must reject high bits.
    /// Regression fence: catches `args[N] as u32` added by muscle
    /// memory instead of `p!(N)` in a new syscall arm.
    const U32_SLOTS_BY_SYSCALL: &[(u64, &[usize])] = &[
        (22, &[0]),
        (44, &[1]),
        (52, &[0, 1, 3]),
        (82, &[0, 1]),
        (83, &[0]),
        (84, &[0, 2, 3]),
        (85, &[0, 2, 3]),
        (86, &[0]),
        (87, &[0]),
        (93, &[0, 1]),
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
        (114, &[0]),
        (115, &[0]),
        (116, &[0]),
        (117, &[0, 1]),
        (128, &[0, 1, 3]),
        (129, &[0]),
        (130, &[0, 1]),
        (133, &[0, 1, 2, 3]),
        (134, &[0]),
        (156, &[0, 1]),
        (170, &[0, 1, 2, 3]),
        (172, &[0, 1, 2, 3, 4, 5]),
        (173, &[0]),
        (177, &[0, 1, 2]),
        (178, &[0]),
        (190, &[0, 1]),
        (341, &[0]),
        (348, &[2]),
        (349, &[0]),
        (352, &[0]),
        (403, &[0, 1, 2, 3]),
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
            classify(115, &ok),
            Lv2Request::SemaphorePost { id: 7, val: -1 }
        );
        let bad = [7u64, 0x1_0000_0001, 0, 0, 0, 0, 0, 0];
        assert!(matches!(
            classify(115, &bad),
            Lv2Request::Malformed { number: 115, .. }
        ));
    }
}
