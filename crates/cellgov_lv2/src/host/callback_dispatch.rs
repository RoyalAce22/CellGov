//! Worker-thread callback-dispatch primitive.
//!
//! Cross-crate contract: an HLE handler calls
//! [`Lv2Host::call_guest_callback_sync`] with a guest OPD pointer; the
//! host emits [`Lv2Dispatch::CallbackSpawn`]; the runtime mints a worker
//! [`PpuThreadId`] and re-enters via [`Lv2Host::attach_callback_worker`]
//! to record the worker -> parent link. The worker's terminal `blr`
//! sets `PC = LR` where LR was staged to
//! [`CALLBACK_RETURN_CODE_ADDR`] -- a code address, not an OPD --
//! landing on the trampoline body that issues a CellGov-private LV2
//! syscall classified as [`crate::Lv2Request::CallbackDispatchReturn`].
//! [`Lv2Host::dispatch_callback_return`] resolves the linkage,
//! decrements the recursion-depth tracker, and emits
//! [`Lv2Dispatch::WakeAndReturn`] keyed to the parent.
//!
//! Per-handler resume state rides in [`crate::CallbackReturnStage`]
//! inside [`crate::PendingResponse::CallbackReturn`]; the worker's
//! captured `r3..=r10` overwrite the placeholder `args` at wake time.

use cellgov_event::UnitId;
use cellgov_ps3_abi::callback_dispatch::{CALLBACK_DEPTH_CAP, CALLBACK_RETURN_CODE_ADDR};

use crate::dispatch::{CallbackReturnStage, Lv2Dispatch, PendingResponse, PpuThreadInitState};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::ppu_thread::PpuThreadId;

/// Failure modes for [`Lv2Host::call_guest_callback_sync`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackError {
    /// Parent is already at [`CALLBACK_DEPTH_CAP`].
    TooDeep,
    /// OPD pointer unmapped or short-read; no state mutated.
    OpdReadFailed,
    /// Child-stack arena exhausted.
    StackAllocFailed,
}

impl Lv2Host {
    /// Invoke a guest callback synchronously on a fresh worker PPU thread,
    /// parking `parent` until the worker returns through the trampoline.
    ///
    /// The worker inherits `parent`'s `tls_base` so `r13`-relative TLS
    /// dereferences (locale tables, errno) resolve; sharing is sound
    /// while `parent` is parked Blocked. `args[0..=7]` map to worker
    /// `r3..=r10`. `LR` is staged to [`CALLBACK_RETURN_CODE_ADDR`]
    /// -- the trampoline code address, not an OPD; `blr` branches to
    /// `LR` directly and an OPD would decode as garbage.
    ///
    /// On success returns [`Lv2Dispatch::CallbackSpawn`]; the runtime
    /// mints the worker id and re-enters [`Self::attach_callback_worker`].
    /// The parent's [`PendingResponse::CallbackReturn`] carries `stage`
    /// and zero-filled `args`; the trampoline-return wake overwrites
    /// `args` with the worker's captured `r3..=r10`.
    ///
    /// # Errors
    /// - [`CallbackError::TooDeep`]
    /// - [`CallbackError::OpdReadFailed`]
    /// - [`CallbackError::StackAllocFailed`]
    pub fn call_guest_callback_sync(
        &mut self,
        parent: UnitId,
        opd_addr: u32,
        args: [u64; 8],
        stage: CallbackReturnStage,
        rt: &dyn Lv2Runtime,
    ) -> Result<Lv2Dispatch, CallbackError> {
        let current_depth = self.callback_depth.get(&parent).copied().unwrap_or(0);
        if current_depth >= CALLBACK_DEPTH_CAP {
            return Err(CallbackError::TooDeep);
        }

        let parent_tls_base = self
            .ppu_threads
            .get_by_unit(parent)
            .map(|t| u64::from(t.attrs.tls_base))
            .unwrap_or(0);

        // OPD layout: u32 BE code_addr || u32 BE toc.
        // first_chunk::<8> folds the length check and the slice-to-array
        // conversion into one infallible-on-Some operation.
        let opd_bytes = rt
            .read_committed(opd_addr as u64, 8)
            .ok_or(CallbackError::OpdReadFailed)?;
        let opd: &[u8; 8] = opd_bytes
            .first_chunk::<8>()
            .ok_or(CallbackError::OpdReadFailed)?;
        let entry_code = u32::from_be_bytes([opd[0], opd[1], opd[2], opd[3]]) as u64;
        let entry_toc = u32::from_be_bytes([opd[4], opd[5], opd[6], opd[7]]) as u64;

        let stack = self
            .allocate_child_stack(0x4000, 0x10)
            .ok_or(CallbackError::StackAllocFailed)?;

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

    /// Link a freshly minted worker to its parent and bump the parent's
    /// recursion depth.
    ///
    /// # Invariants
    /// - `worker` is not yet in `callback_parents`.
    /// - `parent`'s depth is below [`CALLBACK_DEPTH_CAP`] (enforced
    ///   upstream by [`Self::call_guest_callback_sync`]).
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

    /// True when `worker_unit` is a registered callback worker whose
    /// parent is parked. The runtime's fault path uses this to treat a
    /// mid-body worker fault as recoverable rather than run-fatal.
    pub fn is_callback_worker(&self, worker_unit: UnitId) -> bool {
        let Some(worker_thread) = self.ppu_threads.thread_id_for_unit(worker_unit) else {
            return false;
        };
        self.callback_parents.contains_key(&worker_thread)
    }

    /// Fault-path wrapper: synthesize a wake with `args = [fault_code; 8]`.
    ///
    /// Synthetic stage writes `args[0]` to parent `r3`. Consumer stages
    /// reading their own cb_result struct see whatever pre-fault state
    /// the worker left; mapping fault to a stage-specific error code is
    /// the consumer's job.
    ///
    /// Returns `None` when `worker_unit` is not a registered callback
    /// worker; caller falls through to the run-terminating fault path.
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
        // Callback workers spawn detached; the waiter list is empty.
        let _no_joiners = self.ppu_threads.mark_finished(worker_thread, args[0]);
        let response_update = PendingResponse::CallbackReturn { stage, args };
        Lv2Dispatch::WakeAndReturn {
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
                assert_eq!(worker_init.arg, args[0]);
                assert_eq!(worker_init.extra_args[0], args[1]);
                assert_eq!(worker_init.extra_args[6], args[7]);
                assert_eq!(worker_init.lr_sentinel, CALLBACK_RETURN_CODE_ADDR as u64);
                match parent_pending {
                    PendingResponse::CallbackReturn {
                        stage: CallbackReturnStage::Synthetic,
                        args: pending_args,
                    } => {
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
                    stack_base: 0xD010_0000,
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
                assert_eq!(code, captured_args[0]);
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
                    stack_base: 0xD010_0000,
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
        assert_ne!(depth_after_attach, depth_after_return);
    }
}
