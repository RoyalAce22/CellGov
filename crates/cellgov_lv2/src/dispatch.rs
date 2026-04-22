//! Dispatch result types returned by `Lv2Host::dispatch`.
//!
//! `Lv2Dispatch` tells the runtime how to complete a syscall. Every
//! variant carries plain data -- no closures, no runtime references.
//!
//! # Source-unit invariant
//! The *source* is the caller's `UnitId`, passed to
//! `Lv2Host::dispatch`. Variants that block or mutate the caller
//! ([`Lv2Dispatch::Block`], [`Lv2Dispatch::BlockAndWake`],
//! [`Lv2Dispatch::PpuThreadCreate`], [`Lv2Dispatch::PpuThreadExit`])
//! do so implicitly. A dispatch never mutates a unit other than the
//! caller's own, absent an explicit `woken_unit_ids` entry.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use cellgov_effects::Effect;
use cellgov_event::UnitId;

use crate::sync_primitives::EventPayload;

/// How the runtime should complete a dispatched syscall.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lv2Dispatch {
    /// Immediate syscall return: commit `effects`, then write `code`
    /// to r3.
    ///
    /// Effects commit regardless of `code`, so host MUST emit
    /// `effects = vec![]` for error codes -- otherwise a
    /// `SharedWriteIntent` paired with an error return still
    /// commits the write.
    Immediate {
        /// Value for r3 (0 = CELL_OK).
        code: u64,
        /// Side effects to commit.
        effects: Vec<Effect>,
    },
    /// Construct and register SPUs for a thread group.
    ///
    /// Keyed by slot index -- guests may leave slots uninitialized
    /// (sparse groups are legal), so the slot lives in the key
    /// rather than the Vec position. BTreeMap iteration is ascending
    /// by slot, giving byte-stable registration order.
    RegisterSpu {
        /// Slot-keyed per-SPU init state.
        inits: BTreeMap<u32, SpuInitState>,
        /// Side effects to commit.
        effects: Vec<Effect>,
        /// Value for the caller's r3.
        code: u64,
    },
    /// Park the caller until a condition resolves.
    Block {
        /// Why the caller is blocking.
        reason: Lv2BlockReason,
        /// Applied at wake time.
        pending: PendingResponse,
        /// Commit before blocking.
        effects: Vec<Effect>,
    },
    /// Source called `sys_ppu_thread_exit`; source becomes Finished
    /// and each joiner in `woken_unit_ids` wakes.
    ///
    /// Joiners hold a [`PendingResponse::PpuThreadJoin`], so the wake
    /// writes `exit_value` (u64 BE) to the joiner's `status_out_ptr`
    /// and sets r3 = 0. No r3 is written for the source.
    PpuThreadExit {
        /// Delivered to joiners via their `status_out_ptr`, not r3.
        exit_value: u64,
        /// Joiners to transition Blocked -> Runnable.
        woken_unit_ids: Vec<UnitId>,
        /// Side effects to commit.
        effects: Vec<Effect>,
    },
    /// Release-side completion: source returns `code`, units in
    /// `woken_unit_ids` resolve their pending responses.
    ///
    /// Emitted by lwmutex/mutex unlock, semaphore post, event-queue
    /// send, event-flag set, cond signal. `response_updates` carries
    /// per-waiter pending-response overrides for primitives whose
    /// wake payload is only known at release time
    /// (`sys_event_queue_send`, `sys_event_flag_set`).
    ///
    /// # Invariants
    /// - Every `response_updates` key MUST be in `woken_unit_ids`;
    ///   an update for a non-woken unit would silently mutate an
    ///   unrelated future wait.
    /// - An update's [`PendingResponse::variant_tag`] MUST match
    ///   the existing entry's tag. Updates are partial fills, not
    ///   variant replacements.
    WakeAndReturn {
        /// Value for the release caller's r3.
        code: u64,
        /// Units whose pending responses resolve.
        woken_unit_ids: Vec<UnitId>,
        /// Per-waiter pending-response overrides.
        response_updates: Vec<(UnitId, PendingResponse)>,
        /// Commit alongside the wakes.
        effects: Vec<Effect>,
    },
    /// Source blocks AND other units wake in the same dispatch.
    ///
    /// Emitted by `sys_cond_wait`: releasing the cond's associated
    /// mutex can transfer ownership to a parked mutex waiter, which
    /// wakes in the same step the cond caller blocks. The source's
    /// r3 is not set here -- the eventual wake resolves `pending`.
    ///
    /// # Invariants
    /// Same `woken_unit_ids` / `response_updates` correspondence and
    /// variant-tag match rules as [`Self::WakeAndReturn`].
    BlockAndWake {
        /// Why the source is blocking.
        reason: Lv2BlockReason,
        /// Resolves the source at wake time.
        pending: PendingResponse,
        /// Units to wake alongside the source's block.
        woken_unit_ids: Vec<UnitId>,
        /// Per-waiter pending-response overrides.
        response_updates: Vec<(UnitId, PendingResponse)>,
        /// Commit alongside the transitions.
        effects: Vec<Effect>,
    },
    /// Source called `sys_ppu_thread_create`.
    ///
    /// OPD resolution (16 BE bytes at the descriptor address) runs
    /// host-side via `Lv2Runtime::read_committed` before this variant
    /// is emitted. Bad addresses surface as
    /// `Immediate { code: CELL_EFAULT }` and no child is registered,
    /// so the runtime sees this variant only when `init` is fully
    /// materialized.
    ///
    /// # Invariants
    /// - `tls_bytes.is_empty()` OR `init.tls_base != 0` -- a
    ///   non-empty TLS image cannot commit to guest address 0. The
    ///   host rejects the violation with `CELL_EINVAL` upstream and
    ///   the runtime asserts defensively.
    PpuThreadCreate {
        /// Guest address for the minted thread id (u64 BE).
        id_ptr: u32,
        /// PPC64-ABI seed values (OPD already resolved).
        init: PpuThreadInitState,
        /// Lowest address of the child's stack block.
        stack_base: u64,
        /// Size of the child's stack block.
        stack_size: u64,
        /// Initial bytes to commit at `init.tls_base`.
        tls_bytes: Vec<u8>,
        /// Captured in thread attrs.
        priority: u32,
        /// Commit alongside the create transition.
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
/// `entry_code` and `entry_toc` are resolved OPD words read from
/// guest memory upstream, not the OPD descriptor address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuThreadInitState {
    /// First word of the OPD -- NIP for the child.
    pub entry_code: u64,
    /// Second word of the OPD -- r2 (TOC) for the child.
    pub entry_toc: u64,
    /// r3 argument.
    pub arg: u64,
    /// r1: points at the 16-byte back-chain area at the top of the
    /// child's stack.
    pub stack_top: u64,
    /// r13. Zero when the ELF has no PT_TLS segment.
    pub tls_base: u64,
    /// Loaded into LR; entered if the child returns from entry.
    pub lr_sentinel: u64,
}

/// Per-slot SPU init state.
///
/// The slot index is the [`Lv2Dispatch::RegisterSpu`]`::inits` key,
/// not a field here.
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
    /// Owning thread group (used for join tracking).
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
    /// `sys_cond_wait`. Caller has released `mutex_id` on the way
    /// in and re-acquires on wake via
    /// [`PendingResponse::CondWakeReacquire`]. `mutex_kind`
    /// disambiguates lwmutex vs heavy mutex (distinct id spaces).
    Cond {
        /// Cond id.
        id: u32,
        /// Associated mutex the caller released on the way in.
        mutex_id: u32,
        /// Which mutex table `mutex_id` names.
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
    /// On wake, set r3 = `code` and write `cause` / `status` to
    /// the guest out-pointers.
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
    /// On wake, write the exit value (u64 BE) into `status_out_ptr`
    /// and set r3 = 0.
    PpuThreadJoin {
        /// Trace/diagnostic; wake matching uses the pending-table
        /// entry, not this field.
        target: u64,
        /// Out-pointer for the child's exit value.
        status_out_ptr: u32,
    },
    /// On wake, write the event payload (4x u64 BE matching
    /// PSL1GHT's `sys_event_t`: offsets 0 / 8 / 16 / 24 =
    /// source / data1 / data2 / data3) to `out_ptr` and set r3 = 0.
    ///
    /// # Panics
    /// Reaching the wake path with `payload = None` means the
    /// send-side dispatch forgot a `response_updates` entry. The
    /// runtime panics rather than delivering zero u64s the guest
    /// cannot distinguish from a real event.
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
    /// On wake, re-acquire `mutex_id` before returning r3 = 0. If
    /// the mutex is held, the caller re-parks on its waiter list
    /// and the pending entry is replaced by
    /// `ReturnCode { code: 0 }`.
    ///
    /// `mutex_kind` disambiguates lwmutex vs heavy mutex (distinct
    /// id spaces).
    ///
    /// # Invariant
    /// The target mutex is alive: `sys_mutex_destroy` /
    /// `sys_lwmutex_destroy` MUST reject with `CELL_EBUSY` while
    /// any cond waiter references it. An empty table entry at
    /// wake time is a host-level invariant break.
    CondWakeReacquire {
        /// Mutex to re-acquire on wake.
        mutex_id: u32,
        /// Which mutex table `mutex_id` names.
        mutex_kind: CondMutexKind,
    },
}

impl PendingResponse {
    /// Stable variant discriminant for `response_updates`
    /// variant-match checks in [`Lv2Dispatch::WakeAndReturn`] and
    /// [`Lv2Dispatch::BlockAndWake`]. The runtime asserts
    /// `existing.variant_tag() == update.variant_tag()` before
    /// installing an override.
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
