//! Dispatch result types returned by `Lv2Host::dispatch`.
//!
//! `Lv2Dispatch` tells the runtime what to do with the syscall: complete
//! it immediately, register a new SPU, or block the caller. Every
//! variant carries plain data -- no closures, no runtime references.

use cellgov_effects::Effect;
use cellgov_event::UnitId;

/// How the runtime should complete a dispatched syscall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lv2Dispatch {
    /// Immediate completion. The runtime writes `code` into the PPU's
    /// r3, advances PC past the `sc`, and commits any effects.
    Immediate {
        /// Return code for r3 (0 = CELL_OK).
        code: u64,
        /// Effects the host wants the runtime to commit.
        effects: Vec<Effect>,
    },
    /// The host asks the runtime to construct and register SPUs.
    /// One entry per slot in the thread group, in deterministic slot
    /// order. The runtime constructs and registers each.
    RegisterSpu {
        /// Initialization state for each SPU to create, in slot order.
        inits: Vec<SpuInitState>,
        /// Effects to commit alongside the registration.
        effects: Vec<Effect>,
        /// Return code for r3.
        code: u64,
    },
    /// The host wants the caller to block until a condition resolves.
    Block {
        /// Why the caller is blocking.
        reason: Lv2BlockReason,
        /// What the runtime should do when the block resolves.
        pending: PendingResponse,
        /// Effects to commit before blocking.
        effects: Vec<Effect>,
    },
    /// The calling PPU thread invoked `sys_ppu_thread_exit`. The
    /// runtime transitions the source unit to
    /// `UnitStatus::Finished` and wakes each unit in
    /// `woken_unit_ids` with `exit_value` as its r3. No return
    /// value is written for the source (the thread is gone).
    PpuThreadExit {
        /// Exit value the thread passed to `sys_ppu_thread_exit`.
        /// Propagated to joiners' r3 when they wake.
        exit_value: u64,
        /// Unit ids of blocked joiners to transition back to
        /// `Runnable`. Empty if no one is currently joining.
        woken_unit_ids: Vec<UnitId>,
        /// Effects to commit alongside the exit transition.
        effects: Vec<Effect>,
    },
    /// Immediate completion that also wakes one or more parked
    /// PPU-thread waiters.
    ///
    /// Emitted by the release side of a synchronization primitive
    /// (lwmutex unlock, mutex unlock, semaphore post, etc.). The
    /// caller's syscall returns `code` (typically CELL_OK) with
    /// the unit staying `Runnable`. Each unit in `woken_unit_ids`
    /// has its `PendingResponse` consumed from the syscall-response
    /// table and its status transitioned from `Blocked` to
    /// `Runnable`. Supported `PendingResponse` variants at wake:
    ///
    ///   * `ReturnCode { code }` -- set r3 to `code`. Used by
    ///     lwmutex / mutex / semaphore.
    ///   * `EventQueueReceive { .. }` -- write the 32-byte payload
    ///     to the caller's out pointer (via SharedWriteIntent in
    ///     `effects`) and set r3 = 0. Used by event queue.
    ///   * `CondWakeReacquire { .. }` -- handled in a later phase;
    ///     transitions the caller to `WaitingOnMutex` rather than
    ///     waking cleanly.
    WakeAndReturn {
        /// Return code for the release caller's r3. Typically 0
        /// (CELL_OK).
        code: u64,
        /// PPU units to wake. Their pending responses determine
        /// what r3 and out-pointer writes the runtime emits.
        woken_unit_ids: Vec<UnitId>,
        /// Per-waiter `PendingResponse` overrides applied before
        /// the wake resolves.
        ///
        /// Used by primitives whose wake payload is not known
        /// until the release-side dispatch. `sys_event_queue_send`
        /// is the primary case: the receiver's
        /// `PendingResponse::EventQueueReceive` was installed at
        /// wait time with placeholder zero data; send fills in
        /// the real `source / data1 / data2 / data3` here and the
        /// runtime replays those values to the waiter's out
        /// pointer via the normal wake path. Empty for primitives
        /// whose pending response is complete at wait time
        /// (lwmutex / mutex / semaphore all use
        /// `PendingResponse::ReturnCode { code: 0 }`, which needs
        /// no update).
        response_updates: Vec<(UnitId, PendingResponse)>,
        /// Effects to commit alongside the wake transitions.
        effects: Vec<Effect>,
    },
    /// The caller is blocking AND one or more other units are
    /// waking in the same dispatch.
    ///
    /// Emitted by `sys_cond_wait`: releasing the associated mutex
    /// on the way into the cond wait can transfer ownership to a
    /// parked mutex waiter, which must wake in the same step the
    /// cond caller blocks. This is the one sync primitive that
    /// combines a source-side block with a side-effect wake.
    ///
    /// The runtime:
    ///   * Applies `effects`.
    ///   * Applies `response_updates` to the pending-response
    ///     table (typically empty for cond-wait; waiters parked on
    ///     the released mutex already hold
    ///     `PendingResponse::ReturnCode { code: 0 }`).
    ///   * Resolves each unit in `woken_unit_ids` via the normal
    ///     wake path (take pending, write r3 / out pointers,
    ///     transition to `Runnable`).
    ///   * Inserts `pending` as the source unit's pending response
    ///     and transitions the source from `Runnable` to
    ///     `Blocked`.
    ///
    /// The source's r3 is NOT set here; the eventual wake that
    /// resolves `pending` writes r3 at that point.
    BlockAndWake {
        /// Why the caller is blocking.
        reason: Lv2BlockReason,
        /// What the runtime should do when the source's block
        /// resolves.
        pending: PendingResponse,
        /// Units to wake alongside the source's block.
        woken_unit_ids: Vec<UnitId>,
        /// Per-waiter response overrides applied before resolving
        /// the woken set.
        response_updates: Vec<(UnitId, PendingResponse)>,
        /// Effects to commit alongside the transitions.
        effects: Vec<Effect>,
    },
    /// The calling thread invoked `sys_ppu_thread_create`. The
    /// runtime allocates a fresh `PpuExecutionUnit`, seeds it per
    /// the PPC64 ABI (PC, TOC, r1, r3, r13, LR sentinel), inserts
    /// it into the `PpuThreadTable` via the provided `attrs`, and
    /// writes the minted thread id into `id_ptr`. Return code for
    /// the caller is CELL_OK (0).
    PpuThreadCreate {
        /// Guest address to write the minted thread id (u64 BE).
        id_ptr: u32,
        /// OPD address of the entry function. Runtime resolves
        /// `code_addr` and `toc` by reading 16 bytes at this
        /// address from guest memory.
        entry_opd: u32,
        /// Stack top (value to load into the child's r1 register).
        stack_top: u64,
        /// Child stack block base (lowest address of the reserved
        /// range). Recorded in thread attrs.
        stack_base: u64,
        /// Child stack size.
        stack_size: u64,
        /// Argument value for the child's r3 register.
        arg: u64,
        /// Base address of the child's per-thread TLS block (0
        /// when the ELF has no PT_TLS segment). Runtime loads
        /// this into r13.
        tls_base: u64,
        /// Initial bytes to commit at `tls_base` before the child
        /// starts. Empty when there is no TLS.
        tls_bytes: Vec<u8>,
        /// Priority hint captured in thread attrs.
        priority: u32,
        /// Effects to commit alongside the create transition
        /// (e.g. the id_ptr write).
        effects: Vec<Effect>,
    },
}

/// A stable, deterministic, host-side token identifying a loaded SPU
/// image. Allocated by a monotonic counter, starting at 1 (0 is
/// reserved as "no image"). Not a pointer, not an index into a Vec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpuImageHandle(u32);

impl SpuImageHandle {
    /// Wrap a raw handle value.
    #[inline]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw handle value.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

/// Initialization state for a new child PPU thread constructed
/// by `sys_ppu_thread_create`.
///
/// Pure data. The runtime's PPU factory reads these fields to
/// seed the child's `PpuState` per the PPC64 ABI: PC from
/// `entry_code`, r2 (TOC) from `entry_toc`, r1 (stack) from
/// `stack_top`, r3 (argument) from `arg`, r13 (TLS) from
/// `tls_base`, and LR from the supplied sentinel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuThreadInitState {
    /// Child entry code address (resolved from the OPD).
    pub entry_code: u64,
    /// TOC pointer for the child's module (second word of the OPD).
    pub entry_toc: u64,
    /// Argument passed to the entry function via r3.
    pub arg: u64,
    /// Initial stack pointer (r1). Points at the reserved 16-byte
    /// back-chain area at the top of the child's stack block.
    pub stack_top: u64,
    /// Per-thread TLS base (r13). Zero when the ELF has no
    /// PT_TLS segment.
    pub tls_base: u64,
    /// Sentinel address loaded into LR. When the child's entry
    /// function returns, execution jumps here -- the runtime
    /// arranges for a sentinel that triggers
    /// `sys_ppu_thread_exit(0)`.
    pub lr_sentinel: u64,
}

/// Initialization state for a new SPU execution unit.
///
/// Pure data -- the host constructs it from the image registry and
/// guest memory, the runtime applies it when creating the unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuInitState {
    /// Bytes to copy into the SPU's local store.
    pub ls_bytes: Vec<u8>,
    /// Entry point PC within the local store.
    pub entry_pc: u32,
    /// Initial stack pointer (r1).
    pub stack_ptr: u32,
    /// SPU thread arguments (loaded into r3..=r6).
    pub args: [u64; 4],
    /// Unit id of the owning thread group (for join tracking).
    pub group_id: u32,
    /// Slot index within the group.
    pub slot: u32,
}

/// Why the LV2 host is blocking the caller.
///
/// Separate from `cellgov_core::BlockReason` because `cellgov_lv2`
/// does not depend on `cellgov_core`. The runtime maps this to its
/// own `BlockReason` when it consumes the dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2BlockReason {
    /// sys_spu_thread_group_join: waiting for all SPUs in the group
    /// to finish.
    ThreadGroupJoin {
        /// The group being joined.
        group_id: u32,
    },
    /// sys_ppu_thread_join: waiting for a specific PPU thread to
    /// call sys_ppu_thread_exit.
    PpuThreadJoin {
        /// Guest id of the PPU thread being joined.
        target: u64,
    },
    /// sys_lwmutex_lock: waiting for a contended lwmutex to be
    /// released by its current owner.
    LwMutex {
        /// Guest id of the lwmutex being awaited.
        id: u32,
    },
    /// sys_mutex_lock: waiting for a contended heavy mutex to be
    /// released by its current owner.
    Mutex {
        /// Guest id of the heavy mutex being awaited.
        id: u32,
    },
    /// sys_semaphore_wait: waiting for a `sys_semaphore_post` that
    /// hands ownership of a slot to this waiter.
    Semaphore {
        /// Guest id of the semaphore being awaited.
        id: u32,
    },
    /// sys_event_queue_receive: waiting for a
    /// `sys_event_port_send` (or equivalent) to deliver a
    /// payload to this queue.
    EventQueue {
        /// Guest id of the event queue being awaited.
        id: u32,
    },
    /// sys_event_flag_wait: waiting for a `sys_event_flag_set`
    /// that satisfies the waiter's mask.
    EventFlag {
        /// Guest id of the event flag being awaited.
        id: u32,
    },
    /// sys_cond_wait: waiting on a condition variable. The caller
    /// has observably released `mutex_id` on the way in and will
    /// re-acquire it (via `PendingResponse::CondWakeReacquire`) on
    /// wake.
    Cond {
        /// Guest id of the cond being awaited.
        id: u32,
        /// Guest id of the associated mutex the caller released.
        mutex_id: u32,
    },
}

/// What the runtime should do when a blocked PPU is woken.
///
/// Stored in the runtime-owned `SyscallResponseTable`, keyed by the
/// blocked unit's `UnitId`. When the wake condition fires, the
/// runtime reads this, fills r3, and (for join) writes the cause/status
/// out-pointers via `SharedWriteIntent` effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingResponse {
    /// On wake, set r3 = `code`. No out-pointer writes.
    ReturnCode {
        /// Value for r3.
        code: u64,
    },
    /// On wake, set r3 = `code` and write `cause`/`status` to the
    /// guest addresses the caller provided.
    ThreadGroupJoin {
        /// Which group the caller is joining (for wake matching).
        group_id: u32,
        /// Value for r3.
        code: u64,
        /// Guest address to write the exit cause.
        cause_ptr: u32,
        /// Guest address to write the exit status.
        status_ptr: u32,
        /// Exit cause value (filled in at wake time).
        cause: u32,
        /// Exit status value (filled in at wake time).
        status: u32,
    },
    /// On wake, write the PPU child's exit value (u64 big-endian)
    /// into `status_out_ptr` and set r3 = 0 (CELL_OK).
    PpuThreadJoin {
        /// Guest id of the thread being joined. Not strictly
        /// required for wake (the runtime matches via pending
        /// table entries), but useful for trace/diagnostics.
        target: u64,
        /// Guest address to receive the child's exit value.
        status_out_ptr: u32,
    },
    /// On wake, write the event payload's four u64 fields (big-
    /// endian) into the `sys_event_t*` out pointer the caller
    /// passed to `sys_event_queue_receive`, then set r3 = 0.
    ///
    /// Layout matches PSL1GHT's `sys_event_t`:
    ///   offset 0  -- source (event queue / port id)
    ///   offset 8  -- data1
    ///   offset 16 -- data2
    ///   offset 24 -- data3
    ///
    /// Fields are populated at wake time from the payload chosen
    /// by the matching `sys_event_queue_send` (or from the head of
    /// the queue, if the payload was buffered before the waiter
    /// parked).
    EventQueueReceive {
        /// Guest address of the caller's `sys_event_t` out buffer.
        out_ptr: u32,
        /// Event source id (first u64 of the payload).
        source: u64,
        /// Event data1 (second u64).
        data1: u64,
        /// Event data2 (third u64).
        data2: u64,
        /// Event data3 (fourth u64).
        data3: u64,
    },
    /// On wake from `sys_event_flag_wait`, write the observed
    /// bit pattern (u64 big-endian) to `result_ptr` and set
    /// r3 = 0.
    EventFlagWake {
        /// Guest address to receive the observed bit pattern.
        result_ptr: u32,
        /// Observed bit pattern at wake time (set by the
        /// matching `sys_event_flag_set` via `response_updates`).
        observed: u64,
    },
    /// On wake from `sys_cond_wait`, re-acquire `mutex_id` before
    /// returning r3 = 0.
    ///
    /// The wake path consults the mutex table: if free, acquire
    /// and set r3 = 0 and clear the pending entry. If held, the
    /// caller is re-parked on the mutex waiter list with its
    /// `GuestBlockReason` transitioning from `WaitingOnCond` to
    /// `WaitingOnMutex` / `WaitingOnLwMutex`, and the pending
    /// entry is replaced with a plain `ReturnCode { code: 0 }` so
    /// the eventual unlock-wake resolves it.
    ///
    /// `mutex_kind` distinguishes lwmutex from heavy mutex because
    /// they live in distinct tables with distinct id spaces --
    /// lwmutex id 7 and mutex id 7 are two different primitives.
    CondWakeReacquire {
        /// Guest id of the mutex to re-acquire on wake.
        mutex_id: u32,
        /// Whether `mutex_id` names a lightweight mutex or a heavy
        /// mutex.
        mutex_kind: CondMutexKind,
    },
}

/// Which mutex table `PendingResponse::CondWakeReacquire` targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondMutexKind {
    /// The cond's associated mutex is a `sys_lwmutex`.
    LwMutex,
    /// The cond's associated mutex is a heavy `sys_mutex`.
    Mutex,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spu_image_handle_roundtrip() {
        let h = SpuImageHandle::new(42);
        assert_eq!(h.raw(), 42);
    }

    #[test]
    fn spu_image_handle_zero_reserved() {
        let h = SpuImageHandle::new(0);
        assert_eq!(h.raw(), 0);
    }

    #[test]
    fn spu_image_handle_ordering() {
        assert!(SpuImageHandle::new(1) < SpuImageHandle::new(2));
    }

    #[test]
    fn pending_response_return_code() {
        let p = PendingResponse::ReturnCode { code: 0 };
        assert_eq!(p, PendingResponse::ReturnCode { code: 0 });
    }

    #[test]
    fn pending_response_join_carries_pointers() {
        let p = PendingResponse::ThreadGroupJoin {
            group_id: 1,
            code: 0,
            cause_ptr: 0x1000,
            status_ptr: 0x1004,
            cause: 1,
            status: 0,
        };
        assert!(matches!(p, PendingResponse::ThreadGroupJoin { .. }));
    }

    #[test]
    fn pending_response_event_queue_receive_round_trip() {
        let p = PendingResponse::EventQueueReceive {
            out_ptr: 0x2000,
            source: 0x11,
            data1: 0x22,
            data2: 0x33,
            data3: 0x44,
        };
        match p {
            PendingResponse::EventQueueReceive {
                out_ptr,
                source,
                data1,
                data2,
                data3,
            } => {
                assert_eq!(out_ptr, 0x2000);
                assert_eq!(source, 0x11);
                assert_eq!(data1, 0x22);
                assert_eq!(data2, 0x33);
                assert_eq!(data3, 0x44);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn pending_response_cond_wake_reacquire_distinguishes_kind() {
        let lw = PendingResponse::CondWakeReacquire {
            mutex_id: 7,
            mutex_kind: CondMutexKind::LwMutex,
        };
        let hv = PendingResponse::CondWakeReacquire {
            mutex_id: 7,
            mutex_kind: CondMutexKind::Mutex,
        };
        assert_ne!(lw, hv);
    }

    #[test]
    fn lv2_dispatch_immediate() {
        let d = Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![],
        };
        assert!(matches!(d, Lv2Dispatch::Immediate { code: 0, .. }));
    }

    #[test]
    fn spu_init_state_fields() {
        let init = SpuInitState {
            ls_bytes: vec![0; 256],
            entry_pc: 0x100,
            stack_ptr: 0x3FFF0,
            args: [1, 2, 3, 4],
            group_id: 1,
            slot: 0,
        };
        assert_eq!(init.entry_pc, 0x100);
        assert_eq!(init.args[0], 1);
    }

    #[test]
    fn lv2_block_reason_join() {
        let r = Lv2BlockReason::ThreadGroupJoin { group_id: 5 };
        assert_eq!(r, Lv2BlockReason::ThreadGroupJoin { group_id: 5 });
    }
}
