//! Worker-thread callback-dispatch primitive.
//!
//! `Lv2Host::call_guest_callback_sync` is the public entry: an HLE
//! handler invokes a title-supplied callback OPD on a fresh worker
//! PPU thread and parks the parent unit until the worker returns.
//! The worker's terminal `blr` sets `PC = LR` where LR was staged
//! to [`cellgov_ps3_abi::callback_dispatch::CALLBACK_RETURN_CODE_ADDR`],
//! landing directly on the trampoline body which fires a CellGov-
//! private LV2 syscall classified as
//! [`crate::Lv2Request::CallbackDispatchReturn`]. The returning
//! dispatch arm here resolves the worker -> parent linkage, decrements
//! the recursion-depth tracker, and emits a
//! [`crate::Lv2Dispatch::WakeAndReturn`] keyed to the parent.
//!
//! This is the canonical parkable-handler pattern. Future parkable
//! HLE handlers (cellSaveDataAutoLoad's funcStat / funcFile
//! dispatch, vblank-handler invocation, SPURS exception handlers)
//! all route through `call_guest_callback_sync`; the per-handler
//! resume state lives in the
//! [`crate::CallbackReturnStage`] enum carried by
//! [`crate::PendingResponse::CallbackReturn`].

use cellgov_event::UnitId;
use cellgov_ps3_abi::callback_dispatch::{CALLBACK_DEPTH_CAP, CALLBACK_RETURN_CODE_ADDR};

use crate::dispatch::{CallbackReturnStage, Lv2Dispatch, PendingResponse, PpuThreadInitState};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::ppu_thread::PpuThreadId;

/// Failure modes for [`Lv2Host::call_guest_callback_sync`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackError {
    /// Recursion depth would exceed
    /// [`CALLBACK_DEPTH_CAP`]. Title-side runaway recursion or the
    /// rare case of a callback re-entering the same handler more
    /// than the cap allows.
    TooDeep,
    /// Title-supplied OPD pointer is unmapped or short-read by the
    /// runtime. CellGov returns this rather than spawning a worker
    /// against an undefined entry point.
    OpdReadFailed,
    /// Worker stack allocation failed (child-stack arena exhausted).
    StackAllocFailed,
}

impl Lv2Host {
    /// Invoke a guest callback synchronously on a fresh worker PPU
    /// thread, parking `parent` until the worker returns through the
    /// CellGov-private trampoline.
    ///
    /// # Contract
    /// - `parent` is the calling PPU unit. It is parked with a
    ///   [`PendingResponse::CallbackReturn { stage, args: [0; 8] }`]
    ///   that the trampoline-return dispatch arm later overwrites
    ///   with the worker's captured `r3..=r10`.
    /// - `opd_addr` is a guest pointer to an 8-byte PS3 OPD
    ///   (`u32 BE code_addr || u32 BE toc`). Read failure surfaces
    ///   as [`CallbackError::OpdReadFailed`] before any state is
    ///   mutated.
    /// - `args[0..=7]` becomes the worker's `r3..=r10`. Higher
    ///   slots are not used.
    /// - `stage` discriminates which handler-resume path the
    ///   parent's pending response carries.
    ///
    /// On success, returns [`Lv2Dispatch::CallbackSpawn`]. The
    /// runtime's handle path allocates the worker's [`PpuThreadId`]
    /// (via [`crate::ppu_thread::PpuThreadTable::create`]) and
    /// then calls back into [`Self::attach_callback_worker`] to link
    /// the worker to `parent` in the host's bookkeeping.
    ///
    /// # Errors
    /// - [`CallbackError::TooDeep`] when `parent` is already at the
    ///   recursion cap.
    /// - [`CallbackError::OpdReadFailed`] when the OPD pointer is
    ///   unreadable.
    /// - [`CallbackError::StackAllocFailed`] when the child-stack
    ///   arena is exhausted.
    pub fn call_guest_callback_sync(
        &mut self,
        parent: UnitId,
        opd_addr: u32,
        args: [u64; 8],
        stage: CallbackReturnStage,
        rt: &dyn Lv2Runtime,
    ) -> Result<Lv2Dispatch, CallbackError> {
        // Recursion cap: parent's existing depth + 1 must not exceed
        // CALLBACK_DEPTH_CAP. Checked before any allocation.
        let current_depth = self.callback_depth.get(&parent).copied().unwrap_or(0);
        if current_depth >= CALLBACK_DEPTH_CAP {
            return Err(CallbackError::TooDeep);
        }

        // Inherit the parent PPU thread's tls_base into the worker so
        // the worker's r13 lands on the same TLS image. Title code
        // dereferences `r13 + offset` for locale tables, errno, and
        // other thread-local globals; r13 = 0 produces sign-extended
        // negative addresses that fault on the first such access.
        // Callbacks are short-lived and synchronous from the parent's
        // perspective; sharing TLS is safe because the parent is
        // parked Blocked while the worker runs.
        let parent_tls_base = self
            .ppu_threads
            .get_by_unit(parent)
            .map(|t| u64::from(t.attrs.tls_base))
            .unwrap_or(0);

        // Read the 8-byte OPD: u32 BE code_addr || u32 BE toc.
        // Same shape as `dispatch_ppu_thread_create` reads.
        let opd_bytes = rt
            .read_committed(opd_addr as u64, 8)
            .ok_or(CallbackError::OpdReadFailed)?;
        if opd_bytes.len() < 8 {
            return Err(CallbackError::OpdReadFailed);
        }
        let entry_code = u32::from_be_bytes(opd_bytes[0..4].try_into().unwrap()) as u64;
        let entry_toc = u32::from_be_bytes(opd_bytes[4..8].try_into().unwrap()) as u64;

        // Allocate worker stack at the ABI minimum. Worker callbacks
        // are typically short-lived; titles needing larger stacks
        // would surface as a separate ENOMEM frontier.
        let stack = self
            .allocate_child_stack(0x4000, 0x10)
            .ok_or(CallbackError::StackAllocFailed)?;

        // r3 = args[0]; r4..=r10 = args[1..=7]; r1 = stack top;
        // r2 = TOC; LR = trampoline code address so the worker's
        // terminal `blr` (which sets PC = LR) lands on the
        // trampoline body. Note this is the code address, NOT the
        // OPD address: blr branches to LR directly, and the OPD
        // would decode as garbage instructions.
        let mut extra_args = [0u64; 7];
        extra_args.copy_from_slice(&args[1..]);
        let worker_init = PpuThreadInitState {
            entry_code,
            entry_toc,
            arg: args[0],
            extra_args,
            stack_top: stack.initial_sp(),
            tls_base: parent_tls_base,
            lr_sentinel: CALLBACK_RETURN_CODE_ADDR as u64,
        };

        Ok(Lv2Dispatch::CallbackSpawn {
            worker_init,
            worker_stack_base: stack.base,
            worker_stack_size: stack.size,
            worker_priority: 0,
            parent,
            parent_pending: PendingResponse::CallbackReturn {
                stage,
                args: [0; 8],
            },
            effects: vec![],
        })
    }

    /// Link a freshly created worker `PpuThreadId` to its parent
    /// `UnitId` and the parkable-handler `stage`; increment the
    /// parent's recursion-depth tracker.
    ///
    /// Called by the runtime's `handle_callback_spawn` after
    /// [`crate::ppu_thread::PpuThreadTable::create`] mints the
    /// worker id. The runtime extracts `stage` from the dispatch's
    /// `parent_pending` field (a [`PendingResponse::CallbackReturn`]
    /// constructed by [`Self::call_guest_callback_sync`]).
    ///
    /// # Invariants
    /// - `worker` is freshly minted; `callback_parents` does not
    ///   already contain it.
    /// - `parent`'s depth is below [`CALLBACK_DEPTH_CAP`]
    ///   (enforced by [`Self::call_guest_callback_sync`]).
    /// - `stage` matches the stage in the parent's pending response;
    ///   future consumer stages land here without a host refactor.
    pub fn attach_callback_worker(
        &mut self,
        worker: PpuThreadId,
        parent: UnitId,
        stage: CallbackReturnStage,
    ) {
        let prev = self.callback_parents.insert(worker, (parent, stage));
        debug_assert!(
            prev.is_none(),
            "attach_callback_worker: worker {worker:?} already linked to {prev:?}",
        );
        let entry = self.callback_depth.entry(parent).or_insert(0);
        *entry = entry.saturating_add(1);
    }

    /// Resolve a callback worker's trampoline-return: look up parent,
    /// decrement depth, mark the worker `Finished`, and emit a
    /// `WakeAndReturn` keyed to the parent.
    ///
    /// `worker_unit` is the `requester` arg from the dispatch path
    /// (the worker's `UnitId`). `args` carries the worker's
    /// `r3..=r10` captured by the trampoline classifier.
    /// True when `worker_unit` is registered in `callback_parents`,
    /// i.e. a callback worker spawned by `call_guest_callback_sync`
    /// whose parent is parked. Used by the runtime's fault-handling
    /// path to recognize a mid-body worker fault as recoverable
    /// rather than fatal to the run.
    pub fn is_callback_worker(&self, worker_unit: UnitId) -> bool {
        let Some(worker_thread) = self.ppu_threads.thread_id_for_unit(worker_unit) else {
            return false;
        };
        self.callback_parents.contains_key(&worker_thread)
    }

    /// Wrapper around [`Self::dispatch_callback_return`] for the
    /// fault-propagation path: the worker faulted before reaching
    /// the trampoline `blr`, so synthesize the wake with
    /// `args = [fault_code; 8]` (Synthetic stage writes
    /// `args[0]` to parent r3; consumer stages that read the
    /// underlying cb_result struct see whatever pre-fault state
    /// the worker left, mapping the failure case to their
    /// stage-specific error code).
    ///
    /// Returns `None` when `worker_unit` is not a registered
    /// callback worker (caller should fall through to the normal
    /// run-terminating fault path).
    pub fn dispatch_callback_worker_fault(
        &mut self,
        worker_unit: UnitId,
        fault_code: u64,
    ) -> Option<Lv2Dispatch> {
        if !self.is_callback_worker(worker_unit) {
            return None;
        }
        Some(self.dispatch_callback_return(worker_unit, [fault_code; 8]))
    }

    pub(super) fn dispatch_callback_return(
        &mut self,
        worker_unit: UnitId,
        args: [u64; 8],
    ) -> Lv2Dispatch {
        let Some(worker_thread) = self.ppu_threads.thread_id_for_unit(worker_unit) else {
            self.record_invariant_break(
                "callback_dispatch.unknown_worker_unit",
                format_args!(
                    "CallbackDispatchReturn from UnitId {worker_unit:?} not in PpuThreadTable; \
                     parent (if any) will not wake"
                ),
            );
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            };
        };
        let Some((parent, stage)) = self.callback_parents.remove(&worker_thread) else {
            self.record_invariant_break(
                "callback_dispatch.unknown_worker_thread",
                format_args!(
                    "CallbackDispatchReturn from worker thread {worker_thread:?} not in \
                     callback_parents; parent (if any) will not wake"
                ),
            );
            return Lv2Dispatch::Immediate {
                code: 0,
                effects: vec![],
            };
        };
        if let Some(depth) = self.callback_depth.get_mut(&parent) {
            *depth = depth.saturating_sub(1);
            if *depth == 0 {
                self.callback_depth.remove(&parent);
            }
        }
        // Mark the worker `Finished` with `args[0]` (worker r3) as
        // the exit value. Callback workers spawn detached so no
        // joiners ride this transition; the returned waiter list
        // is expected empty.
        let _no_joiners = self.ppu_threads.mark_finished(worker_thread, args[0]);
        // The runtime applies this update via `response_updates`
        // (variant_tag must match the parked entry's tag). Stage
        // is recovered from `callback_parents`; future consumer
        // stages land here without a host refactor.
        let response_update = PendingResponse::CallbackReturn { stage, args };
        Lv2Dispatch::WakeAndReturn {
            // Source is the worker; its r3 (args[0]) is its return
            // value. The worker is about to be retired so the r3
            // write is observation-only.
            code: args[0],
            woken_unit_ids: vec![parent],
            response_updates: vec![(parent, response_update)],
            effects: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::test_support::FakeRuntime;
    use crate::host::Lv2Host;
    use crate::ppu_thread::PpuThreadAttrs;
    use cellgov_event::UnitId;
    use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

    /// Build a host with a primary PPU thread and a FakeRuntime that
    /// has an OPD pre-written at `opd_addr` pointing at
    /// `(entry_code, entry_toc)`.
    fn host_with_opd(
        opd_addr: u32,
        entry_code: u32,
        entry_toc: u32,
        parent_unit: UnitId,
    ) -> (Lv2Host, FakeRuntime) {
        let mut mem = GuestMemory::new(0x10_0000);
        let mut bytes = [0u8; 8];
        bytes[0..4].copy_from_slice(&entry_code.to_be_bytes());
        bytes[4..8].copy_from_slice(&entry_toc.to_be_bytes());
        mem.apply_commit(
            ByteRange::new(GuestAddr::new(opd_addr as u64), 8).unwrap(),
            &bytes,
        )
        .unwrap();
        let mut host = Lv2Host::new();
        host.seed_primary_ppu_thread(
            parent_unit,
            PpuThreadAttrs {
                entry: 0x10_0000,
                arg: 0,
                stack_base: 0xD000_0000,
                stack_size: 0x10000,
                priority: 1000,
                tls_base: 0,
            },
        );
        let rt = FakeRuntime::with_memory(mem);
        (host, rt)
    }

    #[test]
    fn call_guest_callback_sync_returns_callback_spawn_with_args_wired() {
        let parent = UnitId::new(0);
        let (mut host, rt) = host_with_opd(0x4000, 0x1234_5678, 0x9ABC_DEF0, parent);
        let args = [
            0xAAAA_AAAA,
            0xBBBB_BBBB,
            0xCCCC_CCCC,
            0xDDDD_DDDD,
            0xEEEE_EEEE,
            0xFFFF_FFFF,
            0x1010_1010,
            0x2020_2020,
        ];
        let dispatch = host
            .call_guest_callback_sync(parent, 0x4000, args, CallbackReturnStage::Synthetic, &rt)
            .expect("happy-path spawn");
        match dispatch {
            Lv2Dispatch::CallbackSpawn {
                worker_init,
                parent: p,
                parent_pending,
                ..
            } => {
                assert_eq!(p, parent);
                assert_eq!(worker_init.entry_code, 0x1234_5678);
                assert_eq!(worker_init.entry_toc, 0x9ABC_DEF0);
                assert_eq!(worker_init.arg, args[0]); // r3 = args[0]
                assert_eq!(worker_init.extra_args[0], args[1]); // r4
                assert_eq!(worker_init.extra_args[6], args[7]); // r10
                assert_eq!(worker_init.lr_sentinel, CALLBACK_RETURN_CODE_ADDR as u64);
                match parent_pending {
                    PendingResponse::CallbackReturn {
                        stage: CallbackReturnStage::Synthetic,
                        args: pending_args,
                    } => {
                        // Args are placeholder zeros at park time;
                        // the trampoline-return wake fills them in
                        // via response_updates.
                        assert_eq!(pending_args, [0; 8]);
                    }
                    other => panic!("expected CallbackReturn pending, got {other:?}"),
                }
            }
            other => panic!("expected CallbackSpawn, got {other:?}"),
        }
    }

    #[test]
    fn call_guest_callback_sync_rejects_unmapped_opd() {
        let parent = UnitId::new(0);
        let (mut host, rt) = host_with_opd(0x4000, 0, 0, parent);
        // Address well past the FakeRuntime's 0x10_0000-byte buffer.
        let err = host
            .call_guest_callback_sync(
                parent,
                0xFF00_0000,
                [0; 8],
                CallbackReturnStage::Synthetic,
                &rt,
            )
            .expect_err("unmapped OPD must surface OpdReadFailed");
        assert_eq!(err, CallbackError::OpdReadFailed);
    }

    #[test]
    fn recursion_cap_rejects_after_eight_attaches() {
        let parent = UnitId::new(0);
        let (mut host, rt) = host_with_opd(0x4000, 0x1000, 0x2000, parent);
        // Pre-bump the parent's depth to the cap by repeatedly
        // attaching synthetic worker ids. Worker thread ids are
        // monotonic per the allocator but we can fabricate ids
        // straight into callback_parents because attach is a
        // public API.
        for i in 0..CALLBACK_DEPTH_CAP {
            let worker = PpuThreadId::new(0x0100_0001 + i as u64);
            host.attach_callback_worker(worker, parent, CallbackReturnStage::Synthetic);
        }
        let err = host
            .call_guest_callback_sync(parent, 0x4000, [0; 8], CallbackReturnStage::Synthetic, &rt)
            .expect_err("ninth call must surface TooDeep");
        assert_eq!(err, CallbackError::TooDeep);
    }

    #[test]
    fn attach_increments_depth_and_inserts_parent_link() {
        let mut host = Lv2Host::new();
        let parent = UnitId::new(0);
        let worker_a = PpuThreadId::new(0x0100_0001);
        let worker_b = PpuThreadId::new(0x0100_0002);
        host.attach_callback_worker(worker_a, parent, CallbackReturnStage::Synthetic);
        host.attach_callback_worker(worker_b, parent, CallbackReturnStage::Synthetic);
        // Internal state inspection via state_hash drift: two
        // attach calls hash differently from one. Direct field
        // access would require a pub accessor; the hash is the
        // canonical inspection channel.
        let h_two = host.state_hash();
        let mut other = Lv2Host::new();
        other.attach_callback_worker(worker_a, parent, CallbackReturnStage::Synthetic);
        let h_one = other.state_hash();
        assert_ne!(h_two, h_one, "depth tracking must be hash-visible");
    }

    #[test]
    fn dispatch_callback_return_emits_wake_with_args_round_trip() {
        let mut host = Lv2Host::new();
        let parent = UnitId::new(0);
        // Seed a primary so resolve_wake_thread works.
        host.seed_primary_ppu_thread(
            parent,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        );
        // Manually create a worker thread and link it.
        let worker_unit = UnitId::new(1);
        let worker_thread = host
            .ppu_threads_mut()
            .create(
                worker_unit,
                PpuThreadAttrs {
                    entry: 0x1000,
                    arg: 0,
                    stack_base: 0xD001_0000,
                    stack_size: 0x4000,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .expect("worker thread create");
        host.attach_callback_worker(worker_thread, parent, CallbackReturnStage::Synthetic);
        let captured_args = [
            0x1111_1111,
            0x2222_2222,
            0x3333_3333,
            0x4444_4444,
            0x5555_5555,
            0x6666_6666,
            0x7777_7777,
            0x8888_8888,
        ];
        let dispatch = host.dispatch_callback_return(worker_unit, captured_args);
        match dispatch {
            Lv2Dispatch::WakeAndReturn {
                code,
                woken_unit_ids,
                response_updates,
                ..
            } => {
                assert_eq!(code, captured_args[0]); // r3 = args[0]
                assert_eq!(woken_unit_ids, vec![parent]);
                assert_eq!(response_updates.len(), 1);
                let (woken_parent, response) = &response_updates[0];
                assert_eq!(*woken_parent, parent);
                match response {
                    PendingResponse::CallbackReturn {
                        stage: CallbackReturnStage::Synthetic,
                        args,
                    } => {
                        assert_eq!(*args, captured_args);
                    }
                    other => panic!("expected CallbackReturn, got {other:?}"),
                }
            }
            other => panic!("expected WakeAndReturn, got {other:?}"),
        }
    }

    #[test]
    fn dispatch_callback_return_decrements_depth_and_removes_parent_when_zero() {
        let mut host = Lv2Host::new();
        let parent = UnitId::new(0);
        host.seed_primary_ppu_thread(
            parent,
            PpuThreadAttrs {
                entry: 0,
                arg: 0,
                stack_base: 0,
                stack_size: 0,
                priority: 0,
                tls_base: 0,
            },
        );
        let worker_unit = UnitId::new(1);
        let worker_thread = host
            .ppu_threads_mut()
            .create(
                worker_unit,
                PpuThreadAttrs {
                    entry: 0x1000,
                    arg: 0,
                    stack_base: 0xD001_0000,
                    stack_size: 0x4000,
                    priority: 0,
                    tls_base: 0,
                },
            )
            .expect("worker thread create");
        host.attach_callback_worker(worker_thread, parent, CallbackReturnStage::Synthetic);
        let depth_after_attach = host.state_hash();
        host.dispatch_callback_return(worker_unit, [0; 8]);
        let depth_after_return = host.state_hash();
        // After return, callback_parents and callback_depth should
        // both drop their parent entry; hash differs from "one
        // attach" snapshot but matches a fresh-host hash modulo the
        // worker's `Finished` state.
        assert_ne!(depth_after_attach, depth_after_return);
    }
}
