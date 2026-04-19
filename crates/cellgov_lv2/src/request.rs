//! Typed LV2 syscall requests.
//!
//! The PPU's `run_until_yield` packages syscall arguments into one of
//! these variants and yields with `YieldReason::Syscall`. The runtime
//! passes the request to `Lv2Host::dispatch`. The PPU crate does not
//! depend on this crate -- the runtime decodes the raw GPR values into
//! an `Lv2Request` at the boundary.

/// A typed LV2 syscall request.
///
/// Each variant carries the guest-address arguments the PPU placed in
/// GPRs 3..=10 before executing `sc`. All pointer fields are guest
/// effective addresses (u32 on PS3 despite the 64-bit ELF container).
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
        /// Guest address of the attribute struct (opaque, not inspected).
        attr_ptr: u32,
    },
    /// sys_spu_thread_initialize (172).
    /// ABI: r3=thread_ptr, r4=group, r5=spu_num, r6=img_ptr, r7=attr_ptr, r8=arg_ptr
    SpuThreadInitialize {
        /// Guest address to write the allocated thread id into.
        thread_ptr: u32,
        /// Thread group id returned by a previous create call.
        group_id: u32,
        /// Slot index within the group (0-based).
        thread_num: u32,
        /// Guest address of the sys_spu_image_t struct (contains handle).
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
        /// Thread id returned by sysSpuThreadInitialize.
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
        /// Timeout in microseconds (0 = infinite).
        timeout: u64,
    },
    /// sys_mutex_unlock (104).
    MutexUnlock {
        /// Mutex id.
        mutex_id: u32,
    },
    /// sys_mutex_trylock (103). Non-blocking acquire: returns
    /// CELL_OK on success, EBUSY if the mutex is currently owned,
    /// ESRCH for an unknown id. Never parks the caller.
    MutexTryLock {
        /// Mutex id.
        mutex_id: u32,
    },
    /// sys_semaphore_create (93). Allocates a semaphore id,
    /// initializes count = `initial` and max = `max`, writes the
    /// id to `id_ptr`. Returns EINVAL if `initial > max` or either
    /// is negative.
    SemaphoreCreate {
        /// Guest address to receive the minted semaphore id (u32 BE).
        id_ptr: u32,
        /// Guest address of the attribute struct (opaque,
        /// ignored).
        attr_ptr: u32,
        /// Initial resource count.
        initial: i32,
        /// Maximum resource count.
        max: i32,
    },
    /// sys_semaphore_destroy (94). Removes the semaphore table
    /// entry referenced by `id`. Fails with EBUSY if any waiter is
    /// parked.
    SemaphoreDestroy {
        /// Semaphore id to destroy.
        id: u32,
    },
    /// sys_semaphore_wait (114). Decrements the count if > 0;
    /// otherwise parks the caller until a post arrives. Timeout is
    /// captured and ignored; all waits are indefinite.
    SemaphoreWait {
        /// Semaphore id.
        id: u32,
        /// Timeout in microseconds (0 = infinite). Captured and
        /// ignored.
        timeout: u64,
    },
    /// sys_semaphore_post (115). If a waiter is parked, wakes the
    /// head of the waiter list (count unchanged). Otherwise
    /// increments count by `val`. Returns EINVAL if the increment
    /// would push count past max.
    SemaphorePost {
        /// Semaphore id.
        id: u32,
        /// Number of slots to post. Must be positive. Only val ==
        /// 1 is accepted; multi-slot post is deferred because it
        /// complicates the wake protocol (N posts could wake N
        /// waiters in one dispatch).
        val: i32,
    },
    /// sys_semaphore_trywait (116). Non-blocking wait: decrement
    /// and return CELL_OK on success, EBUSY if the count is zero,
    /// ESRCH for an unknown id. Never parks the caller.
    SemaphoreTryWait {
        /// Semaphore id.
        id: u32,
    },
    /// sys_semaphore_get_value (117). Writes the current count to
    /// `out_ptr` as a big-endian u32 and returns CELL_OK.
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
    /// sys_event_queue_receive (130). Pops one payload from the
    /// queue into the caller's `sys_event_t` out buffer and
    /// returns CELL_OK, or parks the caller until a matching
    /// `sys_event_queue_send` delivers a payload. Timeout is
    /// captured and ignored; all waits are indefinite.
    EventQueueReceive {
        /// Queue id.
        queue_id: u32,
        /// Guest address of the `sys_event_t` output buffer (32
        /// bytes: source / data1 / data2 / data3, each big-endian
        /// u64).
        out_ptr: u32,
        /// Timeout in microseconds (0 = infinite). Captured and
        /// ignored.
        timeout: u64,
    },
    /// sys_event_port_send (134). Sends a payload into the queue
    /// bound to `port_id`. The event-port table is not modeled
    /// separately; the handler treats `port_id` as the
    /// destination queue id directly (matches the ABI when 1:1
    /// port-to-queue bindings are used, which is the
    /// overwhelmingly common pattern).
    EventPortSend {
        /// Event port id (treated as queue id).
        port_id: u32,
        /// First payload word (data1).
        data1: u64,
        /// Second payload word (data2).
        data2: u64,
        /// Third payload word (data3).
        data3: u64,
    },
    /// sys_event_flag_create (82). Allocates an event flag id
    /// with the given initial bit state. Protocol and type bits
    /// in the attribute struct are captured and ignored.
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
    /// sys_event_flag_wait (84). Blocks the caller until the flag
    /// bits match `bits` per `mode`. `result_ptr` receives the
    /// observed bit pattern on wake (u64 BE). Timeout captured
    /// and ignored.
    EventFlagWait {
        /// Event flag id.
        id: u32,
        /// Bit mask to match.
        bits: u64,
        /// Wait mode (AND/OR matching, CLEAR/NO-CLEAR on wake).
        /// Encoded as the raw ABI u32 value; the handler maps to
        /// `EventFlagWaitMode`.
        mode: u32,
        /// Guest address to write the observed bit pattern (u64 BE).
        result_ptr: u32,
        /// Timeout in microseconds (0 = infinite). Captured and
        /// ignored.
        timeout: u64,
    },
    /// sys_event_flag_set (86). ORs `bits` into the flag's bit
    /// state and wakes any matching waiters.
    EventFlagSet {
        /// Event flag id.
        id: u32,
        /// Bits to OR into the flag.
        bits: u64,
    },
    /// sys_event_flag_clear (87). AND-NOTs `bits` from the flag's
    /// bit state. Does not wake anyone.
    EventFlagClear {
        /// Event flag id.
        id: u32,
        /// Bits to clear.
        bits: u64,
    },
    /// sys_event_flag_trywait (85). Non-blocking wait: if the
    /// mask matches, apply CLEAR (if mode includes it) and return
    /// CELL_OK; otherwise EBUSY.
    EventFlagTryWait {
        /// Event flag id.
        id: u32,
        /// Bit mask to match.
        bits: u64,
        /// Wait mode (raw ABI u32).
        mode: u32,
        /// Guest address to write the observed bit pattern.
        result_ptr: u32,
    },
    /// sys_event_queue_tryreceive (133). Non-blocking batch pop.
    /// Writes up to `size` payloads to the caller's output array
    /// starting at `event_array`, and writes the actual number
    /// written to `count_out`. Returns CELL_OK (even if zero
    /// payloads were available).
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
        /// Allocation size in bytes (must be aligned to page size).
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
    /// sys_ppu_thread_yield (43). Pure scheduling hint -- the
    /// syscall completes immediately with `CELL_OK` and the
    /// runtime's round-robin scheduler naturally hands control to
    /// the next runnable unit on the next step.
    PpuThreadYield,
    /// sys_ppu_thread_exit (41). The calling PPU thread is done;
    /// the runtime transitions its unit to `Finished` and wakes
    /// any units joining on it with this exit value.
    PpuThreadExit {
        /// Exit value passed to joiners' r3 on wake.
        exit_value: u64,
    },
    /// sys_ppu_thread_join (44). Blocks the caller until `target`
    /// calls `sys_ppu_thread_exit`. On wake the runtime writes the
    /// target's exit value to `status_out_ptr` (u64 big-endian)
    /// and returns CELL_OK.
    PpuThreadJoin {
        /// Guest thread id of the child to join on.
        target: u64,
        /// Guest address to receive the child's exit value on wake.
        status_out_ptr: u32,
    },
    /// sys_lwmutex_create (95). Allocates a fresh lwmutex id and
    /// writes it to `id_ptr`. The attribute bag is captured but
    /// only `name` and `recursion` are surfaced; advanced attributes
    /// are stored-and-ignored.
    LwMutexCreate {
        /// Guest address to receive the minted lwmutex id (u32 BE).
        id_ptr: u32,
        /// Guest address of the attribute struct (opaque, not
        /// inspected at this level).
        attr_ptr: u32,
    },
    /// sys_lwmutex_destroy (96). Removes the lwmutex table entry
    /// referenced by `id`. Fails with EBUSY if any waiter is
    /// parked; the handler validates that before calling the table.
    LwMutexDestroy {
        /// Lwmutex id to destroy.
        id: u32,
    },
    /// sys_lwmutex_lock (97). Acquires the lwmutex `id` for the
    /// caller. If unowned, completes immediately with CELL_OK. If
    /// owned by another thread, blocks the caller until a
    /// subsequent `sys_lwmutex_unlock` transfers ownership.
    /// `timeout` is captured but ignored; all waits are indefinite.
    LwMutexLock {
        /// Lwmutex id.
        id: u32,
        /// Timeout in microseconds (0 = infinite). Captured and
        /// ignored; deferred to post-alpha.
        timeout: u64,
    },
    /// sys_lwmutex_unlock (98). Releases the lwmutex `id` on
    /// behalf of the caller (must be the current owner). If a
    /// waiter is parked, transfers ownership to the head of the
    /// queue and wakes that thread with CELL_OK. Otherwise clears
    /// ownership. Returns EPERM if the caller does not own the
    /// mutex, ESRCH for an unknown id.
    LwMutexUnlock {
        /// Lwmutex id.
        id: u32,
    },
    /// sys_lwmutex_trylock (99). Non-blocking acquire: returns
    /// CELL_OK on success, EBUSY if the mutex is currently owned,
    /// ESRCH for an unknown id. Never parks the caller.
    LwMutexTryLock {
        /// Lwmutex id.
        id: u32,
    },
    /// sys_cond_create (105). Allocates a cond id bound to the
    /// heavy mutex `mutex_id`. The attribute struct is captured
    /// and ignored. Fails with ESRCH if `mutex_id` does not name
    /// an existing heavy mutex.
    CondCreate {
        /// Guest address to receive the minted cond id (u32 BE).
        id_ptr: u32,
        /// Guest id of the associated heavy mutex.
        mutex_id: u32,
        /// Guest address of the attribute struct (opaque).
        attr_ptr: u32,
    },
    /// sys_cond_destroy (106). Removes the cond table entry
    /// referenced by `id`. Fails with EBUSY if any waiter is
    /// parked.
    CondDestroy {
        /// Cond id to destroy.
        id: u32,
    },
    /// sys_cond_wait (107). Releases the associated mutex (as if
    /// the caller had called sys_mutex_unlock, including waking
    /// any mutex waiter) and parks the caller until a matching
    /// sys_cond_signal / _signal_all / _signal_to wakes it. On
    /// wake, the caller re-acquires the mutex (immediately if
    /// free, otherwise re-parks on the mutex waiter list).
    /// Timeout is captured and ignored.
    CondWait {
        /// Cond id.
        id: u32,
        /// Timeout in microseconds (0 = infinite). Captured and
        /// ignored.
        timeout: u64,
    },
    /// sys_cond_signal (108). Wakes the head of the cond's waiter
    /// list (non-sticky: a signal with no waiters is observably
    /// lost). The woken thread re-acquires the associated mutex
    /// before returning.
    CondSignal {
        /// Cond id.
        id: u32,
    },
    /// sys_cond_signal_all (109). Wakes all cond waiters; each
    /// transitions independently through the mutex re-acquire
    /// path. Non-sticky.
    CondSignalAll {
        /// Cond id.
        id: u32,
    },
    /// sys_cond_signal_to (110). Wakes a specific thread parked
    /// on the cond. Non-sticky; fails with ESRCH if the target is
    /// not parked on this cond.
    CondSignalTo {
        /// Cond id.
        id: u32,
        /// Guest PPU thread id of the target.
        target_thread: u32,
    },
    /// sys_ppu_thread_create (52). Spawns a new PPU thread and
    /// writes its guest-facing id to `id_ptr`.
    PpuThreadCreate {
        /// Guest address to receive the minted thread id (u64 BE).
        id_ptr: u32,
        /// OPD address of the entry function. The handler reads
        /// the first 8 bytes to get the code address and the next
        /// 8 bytes for the TOC.
        entry_opd: u32,
        /// Argument passed as the child's r3 on first execution.
        arg: u64,
        /// Priority. Captured from the guest but not consulted
        /// by the current round-robin scheduler.
        priority: u32,
        /// Requested child stack size in bytes.
        stacksize: u64,
        /// Flags (captured but not interpreted at this level).
        flags: u64,
    },
    /// A syscall number that does not map to any known request.
    Unsupported {
        /// The raw syscall number from GPR 11.
        number: u64,
    },
}

/// Build an `Lv2Request` from the raw syscall number and GPR values.
///
/// The PPU places the syscall number in r11 and up to 8 arguments in
/// r3..=r10. This function maps the number to the typed request,
/// extracting the relevant arguments. Unknown syscalls produce
/// `Lv2Request::Unsupported`.
pub fn classify(syscall_num: u64, args: &[u64; 8]) -> Lv2Request {
    match syscall_num {
        156 => Lv2Request::SpuImageOpen {
            img_ptr: args[0] as u32,
            path_ptr: args[1] as u32,
        },
        170 => Lv2Request::SpuThreadGroupCreate {
            id_ptr: args[0] as u32,
            num_threads: args[1] as u32,
            priority: args[2] as u32,
            attr_ptr: args[3] as u32,
        },
        172 => Lv2Request::SpuThreadInitialize {
            thread_ptr: args[0] as u32,
            group_id: args[1] as u32,
            thread_num: args[2] as u32,
            img_ptr: args[3] as u32,
            attr_ptr: args[4] as u32,
            arg_ptr: args[5] as u32,
        },
        173 => Lv2Request::SpuThreadGroupStart {
            group_id: args[0] as u32,
        },
        177 | 178 => Lv2Request::SpuThreadGroupJoin {
            group_id: args[0] as u32,
            cause_ptr: args[1] as u32,
            status_ptr: args[2] as u32,
        },
        190 => Lv2Request::SpuThreadWriteMb {
            thread_id: args[0] as u32,
            value: args[1] as u32,
        },
        403 => Lv2Request::TtyWrite {
            fd: args[0] as u32,
            buf_ptr: args[1] as u32,
            len: args[2] as u32,
            nwritten_ptr: args[3] as u32,
        },
        22 => Lv2Request::ProcessExit {
            code: args[0] as u32,
        },
        43 => Lv2Request::PpuThreadYield,
        41 => Lv2Request::PpuThreadExit {
            exit_value: args[0],
        },
        52 => Lv2Request::PpuThreadCreate {
            id_ptr: args[0] as u32,
            entry_opd: args[1] as u32,
            arg: args[2],
            priority: args[3] as u32,
            stacksize: args[4],
            flags: args[5],
        },
        44 => Lv2Request::PpuThreadJoin {
            target: args[0],
            status_out_ptr: args[1] as u32,
        },
        95 => Lv2Request::LwMutexCreate {
            id_ptr: args[0] as u32,
            attr_ptr: args[1] as u32,
        },
        96 => Lv2Request::LwMutexDestroy { id: args[0] as u32 },
        97 => Lv2Request::LwMutexLock {
            id: args[0] as u32,
            timeout: args[1],
        },
        98 => Lv2Request::LwMutexUnlock { id: args[0] as u32 },
        99 => Lv2Request::LwMutexTryLock { id: args[0] as u32 },
        100 => Lv2Request::MutexCreate {
            id_ptr: args[0] as u32,
            attr_ptr: args[1] as u32,
        },
        102 => Lv2Request::MutexLock {
            mutex_id: args[0] as u32,
            timeout: args[1],
        },
        104 => Lv2Request::MutexUnlock {
            mutex_id: args[0] as u32,
        },
        103 => Lv2Request::MutexTryLock {
            mutex_id: args[0] as u32,
        },
        93 => Lv2Request::SemaphoreCreate {
            id_ptr: args[0] as u32,
            attr_ptr: args[1] as u32,
            initial: args[2] as i32,
            max: args[3] as i32,
        },
        94 => Lv2Request::SemaphoreDestroy { id: args[0] as u32 },
        114 => Lv2Request::SemaphoreWait {
            id: args[0] as u32,
            timeout: args[1],
        },
        115 => Lv2Request::SemaphorePost {
            id: args[0] as u32,
            val: args[1] as i32,
        },
        116 => Lv2Request::SemaphoreTryWait { id: args[0] as u32 },
        117 => Lv2Request::SemaphoreGetValue {
            id: args[0] as u32,
            out_ptr: args[1] as u32,
        },
        128 => Lv2Request::EventQueueCreate {
            id_ptr: args[0] as u32,
            attr_ptr: args[1] as u32,
            key: args[2],
            size: args[3] as u32,
        },
        129 => Lv2Request::EventQueueDestroy {
            queue_id: args[0] as u32,
        },
        130 => Lv2Request::EventQueueReceive {
            queue_id: args[0] as u32,
            out_ptr: args[1] as u32,
            timeout: args[2],
        },
        82 => Lv2Request::EventFlagCreate {
            id_ptr: args[0] as u32,
            attr_ptr: args[1] as u32,
            init: args[2],
        },
        83 => Lv2Request::EventFlagDestroy { id: args[0] as u32 },
        84 => Lv2Request::EventFlagWait {
            id: args[0] as u32,
            bits: args[1],
            mode: args[2] as u32,
            result_ptr: args[3] as u32,
            timeout: args[4],
        },
        85 => Lv2Request::EventFlagTryWait {
            id: args[0] as u32,
            bits: args[1],
            mode: args[2] as u32,
            result_ptr: args[3] as u32,
        },
        86 => Lv2Request::EventFlagSet {
            id: args[0] as u32,
            bits: args[1],
        },
        87 => Lv2Request::EventFlagClear {
            id: args[0] as u32,
            bits: args[1],
        },
        133 => Lv2Request::EventQueueTryReceive {
            queue_id: args[0] as u32,
            event_array: args[1] as u32,
            size: args[2] as u32,
            count_out: args[3] as u32,
        },
        134 => Lv2Request::EventPortSend {
            port_id: args[0] as u32,
            data1: args[1],
            data2: args[2],
            data3: args[3],
        },
        105 => Lv2Request::CondCreate {
            id_ptr: args[0] as u32,
            mutex_id: args[1] as u32,
            attr_ptr: args[2] as u32,
        },
        106 => Lv2Request::CondDestroy { id: args[0] as u32 },
        107 => Lv2Request::CondWait {
            id: args[0] as u32,
            timeout: args[1],
        },
        108 => Lv2Request::CondSignal { id: args[0] as u32 },
        109 => Lv2Request::CondSignalAll { id: args[0] as u32 },
        110 => Lv2Request::CondSignalTo {
            id: args[0] as u32,
            target_thread: args[1] as u32,
        },
        348 => Lv2Request::MemoryAllocate {
            size: args[0],
            flags: args[1],
            alloc_addr_ptr: args[2] as u32,
        },
        349 => Lv2Request::MemoryFree {
            addr: args[0] as u32,
        },
        352 => Lv2Request::MemoryGetUserMemorySize {
            mem_info_ptr: args[0] as u32,
        },
        341 => Lv2Request::MemoryContainerCreate {
            cid_ptr: args[0] as u32,
            size: args[1],
        },
        n => Lv2Request::Unsupported { number: n },
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
        // syscall 43 carries no arguments and maps to the
        // argument-free PpuThreadYield variant regardless of what
        // the registers happen to hold.
        let args = [0xDEAD, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(classify(43, &args), Lv2Request::PpuThreadYield);
    }

    #[test]
    fn classify_ppu_thread_exit_captures_exit_value() {
        // syscall 41's single argument is the exit value.
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
        // syscall 44 carries the target thread id in r3 and the
        // status output pointer in r4.
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
        // syscall 52's six arguments: id_ptr, entry_opd, arg,
        // priority, stacksize, flags. Verifies each lands in the
        // correct slot.
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
        let args = [0; 8];
        let req = classify(999, &args);
        assert_eq!(req, Lv2Request::Unsupported { number: 999 });
    }

    #[test]
    fn spu_thread_group_range_stubs_classify_as_unsupported() {
        let args = [0; 8];
        for n in [171, 174, 175, 176, 179, 180, 192] {
            let req = classify(n, &args);
            assert!(
                matches!(req, Lv2Request::Unsupported { .. }),
                "syscall {n} should be Unsupported"
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
}
