//! The per-dispatch handlers that fold a result back into responses, registry status and wakes.

use std::collections::BTreeMap;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::{PendingResponse, SpuInitState, SpuLoadImage};
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
            self.register_spu_inbound_mailbox(uid);
            self.connect_spu_thread_interrupt_mailbox(uid);
            self.load_saved_signals(uid);
        }
        self.step_woke_others = true;
        self.deliver_syscall_return(source, code);
    }

    /// Write the signal-notification values the kernel keeps for the
    /// thread the new SPU `uid` runs into its registers. A register with
    /// no kept value stays empty (tests/micro/spu_snr_run_save).
    fn load_saved_signals(&mut self, uid: UnitId) {
        let saved = self.lv2_host.thread_groups().saved_signals_of_unit(uid);
        for (register, value) in [
            (cellgov_exec::SignalNotifier::One, saved[0]),
            (cellgov_exec::SignalNotifier::Two, saved[1]),
        ] {
            let Some(value) = value else {
                continue;
            };
            if let Err(refusal) = self.write_unit_signal(uid, register, value) {
                // The unit was registered a moment ago, so a refusal
                // names a host-side disagreement with the registry.
                self.lv2_host.log_invariant_break(
                    "runtime.load_saved_signals_refused",
                    format_args!("SPU {uid:?} refused its kept signal {register:?}: {refusal}"),
                );
            }
        }
    }

    /// Make LV2 the reader of the new SPU thread `uid`'s outbound
    /// interrupt mailbox, through which the thread asks for user events.
    fn connect_spu_thread_interrupt_mailbox(&mut self, uid: UnitId) {
        let connected = self
            .registry
            .get_mut(uid)
            .map(|spu| spu.connect_out_intr_mbox());
        if !matches!(connected, Some(Ok(()))) {
            self.lv2_host.log_invariant_break(
                "runtime.spu_thread_interrupt_mailbox_unconnected",
                format_args!("SPU thread {uid:?} refused its interrupt mailbox: {connected:?}"),
            );
        }
    }

    /// Give the new SPU `uid` its inbound mailbox, which shares its id.
    fn register_spu_inbound_mailbox(&mut self, uid: UnitId) {
        /// SPU Read Inbound Mailbox depth is 4 per
        /// [CBE-Handbook p:533 s:19.6 Table 19-15]; we use it
        /// as the default capacity for dispatch-allocated
        /// mailboxes until the SPU exec layer differentiates
        /// outbound (depth 1) from inbound (depth 4).
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
                "SPU registration for UnitId({:?}) found an existing mailbox; \
                     the dispatch layer must allocate a fresh unit id per SPU",
                uid.raw()
            );
        }
    }

    /// Registers the SPU of the new Raw SPU `id`, stopped with an empty
    /// local store, and enters it in the host's Raw SPU table.
    ///
    /// A Raw SPU runs only once its problem-state run control starts it.
    pub(super) fn handle_raw_spu_create(&mut self, source: UnitId, id: u32, effects: Vec<Effect>) {
        let Some(factory) = &self.spu_factory else {
            self.lv2_host.log_invariant_break(
                "runtime.spu_factory_missing",
                format_args!("Raw SPU {id} created without a factory"),
            );
            return self
                .deliver_syscall_return(source, cellgov_ps3_abi::lv2::errno::CELL_ENOSYS.into());
        };
        let init = SpuInitState {
            image: SpuLoadImage::Segments(Vec::new()),
            entry_pc: 0,
            stack_ptr: 0,
            args: [0; 4],
            group_id: 0,
        };
        let uid = match self
            .registry
            .try_register_dynamic(&|uid| factory(uid, init.clone()))
        {
            Ok(uid) => uid,
            Err(reason) => {
                self.lv2_host.log_invariant_break(
                    "runtime.raw_spu_unit_failed",
                    format_args!("Raw SPU {id}: the factory refused an empty image: {reason}"),
                );
                return self.deliver_syscall_return(
                    source,
                    cellgov_ps3_abi::lv2::errno::CELL_ENOMEM.into(),
                );
            }
        };
        if let Err(err) = self.request_unit_stop(uid) {
            self.lv2_host.log_invariant_break(
                "runtime.raw_spu_stop_failed",
                format_args!("Raw SPU {id} ({uid:?}) refused its stop at create: {err}"),
            );
        }
        self.register_spu_inbound_mailbox(uid);
        self.lv2_host.record_raw_spu(id, uid);
        let caller_space = self.spaces.space_of(source);
        self.open_raw_spu_window(id, uid, caller_space);
        self.apply_lv2_effects(&effects, caller_space);
        self.deliver_syscall_return(source, 0);
    }

    /// Finishes the SPU of a destroyed Raw SPU and unmaps its window.
    pub(super) fn handle_raw_spu_destroy(&mut self, source: UnitId, id: u32, unit: UnitId) {
        self.close_raw_spu_window(id, unit);
        self.registry
            .set_status_override(unit, UnitStatus::Finished);
        // A deadline left behind would wake a finished unit.
        self.timer_wakes.cancel(unit);
        let _ = self.syscall_responses.try_take(unit);
        self.deliver_syscall_return(source, 0);
    }

    /// Takes the message in the Raw SPU's outbound interrupt mailbox, if
    /// it holds one, into its PU interrupt mailbox register, and writes
    /// the register's word at `value_ptr` in the caller's space.
    pub(super) fn handle_raw_spu_read_puint_mb(
        &mut self,
        source: UnitId,
        id: u32,
        unit: UnitId,
        value_ptr: u32,
    ) {
        self.note_problem_state_access(unit);
        let parked = self.registry.effective_status(unit) != Some(UnitStatus::Runnable);
        let message = match self.read_unit_out_intr_mbox(unit) {
            Ok(message) => message,
            Err(err) => {
                self.lv2_host.log_invariant_break(
                    "runtime.raw_spu_puint_mb_unreadable",
                    format_args!(
                        "Raw SPU {id} ({unit:?}) refused its interrupt mailbox read: {err}"
                    ),
                );
                None
            }
        };
        if parked && self.registry.effective_status(unit) == Some(UnitStatus::Runnable) {
            self.step_woke_others = true;
        }
        let Some(word) = self.lv2_host.load_raw_spu_puint_mb(id, message) else {
            self.lv2_host.log_invariant_break(
                "runtime.raw_spu_puint_mb_unknown",
                format_args!("Raw SPU {id} left the table during its puint_mb read"),
            );
            return self
                .deliver_syscall_return(source, cellgov_ps3_abi::lv2::errno::CELL_ESRCH.into());
        };
        let write = Effect::shared_write(
            cellgov_mem::ByteRange::contiguous_u32(value_ptr, 4),
            cellgov_effects::WritePayload::from_slice(&word.to_be_bytes()),
            source,
            self.time,
        );
        let caller_space = self.spaces.space_of(source);
        self.apply_lv2_effects(&[write], caller_space);
        self.deliver_syscall_return(source, 0);
    }

    /// Finishes the SPUs of a terminated group and wakes the group's joiners.
    pub(super) fn handle_spu_group_terminate(
        &mut self,
        source: UnitId,
        group_id: u32,
        finished_units: Vec<UnitId>,
    ) {
        for unit in finished_units {
            self.registry
                .set_status_override(unit, UnitStatus::Finished);
            // A terminate ends a suspended group too; the hold goes with it.
            self.registry.set_suspended(unit, false);
            // A deadline left behind would wake a finished unit.
            self.timer_wakes.cancel(unit);
            let _ = self.syscall_responses.try_take(unit);
        }
        self.wake_group_joiners(group_id);
        self.deliver_syscall_return(source, 0);
    }

    /// Holds a suspended group's units out of the runnable set, or
    /// releases a resumed group's. A unit keeps the status it had, and a
    /// wake that lands while it is held shows once it is released.
    pub(super) fn handle_spu_group_suspension(
        &mut self,
        source: UnitId,
        units: Vec<UnitId>,
        suspended: bool,
    ) {
        for unit in units {
            self.registry.set_suspended(unit, suspended);
        }
        self.deliver_syscall_return(source, 0);
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
                // The join collects the thread, so a later call that
                // names it answers CELL_ESRCH
                // (tests/micro/lv2_ppu_thread_answers).
                if let Some(tid) = self.lv2_host.ppu_threads().thread_id_for_unit(source) {
                    self.lv2_host.ppu_threads_mut().reap(tid);
                }
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
                    // success, as the lv2_ppu_thread_answers capture
                    // shows.
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
