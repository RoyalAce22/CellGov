//! LV2 syscall dispatch extracted from `runtime.rs`.
//!
//! Three entry points form a cohesive unit: each touches
//! `lv2_host`, `syscall_responses`, `registry`, and the
//! mailbox/memory registries together.
//!
//! - [`Runtime::dispatch_syscall`] classifies a `YieldReason::Syscall`
//!   into a typed `Lv2Request` (or routes the HLE-import range).
//! - [`Runtime::dispatch_lv2_request`] runs the request through
//!   `Lv2Host::dispatch`, then routes each resulting `Lv2Dispatch`
//!   variant to its own `handle_<variant>` method.
//! - [`Runtime::apply_lv2_effects`] replays the effect-stream the
//!   LV2 host side-produces (memory commits + mailbox push/wake)
//!   uniformly across every dispatch variant.
//!
//! `handle_ppu_thread_create` is called from here but its body
//! lives in `ppu_create.rs` because that one variant is 100 lines
//! by itself.

use std::collections::BTreeMap;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, UnitStatus};
use cellgov_lv2::{Lv2Dispatch, PendingResponse, SpuInitState};

use super::{trace_bridge::MemoryView, Runtime};

impl Runtime {
    /// Apply effects produced by an LV2 dispatch. Handles
    /// SharedWriteIntent (memory commit) and MailboxSend (FIFO push
    /// + blocked-SPU wake) uniformly across all dispatch variants.
    pub(super) fn apply_lv2_effects(&mut self, effects: &[Effect]) {
        for effect in effects {
            match effect {
                Effect::SharedWriteIntent { range, bytes, .. } => {
                    let _ = self.memory.apply_commit(*range, bytes.bytes());
                }
                Effect::MailboxSend {
                    mailbox, message, ..
                } => {
                    if let Some(mbox) = self.mailbox_registry.get_mut(*mailbox) {
                        mbox.send(message.raw());
                    }
                    let target = UnitId::new(mailbox.raw());
                    if self.registry.effective_status(target) == Some(UnitStatus::Blocked) {
                        self.registry
                            .set_status_override(target, UnitStatus::Runnable);
                    }
                }
                _ => {}
            }
        }
    }

    /// Classify a syscall yield, dispatch through the LV2 host, and
    /// handle the result (immediate return, SPU registration, or block).
    pub(super) fn dispatch_syscall(&mut self, result: &ExecutionStepResult, source: UnitId) {
        let Some(args) = &result.syscall_args else {
            return;
        };

        // HLE import stubs use syscall numbers >= 0x10000.
        if args[0] >= 0x10000 {
            let hle_index = (args[0] - 0x10000) as u32;
            let nid = self.hle.nids.get(&hle_index).copied().unwrap_or(0);
            self.dispatch_hle(source, nid, args);
            return;
        }

        let request = cellgov_lv2::request::classify(
            args[0],
            &[
                args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
            ],
        );
        self.dispatch_lv2_request(request, source);
    }

    /// Route a typed LV2 request through the host, then hand the
    /// resulting `Lv2Dispatch` to its per-variant handler. Exposed
    /// to tests that need to exercise specific dispatch paths
    /// without plumbing a full PPU yield.
    pub(crate) fn dispatch_lv2_request(
        &mut self,
        request: cellgov_lv2::Lv2Request,
        source: UnitId,
    ) {
        let is_process_exit = matches!(request, cellgov_lv2::Lv2Request::ProcessExit { .. });
        let dispatch = self.lv2_host.dispatch(
            request,
            source,
            &MemoryView {
                memory: &self.memory,
                current_tick: self.time,
            },
        );
        match dispatch {
            Lv2Dispatch::Immediate { code, effects } => {
                self.handle_immediate(source, code, effects, is_process_exit);
            }
            Lv2Dispatch::RegisterSpu {
                inits,
                effects,
                code,
            } => {
                self.handle_register_spu(source, inits, effects, code);
            }
            Lv2Dispatch::Block {
                pending, effects, ..
            } => {
                self.handle_block(source, pending, effects);
            }
            Lv2Dispatch::PpuThreadExit {
                exit_value,
                woken_unit_ids,
                effects,
            } => {
                self.handle_ppu_thread_exit(source, exit_value, woken_unit_ids, effects);
            }
            Lv2Dispatch::PpuThreadCreate { .. } => {
                self.handle_ppu_thread_create(source, dispatch);
            }
            Lv2Dispatch::WakeAndReturn {
                code,
                woken_unit_ids,
                response_updates,
                effects,
            } => {
                self.handle_wake_and_return(
                    source,
                    code,
                    woken_unit_ids,
                    response_updates,
                    effects,
                );
            }
            Lv2Dispatch::BlockAndWake {
                pending,
                woken_unit_ids,
                response_updates,
                effects,
                ..
            } => {
                self.handle_block_and_wake(
                    source,
                    pending,
                    woken_unit_ids,
                    response_updates,
                    effects,
                );
            }
        }
    }

    /// Apply an `Immediate` dispatch: commit LV2-produced effects,
    /// then either (a) for `ProcessExit`, transition every unit to
    /// `Finished` and wind down any parked syscall responses, or
    /// (b) for every other syscall, write `code` into r3 of the
    /// source unit.
    fn handle_immediate(
        &mut self,
        source: UnitId,
        code: u64,
        effects: Vec<Effect>,
        is_process_exit: bool,
    ) {
        self.apply_lv2_effects(&effects);
        if is_process_exit {
            let all_ids: Vec<UnitId> = self.registry.ids().collect();
            for uid in &all_ids {
                self.registry
                    .set_status_override(*uid, UnitStatus::Finished);
                // Process exit iterates every unit (PPU and SPU).
                // UnknownUnit (non-SPU) and AlreadyFinished (SPU
                // that already self-finished) are both expected;
                // other Err variants are dispatch-layer bugs.
                match self.lv2_host.notify_spu_finished(*uid) {
                    Ok(_)
                    | Err(cellgov_lv2::thread_group::NotifySpuFinishedError::UnknownUnit)
                    | Err(cellgov_lv2::thread_group::NotifySpuFinishedError::AlreadyFinished {
                        ..
                    }) => {}
                    Err(err) => {
                        eprintln!(
                            "lv2 host invariant break at process_exit.notify_spu_finished: \
                             unit {:?}: {err:?}",
                            uid,
                        );
                    }
                }
                // Every registered unit may or may not have a pending
                // response; legitimate None case on process exit. The
                // returned Option is intentionally discarded.
                let _ = self.syscall_responses.try_take(*uid);
            }
        } else {
            self.registry.set_syscall_return(source, code);
        }
    }

    /// Register each SPU init state as a new unit via the installed
    /// `SpuFactory`, record it in `Lv2Host`, allocate a matching
    /// mailbox slot, and return `code` to the caller.
    ///
    /// BTreeMap iteration yields slots in ascending order, so
    /// registration order is byte-stable across runs.
    fn handle_register_spu(
        &mut self,
        source: UnitId,
        inits: BTreeMap<u32, SpuInitState>,
        effects: Vec<Effect>,
        code: u64,
    ) {
        self.apply_lv2_effects(&effects);
        if let Some(factory) = &self.spu_factory {
            for (slot, init) in inits {
                let gid = init.group_id;
                let uid = self
                    .registry
                    .register_dynamic(&|id| factory(id, init.clone()));
                self.lv2_host.record_spu(uid, gid, slot).expect(
                    "record_spu rejected a freshly allocated unit: dispatch-layer \
                     corruption in the RegisterSpu path",
                );
                self.mailbox_registry
                    .register_at(cellgov_sync::MailboxId::new(uid.raw()));
            }
        }
        self.registry.set_syscall_return(source, code);
    }

    /// Park the source on a primitive: install `pending` on the
    /// syscall response table and flip status to Blocked. The
    /// eventual wake resolves `pending` and writes r3.
    fn handle_block(&mut self, source: UnitId, pending: PendingResponse, effects: Vec<Effect>) {
        self.apply_lv2_effects(&effects);
        let displaced = self.syscall_responses.insert(source, pending);
        debug_assert!(
            displaced.is_none(),
            "handle_block: source {source:?} already had a pending response: {displaced:?}"
        );
        self.registry
            .set_status_override(source, UnitStatus::Blocked);
    }

    /// The source PPU thread is gone. Flip its status to Finished
    /// and wake each join waiter -- each is expected to have a
    /// `PendingResponse::PpuThreadJoin` recorded when it blocked on
    /// sys_ppu_thread_join. Consume the pending response, write
    /// the exit value through its output pointer, and return
    /// CELL_OK via r3. Waiters without a matching pending response
    /// wake with the raw exit value in r3 as a defensive fallback.
    fn handle_ppu_thread_exit(
        &mut self,
        source: UnitId,
        exit_value: u64,
        woken_unit_ids: Vec<UnitId>,
        effects: Vec<Effect>,
    ) {
        self.apply_lv2_effects(&effects);
        self.registry
            .set_status_override(source, UnitStatus::Finished);
        for waiter in woken_unit_ids {
            let pending = self.syscall_responses.try_take(waiter);
            if let Some(PendingResponse::PpuThreadJoin { status_out_ptr, .. }) = pending {
                self.commit_bytes_at(status_out_ptr as u64, &exit_value.to_be_bytes());
                self.registry.set_syscall_return(waiter, 0);
            } else {
                // Only sys_ppu_thread_join parks a waiter on a PPU-
                // thread exit under the current sync surface. If a
                // future primitive parks differently this branch
                // wakes with the raw exit value in r3 rather than
                // writing to an out pointer.
                self.registry.set_syscall_return(waiter, exit_value);
            }
            self.registry
                .set_status_override(waiter, UnitStatus::Runnable);
        }
    }

    /// Release path: caller returns `code` and stays runnable. Apply
    /// per-waiter pending-response overrides, then resolve the
    /// wake list (each wake target pulls its pending response and
    /// transitions Blocked -> Runnable).
    ///
    /// Each override replaces the existing pending entry rather
    /// than merging. The debug-only
    /// [`Self::assert_response_updates_valid`] check enforces two
    /// invariants at apply time: every updated unit appears in
    /// `woken_unit_ids`, and each update's variant matches the
    /// existing entry's variant (see
    /// [`cellgov_lv2::PendingResponse::variant_tag`]).
    fn handle_wake_and_return(
        &mut self,
        source: UnitId,
        code: u64,
        woken_unit_ids: Vec<UnitId>,
        response_updates: Vec<(UnitId, PendingResponse)>,
        effects: Vec<Effect>,
    ) {
        self.apply_lv2_effects(&effects);
        self.registry.set_syscall_return(source, code);
        self.assert_response_updates_valid(
            "handle_wake_and_return",
            &woken_unit_ids,
            &response_updates,
        );
        for (waiter, response) in response_updates {
            // Displacement is the expected path for partial-fill
            // updates (e.g. EventQueueReceive None -> Some); the
            // variant-tag check above already confirmed the update
            // matches an existing entry, so no r3 is lost here.
            let _ = self.syscall_responses.insert(waiter, response);
        }
        self.resolve_sync_wakes(&woken_unit_ids);
    }

    /// Park-and-release path (used by e.g. cond_wait): wake any
    /// held waiters first, then park the source on `pending` and
    /// flip it Blocked. No r3 is set here; the eventual wake that
    /// resolves `pending` writes r3.
    fn handle_block_and_wake(
        &mut self,
        source: UnitId,
        pending: PendingResponse,
        woken_unit_ids: Vec<UnitId>,
        response_updates: Vec<(UnitId, PendingResponse)>,
        effects: Vec<Effect>,
    ) {
        self.apply_lv2_effects(&effects);
        self.assert_response_updates_valid(
            "handle_block_and_wake",
            &woken_unit_ids,
            &response_updates,
        );
        for (waiter, response) in response_updates {
            // See `handle_wake_and_return`: displacement is a
            // legitimate partial-fill update path.
            let _ = self.syscall_responses.insert(waiter, response);
        }
        self.resolve_sync_wakes(&woken_unit_ids);
        let displaced = self.syscall_responses.insert(source, pending);
        debug_assert!(
            displaced.is_none(),
            "handle_block_and_wake: source {source:?} already had a pending response: \
             {displaced:?}"
        );
        self.registry
            .set_status_override(source, UnitStatus::Blocked);
    }

    /// Debug-only guard for `response_updates`: every updated unit
    /// must be in the wake set, and each update's variant must
    /// match the existing pending entry's variant. Silent
    /// violations would otherwise mutate unrelated future waits or
    /// wake a unit with a wrong-shaped response.
    fn assert_response_updates_valid(
        &self,
        site: &'static str,
        woken_unit_ids: &[UnitId],
        response_updates: &[(UnitId, PendingResponse)],
    ) {
        if !cfg!(debug_assertions) {
            return;
        }
        check_response_updates(
            site,
            &self.syscall_responses,
            woken_unit_ids,
            response_updates,
        );
    }
}

/// Free-function body of the debug-only response-updates guard
/// used by [`Runtime::handle_wake_and_return`] /
/// [`Runtime::handle_block_and_wake`]. Extracted so tests can
/// trigger the invariants without standing up a full `Runtime`.
pub(crate) fn check_response_updates(
    site: &'static str,
    table: &crate::syscall_table::SyscallResponseTable,
    woken_unit_ids: &[UnitId],
    response_updates: &[(UnitId, PendingResponse)],
) {
    for (waiter, update) in response_updates {
        assert!(
            woken_unit_ids.contains(waiter),
            "{site}: response_updates entry for {waiter:?} is not in woken_unit_ids",
        );
        if let Some(existing) = table.peek(*waiter) {
            assert_eq!(
                existing.variant_tag(),
                update.variant_tag(),
                "{site}: response_updates variant mismatch for {waiter:?} \
                 (existing tag {}, update tag {})",
                existing.variant_tag(),
                update.variant_tag(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syscall_table::SyscallResponseTable;
    use cellgov_lv2::EventPayload;

    #[test]
    #[should_panic(expected = "is not in woken_unit_ids")]
    fn check_response_updates_rejects_update_for_non_woken_unit() {
        let table = SyscallResponseTable::new();
        let waiter = UnitId::new(42);
        let updates = vec![(waiter, PendingResponse::ReturnCode { code: 0 })];
        // woken_unit_ids does NOT include the waiter -- silent
        // mutation of an unrelated future wait would otherwise
        // follow. Guard must fire.
        check_response_updates("test", &table, &[], &updates);
    }

    #[test]
    #[should_panic(expected = "variant mismatch")]
    fn check_response_updates_rejects_variant_mismatch() {
        let mut table = SyscallResponseTable::new();
        let waiter = UnitId::new(7);
        // Waiter is parked on EventQueueReceive (tag 3).
        let _ = table.insert(
            waiter,
            PendingResponse::EventQueueReceive {
                out_ptr: 0x1000,
                payload: None,
            },
        );
        // Update tries to replace it with a ReturnCode (tag 0).
        let updates = vec![(waiter, PendingResponse::ReturnCode { code: 0 })];
        check_response_updates("test", &table, &[waiter], &updates);
    }

    #[test]
    fn check_response_updates_accepts_same_variant_fill() {
        let mut table = SyscallResponseTable::new();
        let waiter = UnitId::new(7);
        let _ = table.insert(
            waiter,
            PendingResponse::EventQueueReceive {
                out_ptr: 0x1000,
                payload: None,
            },
        );
        // Partial-fill update with the same variant and the same
        // out_ptr is the intended wake-side contract.
        let updates = vec![(
            waiter,
            PendingResponse::EventQueueReceive {
                out_ptr: 0x1000,
                payload: Some(EventPayload {
                    source: 0x11,
                    data1: 0x22,
                    data2: 0x33,
                    data3: 0x44,
                }),
            },
        )];
        check_response_updates("test", &table, &[waiter], &updates);
    }
}
