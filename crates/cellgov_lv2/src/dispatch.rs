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
use std::num::NonZeroU32;

use cellgov_effects::Effect;
use cellgov_event::UnitId;

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
        /// Value for r3.
        code: u64,
        /// Effects to commit.
        effects: Vec<Effect>,
    },
    /// Construct and register SPUs for a thread group; `code` -> r3.
    ///
    /// Keyed by slot index because guests may leave slots
    /// uninitialized. `BTreeMap` iteration gives byte-stable
    /// ascending-slot registration order.
    RegisterSpu {
        /// Per-slot init state.
        inits: BTreeMap<u32, SpuInitState>,
        /// Effects to commit.
        effects: Vec<Effect>,
        /// Value for the caller's r3.
        code: u64,
    },
    /// Park the caller; `pending` is applied at wake time.
    Block {
        /// Why the caller is blocking.
        reason: Lv2BlockReason,
        /// Resolves the caller at wake time.
        pending: PendingResponse,
        /// Effects to commit before blocking.
        effects: Vec<Effect>,
    },
    /// `sys_ppu_thread_exit`: source becomes Finished, joiners wake.
    ///
    /// Joiners hold a [`PendingResponse::PpuThreadJoin`]; the wake
    /// writes `exit_value` (u64 BE) to the joiner's `status_out_ptr`
    /// and sets r3 = 0. The source's r3 is not written.
    PpuThreadExit {
        /// Delivered to joiners via their `status_out_ptr`.
        exit_value: u64,
        /// Joiners to transition Blocked -> Runnable.
        woken_unit_ids: Vec<UnitId>,
        /// Effects to commit.
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
        /// Value for the release caller's r3.
        code: u64,
        /// Units whose pending responses resolve.
        woken_unit_ids: Vec<UnitId>,
        /// Per-waiter pending-response overrides.
        response_updates: Vec<(UnitId, PendingResponse)>,
        /// Effects to commit alongside the wakes.
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
        /// Why the source is blocking.
        reason: Lv2BlockReason,
        /// Resolves the source at wake time.
        pending: PendingResponse,
        /// Units to wake alongside the source's block.
        woken_unit_ids: Vec<UnitId>,
        /// Per-waiter pending-response overrides.
        response_updates: Vec<(UnitId, PendingResponse)>,
        /// Effects to commit alongside the transitions.
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
        /// PPC64-ABI seed values.
        init: PpuThreadInitState,
        /// Lowest address of the child's stack block.
        stack_base: u64,
        /// Size of the child's stack block.
        stack_size: u64,
        /// Bytes to commit at `init.tls_base` before entry.
        tls_bytes: Vec<u8>,
        /// Thread priority captured in attrs.
        priority: u32,
        /// Effects to commit alongside the create.
        effects: Vec<Effect>,
    },
}

/// Monotonic host-side token for a loaded SPU image. Non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpuImageHandle(NonZeroU32);

impl SpuImageHandle {
    /// Wrap a raw handle value. Returns `None` if `raw == 0`.
    #[inline]
    pub const fn new(raw: u32) -> Option<Self> {
        match NonZeroU32::new(raw) {
            Some(nz) => Some(Self(nz)),
            None => None,
        }
    }

    /// The raw handle value (always non-zero).
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0.get()
    }
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
    /// r3 argument.
    pub arg: u64,
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
        /// Group being joined.
        group_id: u32,
    },
    /// `sys_ppu_thread_join`.
    PpuThreadJoin {
        /// Target PPU thread id.
        target: u64,
    },
    /// `sys_lwmutex_lock` on a contended lwmutex.
    LwMutex {
        /// lwmutex id.
        id: u32,
    },
    /// `sys_mutex_lock` on a contended heavy mutex.
    Mutex {
        /// Mutex id.
        id: u32,
    },
    /// `sys_semaphore_wait` on an empty semaphore.
    Semaphore {
        /// Semaphore id.
        id: u32,
    },
    /// `sys_event_queue_receive` on an empty queue.
    EventQueue {
        /// Queue id.
        id: u32,
    },
    /// `sys_event_flag_wait` on an unsatisfied mask.
    EventFlag {
        /// Event-flag id.
        id: u32,
    },
    /// `sys_cond_wait`. Caller released `mutex_id` on entry and
    /// re-acquires on wake via [`PendingResponse::CondWakeReacquire`].
    Cond {
        /// Cond id.
        id: u32,
        /// Mutex the caller released on the way in.
        mutex_id: u32,
        /// Which mutex table `mutex_id` names (distinct id spaces).
        mutex_kind: CondMutexKind,
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
        /// Value for r3.
        code: u64,
    },
    /// On wake, set r3 = `code` and write `cause` / `status` to the
    /// guest out-pointers.
    ThreadGroupJoin {
        /// Used for wake matching.
        group_id: u32,
        /// Value for r3.
        code: u64,
        /// Out-pointer for the exit cause.
        cause_ptr: u32,
        /// Out-pointer for the exit status.
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
        /// Out-pointer for the child's exit value.
        status_out_ptr: u32,
    },
    /// On wake, write the event payload (4x u64 BE matching PSL1GHT's
    /// `sys_event_t`: offsets 0 / 8 / 16 / 24 =
    /// source / data1 / data2 / data3) to `out_ptr` and set r3 = 0.
    ///
    /// # Panics
    /// `payload = None` at wake time: the send-side dispatch forgot a
    /// `response_updates` entry. The runtime panics rather than
    /// deliver zero u64s the guest cannot distinguish from a real
    /// event.
    EventQueueReceive {
        /// Out-pointer for the `sys_event_t` buffer.
        out_ptr: u32,
        /// Filled in at wake time by the send side.
        payload: Option<EventPayload>,
    },
    /// On wake, write `observed` (u64 BE) to `result_ptr` and set
    /// r3 = 0. `observed` is filled by the matching
    /// `sys_event_flag_set` via `response_updates`.
    EventFlagWake {
        /// Out-pointer for the observed pattern.
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
        /// Mutex to re-acquire on wake.
        mutex_id: u32,
        /// Which mutex table `mutex_id` names (distinct id spaces).
        mutex_kind: CondMutexKind,
    },
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
        }
    }
}

/// Which mutex table [`PendingResponse::CondWakeReacquire`] targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondMutexKind {
    /// A `sys_lwmutex`.
    LwMutex,
    /// A heavy `sys_mutex`.
    Mutex,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spu_image_handle_roundtrip() {
        let h = SpuImageHandle::new(42).unwrap();
        assert_eq!(h.raw(), 42);
    }

    #[test]
    fn spu_image_handle_zero_rejected() {
        assert!(SpuImageHandle::new(0).is_none());
    }

    #[test]
    fn spu_image_handle_ordering() {
        assert!(SpuImageHandle::new(1).unwrap() < SpuImageHandle::new(2).unwrap());
    }

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
