//! LV2 syscall dispatch: classify a `YieldReason::Syscall`, route it
//! through `Lv2Host::dispatch`, and fold the result back into runtime
//! state (syscall response, registry status, mailbox push/wake).
//! `handle_ppu_thread_create` lives in `ppu_create.rs`.

use std::collections::BTreeMap;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionStepResult, UnitStatus};
use cellgov_lv2::{Lv2Dispatch, PendingResponse, SpuInitState};

use super::{trace_bridge::MemoryView, Runtime};

impl Runtime {
    /// Apply LV2-produced effects: memory commits, mailbox FIFO push +
    /// blocked-unit wake, and RSX flip requests. `commit_step`'s
    /// per-boundary hook then drives WAITING -> DONE on the next
    /// boundary, matching the NV4097_FLIP_BUFFER observable shape.
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
                Effect::RsxFlipRequest { buffer_index } => {
                    self.rsx_flip.request_flip(*buffer_index);
                }
                _ => {}
            }
        }
    }

    /// Classify a syscall yield, dispatch through the LV2 host, and
    /// fold the result back into runtime state.
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

        // Timer syscalls advance the deterministic guest clock. CellGov
        // is not real-time; instead, every `sys_timer_sleep` /
        // `sys_timer_usleep` call adds the requested duration to
        // `self.time` so subsequent `sys_time_get_system_time` calls
        // reflect the simulated wall time. No yield, no waiter list:
        // other PPU threads see the new time on their next read.
        const SYS_TIMER_USLEEP: u64 = 141;
        const SYS_TIMER_SLEEP: u64 = 142;
        match args[0] {
            SYS_TIMER_USLEEP => {
                let usec = args[1];
                self.advance_guest_time_by_us(usec);
                self.registry.set_syscall_return(source, 0);
                return;
            }
            SYS_TIMER_SLEEP => {
                let seconds = args[1];
                let usec = seconds.saturating_mul(1_000_000);
                self.advance_guest_time_by_us(usec);
                self.registry.set_syscall_return(source, 0);
                return;
            }
            _ => {}
        }

        let request = cellgov_lv2::request::classify(
            args[0],
            &[
                args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
            ],
        );
        self.dispatch_lv2_request(request, source);
    }

    /// Advance `self.time` by `usec` microseconds. Used by the timer
    /// syscalls. Saturates at `u64::MAX` ticks; the simulated clock
    /// never wraps because the test scenarios that push it past
    /// `u64::MAX` do not exist.
    fn advance_guest_time_by_us(&mut self, usec: u64) {
        // 1 tick = 1 nanosecond (cellgov_time::SIMULATED_INSTRUCTIONS_PER_SECOND).
        let delta_ticks = usec.saturating_mul(1_000);
        let new_raw = self.time.raw().saturating_add(delta_ticks);
        self.time = cellgov_time::GuestTicks::new(new_raw);
    }

    /// Route a typed LV2 request through the host and hand each
    /// `Lv2Dispatch` variant to its per-variant handler.
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
                if is_process_exit {
                    // Every other unit transitions to Finished, which
                    // is a state change others can observe.
                    self.step_woke_others = true;
                }
                self.handle_immediate(source, code, effects, is_process_exit);
            }
            Lv2Dispatch::RegisterSpu {
                inits,
                effects,
                code,
            } => {
                if !inits.is_empty() {
                    self.step_woke_others = true;
                }
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
                lwmutex_inheritors,
                effects,
            } => {
                if !woken_unit_ids.is_empty() || !lwmutex_inheritors.is_empty() {
                    self.step_woke_others = true;
                }
                self.handle_ppu_thread_exit(
                    source,
                    exit_value,
                    woken_unit_ids,
                    lwmutex_inheritors,
                    effects,
                );
            }
            Lv2Dispatch::PpuThreadCreate { .. } => {
                // A fresh thread enters Runnable; treat as a wake so
                // the scheduler rotates rather than sticking to the
                // creator.
                self.step_woke_others = true;
                self.handle_ppu_thread_create(source, dispatch);
            }
            Lv2Dispatch::WakeAndReturn {
                code,
                woken_unit_ids,
                response_updates,
                effects,
            } => {
                if !woken_unit_ids.is_empty() {
                    self.step_woke_others = true;
                }
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
                if !woken_unit_ids.is_empty() {
                    self.step_woke_others = true;
                }
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

    /// `ProcessExit` transitions every unit to `Finished` and winds
    /// down any parked syscall responses; other syscalls write `code`
    /// into r3 of the source unit.
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
                // UnknownUnit (non-SPU) and AlreadyFinished (self-finished
                // SPU) are expected during the per-unit sweep; other Err
                // variants are dispatch-layer bugs.
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
                // A unit may or may not have a pending response; None
                // is legitimate on process exit.
                let _ = self.syscall_responses.try_take(*uid);
            }
        } else {
            self.registry.set_syscall_return(source, code);
        }
    }

    /// Register each SPU init via the `SpuFactory`, record it in
    /// `Lv2Host`, allocate a matching mailbox slot, and return `code`
    /// to the caller. `BTreeMap` iteration keeps registration order
    /// byte-stable across runs.
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

    /// Install `pending` on the syscall response table and flip status
    /// to Blocked; the eventual wake resolves `pending` and writes r3.
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

    /// Flip source to Finished and wake each join waiter: consume its
    /// `PpuThreadJoin` pending response, write the exit value through
    /// the out pointer, return CELL_OK via r3. Waiters without a
    /// matching pending response wake with the raw exit value in r3
    /// as a defensive fallback.
    fn handle_ppu_thread_exit(
        &mut self,
        source: UnitId,
        exit_value: u64,
        woken_unit_ids: Vec<UnitId>,
        lwmutex_inheritors: Vec<UnitId>,
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
                // Fallback for a waiter parked without a matching
                // PpuThreadJoin entry; wake with the raw exit value
                // in r3 rather than writing to an out pointer.
                self.registry.set_syscall_return(waiter, exit_value);
            }
            self.registry
                .set_status_override(waiter, UnitStatus::Runnable);
        }
        // Sync waiters whose held lwmutexes were transferred by the
        // exit go through the normal sync-wake path so their
        // `LwMutexWake` pending response repairs the user-space
        // struct (decrement waiter, set owner = inheritor,
        // recursive_count = 1).
        if !lwmutex_inheritors.is_empty() {
            self.resolve_sync_wakes(&lwmutex_inheritors);
        }
    }

    /// Release path: caller returns `code` and stays runnable, each
    /// waiter pulls its pending response and transitions Blocked ->
    /// Runnable. Overrides replace (not merge) the existing entry;
    /// [`Self::assert_response_updates_valid`] enforces that every
    /// updated unit is in `woken_unit_ids` and each update's variant
    /// matches the existing entry.
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
            // Partial-fill update path (e.g. EventQueueReceive None
            // -> Some, EventFlagWake observed=0 -> observed=bits_at_wake):
            // drain the parked entry before inserting the replacement
            // so the insert contract holds. The variant-tag check
            // above confirmed shape compatibility.
            let _ = self.syscall_responses.try_take(waiter);
            let _ = self.syscall_responses.insert(waiter, response);
        }
        self.resolve_sync_wakes(&woken_unit_ids);
    }

    /// Park-and-release (e.g. cond_wait): wake held waiters first, then
    /// park source on `pending` and flip it Blocked. No r3 is set here;
    /// the eventual wake that resolves `pending` writes r3.
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
            let _ = self.syscall_responses.try_take(waiter);
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

    /// Debug-only: every updated unit is in the wake set, and each
    /// update's variant matches the existing pending entry.
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

/// Extracted from [`Runtime::assert_response_updates_valid`] so tests
/// can trigger the invariants without standing up a full `Runtime`.
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
        // ReturnCode is the universal "wake with this r3, no payload"
        // override (cancel paths, timeout paths). It is allowed to
        // replace any prior variant; the variant-tag invariant only
        // applies to payload-carrying refinements.
        if matches!(update, PendingResponse::ReturnCode { .. }) {
            continue;
        }
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
        check_response_updates("test", &table, &[], &updates);
    }

    #[test]
    #[should_panic(expected = "variant mismatch")]
    fn check_response_updates_rejects_variant_mismatch() {
        // Two payload-carrying variants must not swap: a partial
        // refinement that switches shape would feed wrong data into
        // the wake side.
        let mut table = SyscallResponseTable::new();
        let waiter = UnitId::new(7);
        let _ = table.insert(
            waiter,
            PendingResponse::EventQueueReceive {
                out_ptr: 0x1000,
                payload: None,
            },
        );
        let updates = vec![(
            waiter,
            PendingResponse::EventFlagWake {
                result_ptr: 0x1000,
                observed: 0,
            },
        )];
        check_response_updates("test", &table, &[waiter], &updates);
    }

    #[test]
    fn check_response_updates_allows_return_code_to_replace_any_variant() {
        // ReturnCode is the universal cancel/timeout override: it
        // carries no payload and is allowed to replace whatever the
        // waiter parked on.
        let mut table = SyscallResponseTable::new();
        let waiter = UnitId::new(7);
        let _ = table.insert(
            waiter,
            PendingResponse::EventFlagWake {
                result_ptr: 0x1000,
                observed: 0,
            },
        );
        let updates = vec![(waiter, PendingResponse::ReturnCode { code: 0x80010013 })];
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
