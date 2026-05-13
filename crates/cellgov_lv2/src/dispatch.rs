//! [`Lv2Dispatch`] packets: pure-data replies from `Lv2Host::dispatch`
//! telling the runtime how to complete a syscall.
//!
//! # Source-unit invariant
//! The *source* is the caller's `UnitId`. Variants that block or mutate
//! the caller ([`Lv2Dispatch::Block`], [`Lv2Dispatch::BlockAndWake`],
//! [`Lv2Dispatch::PpuThreadCreate`], [`Lv2Dispatch::PpuThreadExit`])
//! do so implicitly; other units are touched only through an explicit
//! `woken_unit_ids` entry.

use std::collections::BTreeMap;

use cellgov_effects::Effect;
use cellgov_event::UnitId;

use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::EventPayload;

/// How the runtime should complete a dispatched syscall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lv2Dispatch {
    /// Commit `effects`, then write `code` (0 = `CELL_OK`) to r3.
    ///
    /// Effects commit regardless of `code`, so error paths MUST emit
    /// `effects = vec![]` -- otherwise a `SharedWriteIntent` paired
    /// with an error return still commits the write.
    Immediate {
        /// Return code written to r3 (0 = `CELL_OK`).
        code: u64,
        /// Effects committed before the return code is written.
        effects: Vec<Effect>,
    },
    /// Construct and register SPUs for a thread group; `code` -> r3.
    ///
    /// `BTreeMap` keying gives byte-stable ascending-slot registration
    /// order across guests that leave slots uninitialized.
    RegisterSpu {
        /// Per-slot SPU init state, keyed by slot index for stable order.
        inits: BTreeMap<u32, SpuInitState>,
        /// Effects committed alongside the registration.
        effects: Vec<Effect>,
        /// Return code written to the caller's r3.
        code: u64,
    },
    /// Park the caller; `pending` is applied at wake time.
    Block {
        /// Why the caller is being parked.
        reason: Lv2BlockReason,
        /// Wake-time response applied when the caller resumes.
        pending: PendingResponse,
        /// Effects committed at park time.
        effects: Vec<Effect>,
    },
    /// `sys_ppu_thread_exit`: source becomes Finished, joiners wake.
    ///
    /// Joiners hold a [`PendingResponse::PpuThreadJoin`]; the wake
    /// writes `exit_value` (u64 BE) to the joiner's `status_out_ptr`
    /// and sets r3 = 0. The source's r3 is not written.
    ///
    /// `lwmutex_inheritors` carries peers parked on kernel-side lwmutex
    /// sleep queues whose held lwmutexes were transferred by the exit
    /// (one transfer per lwmutex with a non-empty waiter list, in id
    /// order). Each entry's pending [`PendingResponse::LwMutexWake`]
    /// resolves via the usual sync-wake path. Real PS3 strands these
    /// waiters; the transfer is a deterministic-oracle simplification
    /// so a guest printf returning to `sys_ppu_thread_exit` without
    /// flushing its stdio lock still allows progress.
    PpuThreadExit {
        /// Exit value delivered to joiners (u64 BE at `status_out_ptr`).
        exit_value: u64,
        /// Joiners woken by the exit.
        woken_unit_ids: Vec<UnitId>,
        /// Peers parked on lwmutexes whose ownership transferred at exit.
        lwmutex_inheritors: Vec<UnitId>,
        /// Effects committed at exit.
        effects: Vec<Effect>,
    },
    /// Release-side completion: source returns `code`, each unit in
    /// `woken_unit_ids` resolves its pending response.
    ///
    /// Emitted by lwmutex/mutex unlock, semaphore post, event-queue
    /// send, event-flag set, and cond signal. `response_updates`
    /// carries per-waiter overrides for primitives whose wake payload
    /// is known only at release time (`sys_event_queue_send`,
    /// `sys_event_flag_set`).
    ///
    /// # Invariants
    /// - Every `response_updates` key MUST appear in
    ///   `woken_unit_ids`; an update for a non-woken unit would
    ///   silently mutate an unrelated future wait.
    /// - An update's [`PendingResponse::variant_tag`] MUST match the
    ///   existing entry's tag -- updates are partial fills, not
    ///   variant replacements.
    WakeAndReturn {
        /// Return code written to the source's r3.
        code: u64,
        /// Units whose pending responses resolve.
        woken_unit_ids: Vec<UnitId>,
        /// Per-waiter overrides applied to the pending response table.
        response_updates: Vec<(UnitId, PendingResponse)>,
        /// Effects committed alongside the wake.
        effects: Vec<Effect>,
    },
    /// Source blocks and other units wake in the same dispatch.
    ///
    /// Emitted by `sys_cond_wait`: releasing the associated mutex can
    /// transfer ownership to a parked mutex waiter, which wakes in the
    /// same step the cond caller blocks. The source's r3 resolves at
    /// wake time via `pending`.
    ///
    /// Same `woken_unit_ids` / `response_updates` invariants as
    /// [`Self::WakeAndReturn`].
    BlockAndWake {
        /// Why the source is being parked.
        reason: Lv2BlockReason,
        /// Wake-time response applied to the source.
        pending: PendingResponse,
        /// Units whose pending responses resolve in the same step.
        woken_unit_ids: Vec<UnitId>,
        /// Per-waiter pending-response overrides.
        response_updates: Vec<(UnitId, PendingResponse)>,
        /// Effects committed alongside the block-and-wake.
        effects: Vec<Effect>,
    },
    /// Worker-thread callback-dispatch spawn: register a fresh PPU
    /// thread for the callback AND park `parent` until the worker
    /// returns. Fuses [`Self::PpuThreadCreate`] and [`Self::Block`]
    /// so the runtime applies both transitions atomically per the
    /// `call_guest_callback_sync` contract.
    ///
    /// The runtime materializes the worker via the existing
    /// PpuThreadCreate path, allocates the worker's `PpuThreadId`
    /// from `PpuThreadTable::create`, links the worker to `parent`
    /// via `Lv2Host::attach_callback_worker`, and parks `parent`
    /// with `parent_pending`. On the worker's terminal `blr` ->
    /// trampoline -> `sc 0`, the runtime emits a [`Self::WakeAndReturn`]
    /// keyed to `parent` with the worker's r3..=r10 as the `args`
    /// field of [`PendingResponse::CallbackReturn`].
    ///
    /// # Invariants
    /// - `parent_pending.variant_tag() == PendingResponse::CallbackReturn(_).variant_tag()`.
    /// - The worker id is allocated by the runtime, not the host;
    ///   the host learns it via `attach_callback_worker` after
    ///   `PpuThreadTable::create` runs.
    CallbackSpawn {
        /// PPC64 register seed for the worker thread.
        worker_init: PpuThreadInitState,
        /// Worker stack base address.
        worker_stack_base: u64,
        /// Worker stack size in bytes.
        worker_stack_size: u64,
        /// Worker scheduling priority.
        worker_priority: u32,
        /// Caller parked until the worker returns.
        parent: UnitId,
        /// Must be a [`PendingResponse::CallbackReturn`].
        parent_pending: PendingResponse,
        /// Effects committed at spawn time.
        effects: Vec<Effect>,
    },
    /// `sys_ppu_thread_create` with the OPD already resolved.
    ///
    /// The host reads the 16 BE OPD bytes via
    /// `Lv2Runtime::read_committed` before emitting this variant; a
    /// bad descriptor address surfaces as
    /// `Immediate { code: CELL_EFAULT }` and no child is registered.
    ///
    /// # Invariants
    /// - `tls_bytes.is_empty()` OR `init.tls_base != 0` -- a
    ///   non-empty TLS image cannot commit to guest address 0. The
    ///   host rejects the violation with `CELL_EINVAL` upstream; the
    ///   runtime asserts defensively.
    PpuThreadCreate {
        /// Guest address to receive the minted thread id (u64 BE).
        id_ptr: u32,
        /// PPC64 register seed for the child thread.
        init: PpuThreadInitState,
        /// Child stack base address.
        stack_base: u64,
        /// Child stack size in bytes.
        stack_size: u64,
        /// Bytes to commit at `init.tls_base` before entry.
        tls_bytes: Vec<u8>,
        /// Child scheduling priority.
        priority: u32,
        /// Effects committed at create time.
        effects: Vec<Effect>,
    },
}

/// PPC64-ABI seed values for a new child PPU thread.
///
/// `entry_code` / `entry_toc` are the resolved OPD words, not the OPD
/// descriptor address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuThreadInitState {
    /// First OPD word -- NIP for the child.
    pub entry_code: u64,
    /// Second OPD word -- r2 (TOC) for the child.
    pub entry_toc: u64,
    /// r3.
    pub arg: u64,
    /// r4..=r10. All zero on the `sys_ppu_thread_create` path (which
    /// only carries one argument). Populated for the callback-dispatch
    /// worker path so a title-supplied callback receives the full
    /// 8-register argument set captured at the parent's call site.
    pub extra_args: [u64; 7],
    /// r1: 16-byte back-chain area at the top of the stack.
    pub stack_top: u64,
    /// r13. Zero when the ELF has no PT_TLS segment.
    pub tls_base: u64,
    /// Loaded into LR; entered if the child returns from entry.
    pub lr_sentinel: u64,
}

/// Per-slot SPU init state; the slot index is the
/// [`Lv2Dispatch::RegisterSpu`]`::inits` key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuInitState {
    /// Bytes to copy into the SPU's local store.
    pub ls_bytes: Vec<u8>,
    /// Entry PC within the local store.
    pub entry_pc: u32,
    /// r1.
    pub stack_ptr: u32,
    /// r3..=r6.
    pub args: [u64; 4],
    /// Owning thread group, used for join tracking.
    pub group_id: u32,
}

/// Why the LV2 host is blocking the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2BlockReason {
    /// `sys_spu_thread_group_join`.
    ThreadGroupJoin {
        /// Target thread group id.
        group_id: u32,
    },
    /// `sys_ppu_thread_join`.
    PpuThreadJoin {
        /// Target PPU thread id.
        target: u64,
    },
    /// `sys_lwmutex_lock` on a contended lwmutex.
    LwMutex {
        /// Target lwmutex id.
        id: u32,
    },
    /// `sys_mutex_lock` on a contended heavy mutex.
    Mutex {
        /// Target mutex id.
        id: u32,
    },
    /// `sys_semaphore_wait` on an empty semaphore.
    Semaphore {
        /// Target semaphore id.
        id: u32,
    },
    /// `sys_event_queue_receive` on an empty queue.
    EventQueue {
        /// Target event queue id.
        id: u32,
    },
    /// `sys_event_flag_wait` on an unsatisfied mask.
    EventFlag {
        /// Target event flag id.
        id: u32,
    },
    /// `sys_cond_wait`. Caller released `mutex_id` on entry and
    /// re-acquires on wake via [`PendingResponse::CondWakeReacquire`].
    Cond {
        /// Target cond variable id.
        id: u32,
        /// Companion mutex released at park, re-acquired at wake.
        mutex_id: u32,
        /// Which mutex table `mutex_id` names (distinct id spaces).
        mutex_kind: CondMutexKind,
    },
    /// Parked on a worker-thread callback dispatch
    /// (`Lv2Host::call_guest_callback_sync`). Resolved when the
    /// worker thread `worker` issues the CellGov-private return
    /// trampoline syscall (`CB_RETURN_SYSCALL`).
    CallbackDispatch {
        /// Used by the trampoline-return dispatch arm to look up the
        /// parent in `Lv2Host::callback_parents` and key the wake.
        worker: PpuThreadId,
    },
}

/// What the runtime should do when a blocked PPU is woken.
///
/// Stored in the runtime's `SyscallResponseTable`, keyed by the
/// blocked unit's `UnitId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingResponse {
    /// On wake, set r3 = `code`. No out-pointer writes.
    ReturnCode {
        /// Return code written to r3 on wake.
        code: u64,
    },
    /// On wake, set r3 = `code` and write `cause` / `status` to the
    /// guest out-pointers.
    ThreadGroupJoin {
        /// Used for wake matching.
        group_id: u32,
        /// Return code written to r3 on wake.
        code: u64,
        /// Guest out-pointer receiving `cause` (u32 BE).
        cause_ptr: u32,
        /// Guest out-pointer receiving `status` (u32 BE).
        status_ptr: u32,
        /// Filled in at wake time.
        cause: u32,
        /// Filled in at wake time.
        status: u32,
    },
    /// On wake, write the exit value (u64 BE) to `status_out_ptr`
    /// and set r3 = 0.
    PpuThreadJoin {
        /// Trace/diagnostic only; wake matching uses the pending
        /// table entry, not this field.
        target: u64,
        /// Guest out-pointer receiving the exit value (u64 BE).
        status_out_ptr: u32,
    },
    /// On wake, write the event payload (4x u64 BE matching the PS3
    /// `sys_event_t` ABI: offsets 0 / 8 / 16 / 24 =
    /// source / data1 / data2 / data3) to `out_ptr` and set r3 = 0.
    ///
    /// # Panics
    /// `payload = None` at wake time: the send-side dispatch forgot a
    /// `response_updates` entry. The runtime panics rather than
    /// deliver zero u64s the guest cannot distinguish from a real
    /// event.
    EventQueueReceive {
        /// Guest out-pointer receiving the 4x u64 BE event payload.
        out_ptr: u32,
        /// Filled in at wake time by the send side.
        payload: Option<EventPayload>,
    },
    /// On wake, write `observed` (u64 BE) to `result_ptr` and set
    /// r3 = 0. `observed` is filled by the matching
    /// `sys_event_flag_set` via `response_updates`.
    EventFlagWake {
        /// Guest out-pointer receiving `observed` (u64 BE).
        result_ptr: u32,
        /// Filled in at wake time by the set side.
        observed: u64,
    },
    /// On wake, re-acquire `mutex_id` before returning r3 = 0. A held
    /// mutex re-parks the caller on its waiter list; the pending
    /// entry is replaced by `ReturnCode { code: 0 }`.
    ///
    /// # Invariant
    /// The target mutex is alive -- `sys_mutex_destroy` /
    /// `sys_lwmutex_destroy` MUST reject with `CELL_EBUSY` while any
    /// cond waiter references it. An empty table entry at wake time
    /// is a host-level invariant break.
    CondWakeReacquire {
        /// Mutex re-acquired before returning r3 = 0.
        mutex_id: u32,
        /// Which mutex table `mutex_id` names (distinct id spaces).
        mutex_kind: CondMutexKind,
    },
    /// On wake, claim ownership of the user-space `sys_lwmutex_t`:
    /// write `owner = caller`, `recursive_count = 1`, decrement the
    /// `waiter` field, and set r3 = 0.
    ///
    /// `mutex_ptr == 0` means the raw LV2 syscall path was used (no
    /// user-space struct), and the wake just sets r3 = 0.
    LwMutexWake {
        /// User-space `sys_lwmutex_t` address (offset 0 = owner,
        /// offset 4 = waiter, offset 12 = recursive_count).
        mutex_ptr: u32,
        /// Caller's PPU thread id (low 32 bits of `PpuThreadId`),
        /// written into the user-space owner slot.
        caller: u32,
    },
    /// Resume a parkable HLE handler that previously called
    /// `Lv2Host::call_guest_callback_sync`. `args` carries the
    /// worker's `r3..=r10` captured at the trampoline. `stage`
    /// discriminates which handler to resume (and where in that
    /// handler's flow). The wake-side delivery writes `args[0]` (the
    /// worker's r3) into the parent's r3; `args[1..=7]` stay in the
    /// response payload for the resuming handler to read.
    ///
    /// # Invariants
    /// - `args` is exactly the eight-register set captured from the
    ///   worker at trampoline entry. Padding or zero-fill outside
    ///   that set is forbidden -- the resuming handler may key on
    ///   `args[N != 0]` and silent zero-fill would be
    ///   indistinguishable from the worker actually returning zero.
    /// - `stage` is constructed by the spawn-side handler and must
    ///   not be mutated between spawn and wake.
    CallbackReturn {
        /// Continuation kind; selects which handler resumes and how.
        stage: CallbackReturnStage,
        /// Worker `r3..=r10` from the trampoline-entry capture.
        args: [u64; 8],
    },
}

/// Continuation kind for [`PendingResponse::CallbackReturn`].
///
/// Each parkable HLE handler adds a variant. `Synthetic` is the
/// test-only placeholder; consumer-specific variants carry the
/// per-handler resume state. `#[non_exhaustive]` makes a missing
/// handler-resume case a compile error in the resume dispatcher
/// while keeping crate-external matches forward-compatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CallbackReturnStage {
    /// Test-only marker. Wake delivery writes the worker's `args[0]`
    /// (r3) into the parent's r3 and routes nothing further.
    Synthetic,
    /// `cellSaveDataAutoLoad` / `cellSaveDataAutoLoad2` resume after
    /// the title's `funcStat` callback returns. The resume arm reads
    /// `cb_result.result` and either finalizes the AutoLoad call
    /// (CELL_OK / `cellSaveData` error code) or transitions into the
    /// funcFile loop.
    AutoLoadAfterStat {
        /// Guest pointer to the HLE-allocated `CellSaveDataCBResult`.
        cb_result_addr: u32,
        /// Guest pointer to the HLE-allocated `CellSaveDataStatGet`.
        stat_get_addr: u32,
        /// Guest pointer to the HLE-allocated `CellSaveDataStatSet`.
        stat_set_addr: u32,
        /// Title-supplied funcFile OPD pointer (for the OK_NEXT
        /// transition into the funcFile loop).
        func_file_opd: u32,
    },
}

impl CallbackReturnStage {
    /// Stable byte tag for state-hash serialization.
    ///
    /// Adding a variant MUST assign a fresh tag and bump the
    /// consumer's state-hash format version.
    pub fn stable_tag(self) -> u8 {
        match self {
            CallbackReturnStage::Synthetic => 0,
            CallbackReturnStage::AutoLoadAfterStat { .. } => 1,
        }
    }
}

impl PendingResponse {
    /// Stable variant discriminant for `response_updates` variant-tag
    /// checks in [`Lv2Dispatch::WakeAndReturn`] and
    /// [`Lv2Dispatch::BlockAndWake`].
    pub fn variant_tag(&self) -> u8 {
        match self {
            PendingResponse::ReturnCode { .. } => 0,
            PendingResponse::ThreadGroupJoin { .. } => 1,
            PendingResponse::PpuThreadJoin { .. } => 2,
            PendingResponse::EventQueueReceive { .. } => 3,
            PendingResponse::EventFlagWake { .. } => 4,
            PendingResponse::CondWakeReacquire { .. } => 5,
            PendingResponse::LwMutexWake { .. } => 6,
            PendingResponse::CallbackReturn { .. } => 7,
        }
    }
}

/// Which mutex table [`PendingResponse::CondWakeReacquire`] targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondMutexKind {
    /// `mutex_id` names a `sys_lwmutex_t`.
    LwMutex,
    /// `mutex_id` names a heavy `sys_mutex_t`.
    Mutex,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_response_variant_tags_are_distinct() {
        let tags = [
            PendingResponse::ReturnCode { code: 0 }.variant_tag(),
            PendingResponse::ThreadGroupJoin {
                group_id: 0,
                code: 0,
                cause_ptr: 0,
                status_ptr: 0,
                cause: 0,
                status: 0,
            }
            .variant_tag(),
            PendingResponse::PpuThreadJoin {
                target: 0,
                status_out_ptr: 0,
            }
            .variant_tag(),
            PendingResponse::EventQueueReceive {
                out_ptr: 0,
                payload: None,
            }
            .variant_tag(),
            PendingResponse::EventFlagWake {
                result_ptr: 0,
                observed: 0,
            }
            .variant_tag(),
            PendingResponse::CondWakeReacquire {
                mutex_id: 0,
                mutex_kind: CondMutexKind::LwMutex,
            }
            .variant_tag(),
            PendingResponse::LwMutexWake {
                mutex_ptr: 0,
                caller: 0,
            }
            .variant_tag(),
            PendingResponse::CallbackReturn {
                stage: CallbackReturnStage::Synthetic,
                args: [0; 8],
            }
            .variant_tag(),
        ];
        let mut seen = std::collections::BTreeSet::new();
        for tag in tags {
            assert!(seen.insert(tag), "duplicate variant_tag byte");
        }
    }

    #[test]
    fn event_queue_receive_payload_round_trip() {
        let original = EventPayload {
            source: 0x11,
            data1: 0x22,
            data2: 0x33,
            data3: 0x44,
        };
        let p = PendingResponse::EventQueueReceive {
            out_ptr: 0x2000,
            payload: Some(original),
        };
        match p {
            PendingResponse::EventQueueReceive { out_ptr, payload } => {
                assert_eq!(out_ptr, 0x2000);
                assert_eq!(payload, Some(original));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn event_queue_receive_payload_none_distinct_from_some_zero() {
        let none = PendingResponse::EventQueueReceive {
            out_ptr: 0x2000,
            payload: None,
        };
        let some_zero = PendingResponse::EventQueueReceive {
            out_ptr: 0x2000,
            payload: Some(EventPayload {
                source: 0,
                data1: 0,
                data2: 0,
                data3: 0,
            }),
        };
        assert_ne!(none, some_zero);
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
        };
        assert_eq!(init.entry_pc, 0x100);
        assert_eq!(init.args[0], 1);
    }

    #[test]
    fn lv2_block_reason_join() {
        let r = Lv2BlockReason::ThreadGroupJoin { group_id: 5 };
        assert_eq!(r, Lv2BlockReason::ThreadGroupJoin { group_id: 5 });
    }

    #[test]
    fn lv2_block_reason_cond_carries_mutex_kind() {
        let lw = Lv2BlockReason::Cond {
            id: 1,
            mutex_id: 7,
            mutex_kind: CondMutexKind::LwMutex,
        };
        let hv = Lv2BlockReason::Cond {
            id: 1,
            mutex_id: 7,
            mutex_kind: CondMutexKind::Mutex,
        };
        assert_ne!(lw, hv);
    }
}
