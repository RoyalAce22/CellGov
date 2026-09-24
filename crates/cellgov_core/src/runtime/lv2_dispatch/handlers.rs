//! The per-dispatch handlers that fold a result back into responses, registry status and wakes.

use std::collections::BTreeMap;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::{PendingResponse, SpuInitState};
use cellgov_trace::HostWriter;

use crate::runtime::Runtime;

impl Runtime {
    /// `ProcessExit` finishes every unit and drops parked responses.
    pub(super) fn handle_immediate(
        &mut self,
        source: UnitId,
        code: u64,
        effects: Vec<Effect>,
        is_process_exit: bool,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        if is_process_exit {
            let all_ids: Vec<UnitId> = self.registry.ids().collect();
            for uid in &all_ids {
                self.registry
                    .set_status_override(*uid, UnitStatus::Finished);
                // UnknownUnit (non-SPU) and AlreadyFinished are
                // expected during the per-unit sweep.
                let notified = self.lv2_host.notify_spu_finished(*uid);
                match notified {
                    Ok(_)
                    | Err(cellgov_lv2::thread_group::NotifySpuFinishedError::UnknownUnit)
                    | Err(cellgov_lv2::thread_group::NotifySpuFinishedError::AlreadyFinished {
                        ..
                    }) => {}
                    Err(err) => {
                        self.lv2_host.log_invariant_break(
                            "runtime.process_exit_notify_spu_finished_failed",
                            format_args!(
                                "notify_spu_finished rejected {uid:?} during the process-exit \
                                 sweep: {err:?}; the thread table and the SPU group state \
                                 disagree about this unit"
                            ),
                        );
                    }
                }
                // The parked response is dropped, so the unit's timer
                // entry must go with it: fire_timer_wakes runs later
                // in this same commit, and a deadline the exiting
                // step just crossed would otherwise wake a unit this
                // sweep finished (dropping a response without
                // cancelling its deadline is the invariant break
                // `Lv2Host::expire_wait` names).
                self.timer_wakes.cancel(*uid);
                let _ = self.syscall_responses.try_take(*uid);
            }
        } else {
            self.deliver_syscall_return(source, code);
        }
    }

    /// `BTreeMap` iteration keeps registration order byte-stable.
    pub(in crate::runtime) fn handle_register_spu(
        &mut self,
        source: UnitId,
        inits: BTreeMap<u32, SpuInitState>,
        effects: Vec<Effect>,
        code: u64,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        if inits.is_empty() {
            return self.deliver_syscall_return(source, code);
        }
        let group_id = inits
            .values()
            .next()
            .expect("nonempty init map checked above")
            .group_id;
        let Some(factory) = &self.spu_factory else {
            let rolled_back = self.lv2_host.cancel_unregistered_spu_group_start(group_id);
            self.lv2_host.log_invariant_break(
                "runtime.spu_factory_missing",
                format_args!(
                    "SPU group {group_id} started without a factory; rollback={rolled_back}"
                ),
            );
            return self
                .deliver_syscall_return(source, cellgov_ps3_abi::lv2::errno::CELL_ENOSYS.into());
        };

        // Construct the whole group before publishing any unit-to-group or
        // mailbox binding. A later slot can therefore fail without exposing a
        // partly started group to the guest.
        let mut created = Vec::with_capacity(inits.len());
        for (slot, init) in inits {
            let gid = init.group_id;
            match self
                .registry
                .try_register_dynamic(&|id| factory(id, init.clone()))
            {
                Ok(uid) => created.push((slot, gid, uid)),
                Err(reason) => {
                    for &(_, _, uid) in &created {
                        self.registry.set_status_override(uid, UnitStatus::Finished);
                    }
                    let rolled_back = self.lv2_host.cancel_unregistered_spu_group_start(group_id);
                    self.lv2_host.log_invariant_break(
                        "runtime.spu_image_load_failed",
                        format_args!(
                            "SPU group {group_id} factory rejected slot {slot}: {reason}; \
                             rollback={rolled_back}"
                        ),
                    );
                    return self.deliver_syscall_return(
                        source,
                        cellgov_ps3_abi::lv2::errno::CELL_EFAULT.into(),
                    );
                }
            }
        }

        for (slot, gid, uid) in created {
            self.lv2_host.record_spu(uid, gid, slot).expect(
                "record_spu rejected a freshly allocated unit: dispatch-layer \
                 corruption in the RegisterSpu path",
            );
            // SPU Read Inbound Mailbox depth is 4 per
            // [CBE-Handbook p:533 s:19.6 Table 19-15]; we use it
            // as the default capacity for dispatch-allocated
            // mailboxes until the SPU exec layer differentiates
            // outbound (depth 1) from inbound (depth 4).
            const SPU_INBOUND_MBOX_DEPTH: usize = 4;
            let inserted = self.mailbox_registry.register_at(
                cellgov_sync::MailboxId::new(uid.raw()),
                SPU_INBOUND_MBOX_DEPTH,
            );
            if !inserted {
                // Collision means the dispatch layer reused a
                // UnitId that already had a mailbox -- SPU
                // mailbox state would silently cross-talk
                // between units.
                self.lv2_host.log_invariant_break(
                    "runtime.register_spu_mailbox_id_collision",
                    format_args!(
                        "{uid:?} reused a live mailbox slot, so SPU mailbox state can cross \
                             between two units; every anchor needs revalidation if this fires"
                    ),
                );
                debug_assert!(
                    inserted,
                    "RegisterSpu for UnitId({:?}) found an existing mailbox; \
                         the dispatch layer must allocate a fresh unit id per SPU",
                    uid.raw()
                );
            }
        }
        self.step_woke_others = true;
        self.deliver_syscall_return(source, code);
    }

    pub(super) fn handle_block(
        &mut self,
        source: UnitId,
        pending: PendingResponse,
        effects: Vec<Effect>,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        let displaced = self.syscall_responses.insert(source, pending);
        if let Some(prev) = &displaced {
            self.lv2_host.log_invariant_break(
                "runtime.handle_block_pending_response_displaced",
                format_args!(
                    "{source:?} blocked again with {prev:?} still pending, so the earlier \
                     response is overwritten and its wake never reaches the guest"
                ),
            );
        }
        debug_assert!(
            displaced.is_none(),
            "handle_block: source {source:?} already had a pending response: {displaced:?}"
        );
        self.registry
            .set_status_override(source, UnitStatus::Blocked);
    }

    /// Waiters without a matching `PpuThreadJoin` response wake with
    /// the raw exit value in r3 instead of writing through the out
    /// pointer.
    pub(super) fn handle_ppu_thread_exit(
        &mut self,
        source: UnitId,
        exit_value: u64,
        woken_unit_ids: Vec<UnitId>,
        lwmutex_inheritors: Vec<UnitId>,
        effects: Vec<Effect>,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        self.registry
            .set_status_override(source, UnitStatus::Finished);
        for waiter in woken_unit_ids {
            self.timer_wakes.cancel(waiter);
            let pending = self.syscall_responses.try_take(waiter);
            if let Some(PendingResponse::PpuThreadJoin { status_out_ptr, .. }) = pending {
                // The out-pointer was decoded from the JOINER's
                // syscall, so it addresses the joiner's space.
                if status_out_ptr != 0 {
                    self.commit_bytes_at(
                        HostWriter::WakeContinuation,
                        waiter,
                        status_out_ptr as u64,
                        &exit_value.to_be_bytes(),
                    );
                    self.deliver_syscall_return(waiter, 0);
                } else {
                    // A NULL out-pointer is never written -- the join
                    // itself still completes (the target is reaped),
                    // but the joiner's r3 reports CELL_EFAULT, not
                    // success. Reap-then-fault ordering has no public
                    // attestation; a console probe of a NULL-vptr join
                    // would settle whether the target is still reaped.
                    self.deliver_syscall_return(
                        waiter,
                        cellgov_ps3_abi::lv2::errno::CELL_EFAULT.into(),
                    );
                }
            } else {
                self.deliver_syscall_return(waiter, exit_value);
            }
            self.registry
                .set_status_override(waiter, UnitStatus::Runnable);
        }
        // Inheritors route through the sync-wake path so their
        // `LwMutexWake` response repairs the user-space struct
        // (decrement waiter, set owner = inheritor, recursive_count = 1).
        if !lwmutex_inheritors.is_empty() {
            self.resolve_sync_wakes(&lwmutex_inheritors);
        }
    }

    /// Overrides replace the existing entry;
    /// [`Self::assert_response_updates_valid`] enforces the
    /// [`Lv2Dispatch::WakeAndReturn`](cellgov_lv2::Lv2Dispatch::WakeAndReturn) `response_updates` invariants.
    ///
    /// Cross-module contract: when the caller is a callback worker,
    /// `Lv2Host::dispatch_callback_return` has already transitioned
    /// its `PpuThread` to `Finished`. `is_ppu_thread_finished_for_unit`
    /// mirrors that into `UnitStatus::Finished` so the PPU loop does
    /// not fetch past the trampoline (which lands on OPD bytes and
    /// decode-faults).
    pub(super) fn handle_wake_and_return(
        &mut self,
        source: UnitId,
        code: u64,
        woken_unit_ids: Vec<UnitId>,
        response_updates: Vec<(UnitId, PendingResponse)>,
        effects: Vec<Effect>,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        self.deliver_syscall_return(source, code);
        self.assert_response_updates_valid(
            "handle_wake_and_return",
            &woken_unit_ids,
            &response_updates,
        );
        for (waiter, response) in response_updates {
            // Partial-fill refinement (e.g. EventQueueReceive
            // None -> Some): drain before re-insert so the insert
            // contract holds. Variant-tag check above guards shape.
            // A restage supersedes any timer deadline on the original
            // wait -- the cond two-hop converts a timed cond wait into
            // an untimed mutex wait.
            self.timer_wakes.cancel(waiter);
            let _ = self.syscall_responses.try_take(waiter);
            let _ = self.syscall_responses.insert(waiter, response);
        }
        self.resolve_sync_wakes(&woken_unit_ids);
        if self.lv2_host.is_ppu_thread_finished_for_unit(source) {
            self.registry
                .set_status_override(source, UnitStatus::Finished);
        }
    }

    /// Park-and-release (e.g. cond_wait): r3 is set by the eventual
    /// wake that resolves `pending`, not by this site.
    pub(super) fn handle_block_and_wake(
        &mut self,
        source: UnitId,
        pending: PendingResponse,
        woken_unit_ids: Vec<UnitId>,
        response_updates: Vec<(UnitId, PendingResponse)>,
        effects: Vec<Effect>,
    ) {
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&effects, caller_space);
        self.assert_response_updates_valid(
            "handle_block_and_wake",
            &woken_unit_ids,
            &response_updates,
        );
        for (waiter, response) in response_updates {
            self.timer_wakes.cancel(waiter);
            let _ = self.syscall_responses.try_take(waiter);
            let _ = self.syscall_responses.insert(waiter, response);
        }
        self.resolve_sync_wakes(&woken_unit_ids);
        let displaced = self.syscall_responses.insert(source, pending);
        if let Some(prev) = &displaced {
            self.lv2_host.log_invariant_break(
                "runtime.handle_block_and_wake_pending_response_displaced",
                format_args!(
                    "{source:?} blocked again with {prev:?} still pending, so the earlier \
                     response is overwritten and its wake never reaches the guest"
                ),
            );
        }
        debug_assert!(
            displaced.is_none(),
            "handle_block_and_wake: source {source:?} already had a pending response: \
             {displaced:?}"
        );
        self.registry
            .set_status_override(source, UnitStatus::Blocked);
    }
}
