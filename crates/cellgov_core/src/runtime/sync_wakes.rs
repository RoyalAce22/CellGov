//! Wake protocol for blocked units: consume `PendingResponse`, commit
//! continuation payload, transition Blocked -> Runnable.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::PendingResponse;
use cellgov_trace::HostWriter;

use super::Runtime;

impl Runtime {
    /// Consume each woken unit's pending response and transition it
    /// Blocked -> Runnable.
    ///
    /// # Panics
    ///
    /// `EventQueueReceive { payload: None }`: four zero u64s would be
    /// indistinguishable from a real event.
    pub(super) fn resolve_sync_wakes(&mut self, woken_unit_ids: &[UnitId]) {
        for waiter in woken_unit_ids {
            let waiter = *waiter;
            // A Finished unit on the wake list is exited-process
            // residue: the exit sweep finishes every unit of the pid
            // but leaves the LV2 host's waiter lists unpurged, so a
            // later release can still pick the unit. On PS3 a process
            // exit deallocates everything the process owns, its PPU
            // and SPU threads included, so no thread of it survives to
            // take a wake. The Runnable transition below would
            // resurrect a unit the guest already terminated -- the
            // same reasoning as the Finished guard in
            // fire_timer_wakes. The release side consumed a waiter
            // slot on a dead unit, which can skew guest-visible
            // primitive state, so the drop is logged.
            if self.registry.effective_status(waiter) == Some(UnitStatus::Finished) {
                self.timer_wakes.cancel(waiter);
                // The pending response (already drained by the exit
                // sweep in the ordinary flow) can never be consumed.
                let _ = self.syscall_responses.try_take(waiter);
                self.lv2_host.log_invariant_break(
                    "runtime.resolve_sync_wakes_waiter_finished",
                    format_args!(
                        "wake for {waiter:?} which is already Finished (exited-process \
                         residue on a host-side waiter list); wake dropped, unit stays \
                         Finished",
                    ),
                );
                continue;
            }
            // Continuation payloads land through pointers the WAITER
            // supplied when it parked, so they resolve in its space.
            let waiter_space = self.spaces.space_of(waiter);
            // A wake through any path supersedes a pending timer
            // deadline; a stale entry would fire a second wake into a
            // unit that re-parked on something else.
            self.timer_wakes.cancel(waiter);
            let pending = self.syscall_responses.try_take(waiter);
            match pending {
                Some(PendingResponse::ReturnCode { code }) => {
                    self.deliver_syscall_return(waiter, code);
                }
                Some(PendingResponse::EventQueueReceive { out_ptr, payload }) => {
                    let payload = payload.unwrap_or_else(|| {
                        panic!(
                            "EventQueueReceive wake for {waiter:?} with unfilled payload \
                             (release-side dispatch forgot response_updates)"
                        )
                    });
                    // The event returns in r4..=r7; `out_ptr` is the
                    // dummy pointer the kernel never writes.
                    let _ = out_ptr;
                    for (reg, value) in cellgov_lv2::event_registers(&payload) {
                        self.registry.push_register_write(waiter, reg, value);
                    }
                    self.deliver_syscall_return(waiter, 0);
                }
                Some(PendingResponse::EventFlagWake {
                    result_ptr,
                    observed,
                }) => {
                    // The kernel stores the observed pattern only
                    // through a non-null result pointer; a waiter that
                    // passed NULL wakes with r3 alone. A console
                    // traces this in
                    // tests/ps3autotests/tests/lv2/sys_event_flag,
                    // whose wait helpers pass 0 and complete cleanly.
                    if result_ptr != 0 {
                        self.commit_bytes_at(
                            HostWriter::WakeContinuation,
                            waiter,
                            result_ptr as u64,
                            &observed.to_be_bytes(),
                        );
                    }
                    self.deliver_syscall_return(waiter, 0);
                }
                Some(PendingResponse::EventFlagCancelWake {
                    result_ptr,
                    observed,
                }) => {
                    // On cancel each drained waiter stores the pattern
                    // captured at the cancel call through its own
                    // non-null result pointer and returns
                    // CELL_ECANCELED.
                    if result_ptr != 0 {
                        self.commit_bytes_at(
                            HostWriter::WakeContinuation,
                            waiter,
                            result_ptr as u64,
                            &observed.to_be_bytes(),
                        );
                    }
                    self.deliver_syscall_return(
                        waiter,
                        cellgov_ps3_abi::lv2::errno::CELL_ECANCELED.into(),
                    );
                }
                Some(PendingResponse::LwMutexWake { mutex_ptr, caller }) => {
                    // `mutex_ptr == 0` is the raw LV2-syscall path with
                    // no user-space struct.
                    if mutex_ptr != 0 {
                        let base = mutex_ptr as u64;
                        // sys_lwmutex_t (24 bytes; see sys_lwmutex_create):
                        //   offset 0  : owner (u32 BE)
                        //   offset 4  : waiter count (u32 BE)
                        //   offset 12 : recursive_count (u32 BE)
                        self.commit_bytes_at(
                            HostWriter::WakeContinuation,
                            waiter,
                            base,
                            &caller.to_be_bytes(),
                        );
                        self.commit_bytes_at(
                            HostWriter::WakeContinuation,
                            waiter,
                            base + 12,
                            &1u32.to_be_bytes(),
                        );
                        let waiter_addr = base + 4;
                        // The user-space struct lives in the waking
                        // unit's space; read it back from there.
                        let bytes = super::spaces::resolve_space_memory(
                            &self.memory,
                            &self.spaces,
                            waiter_space,
                        )
                        .read(
                            cellgov_mem::ByteRange::new(
                                cellgov_mem::GuestAddr::new(waiter_addr),
                                4,
                            )
                            .expect("lwmutex_wake: bad waiter ByteRange"),
                        );
                        let current = u32::from_be_bytes(
                            bytes
                                .expect(
                                    "lwmutex wake: waiter slot read failed after owner \
                                     write succeeded; lwmutex_t crosses an unmapped \
                                     boundary or park-side validation regressed",
                                )
                                .first_chunk::<4>()
                                .copied()
                                .expect("4-byte read returned <4 bytes"),
                        );
                        debug_assert!(
                            current > 0,
                            "lwmutex wake: user-space waiter count already 0 at {waiter_addr:#x} \
                             (host waiter list diverged from guest struct)",
                        );
                        let next = current.saturating_sub(1);
                        self.commit_bytes_at(
                            HostWriter::WakeContinuation,
                            waiter,
                            waiter_addr,
                            &next.to_be_bytes(),
                        );
                    }
                    if let Some(tid) = self.lv2_host.ppu_thread_id_for_unit(waiter) {
                        self.lv2_host.lwmutex_holds_inc(tid);
                    }
                    self.deliver_syscall_return(waiter, 0);
                }
                Some(PendingResponse::CondWakeReacquire { .. }) => {
                    unreachable!(
                        "resolve_sync_wakes: CondWakeReacquire for {waiter:?} reached the \
                         wake resolver. The signal handler must swap to ReturnCode (or \
                         re-park on the mutex waiter list) before adding the waiter to \
                         woken_unit_ids; reaching here means the signal-side state \
                         machine is broken, and returning r3=0 would tell the cond_wait \
                         caller it acquired the mutex when it has not.",
                    );
                }
                Some(
                    PendingResponse::ThreadGroupJoin { .. } | PendingResponse::PpuThreadJoin { .. },
                ) => {
                    unreachable!(
                        "resolve_sync_wakes: join variant for {waiter:?}; join \
                         responses resolve through resolve_join_wakes",
                    );
                }
                None => {
                    // Missing pending response is a bug (release-side
                    // double-wake or park-side missing record). Without
                    // the release log, the unit would still transition
                    // Runnable below and leave guest r3 stale.
                    self.lv2_host.log_invariant_break(
                        "runtime.resolve_sync_wakes_no_pending_response",
                        format_args!(
                            "resolve_sync_wakes: {waiter:?} on the wake list with no pending \
                             response (release-side double-wake or park-side missing record); \
                             unit will transition Runnable regardless, so a missing-record \
                             cause leaves the guest's r3 stale on syscall return"
                        ),
                    );
                    debug_assert!(
                        false,
                        "resolve_sync_wakes: {waiter:?} on the wake list with no pending \
                         response (release-side double-wake or park-side missing record)",
                    );
                }
            }
            self.registry
                .set_status_override(waiter, UnitStatus::Runnable);
        }
    }

    #[cfg(test)]
    pub(crate) fn resolve_sync_wakes_for_test(&mut self, woken_unit_ids: &[UnitId]) {
        self.resolve_sync_wakes(woken_unit_ids);
    }

    #[cfg(test)]
    pub(crate) fn resolve_join_wakes_for_test(&mut self, source: UnitId) {
        self.resolve_join_wakes(source);
    }

    /// Notify the LV2 host that `source` finished; if the enclosing
    /// group is fully finished, wake any PPU blocked on its join.
    pub(super) fn resolve_join_wakes(&mut self, source: UnitId) {
        let notified = self.lv2_host.notify_spu_finished(source);
        let finished_group = match notified {
            Ok(Some(gid)) => gid,
            Ok(None) => return,
            Err(cellgov_lv2::thread_group::NotifySpuFinishedError::UnknownUnit) => return,
            Err(err) => {
                // The process-exit sweep excuses AlreadyFinished
                // because it notifies every unit; this site sees one
                // finish per unit, so a rejection here is a double
                // notify against a live unit.
                self.lv2_host.log_invariant_break(
                    "runtime.resolve_join_wakes_notify_spu_finished_failed",
                    format_args!(
                        "notify_spu_finished rejected {source:?}: {err:?}; no join waiter on \
                         this unit's group wakes from this finish"
                    ),
                );
                return;
            }
        };
        let waiters: Vec<UnitId> = self.syscall_responses.pending_ids().collect();
        for waiter_id in waiters {
            let is_match = self
                .syscall_responses
                .peek(waiter_id)
                .map(|p| {
                    matches!(p, PendingResponse::ThreadGroupJoin { group_id, .. } if *group_id == finished_group)
                })
                .unwrap_or(false);
            if !is_match {
                continue;
            }
            // `take_expected` so an intervening drain panics rather
            // than silently falling through. Runtime is single-threaded,
            // so peek and take_expected see the same variant.
            self.timer_wakes.cancel(waiter_id);
            let pending = self.syscall_responses.take_expected(waiter_id);
            let PendingResponse::ThreadGroupJoin {
                code,
                cause_ptr,
                status_ptr,
                cause,
                status,
                ..
            } = pending
            else {
                unreachable!(
                    "resolve_join_wakes: peek matched ThreadGroupJoin but take_expected \
                     returned {pending:?} for {waiter_id:?}",
                );
            };
            // Both out-pointers came from the joiner's syscall
            // arguments, so they address the joiner's space. The join
            // itself completes regardless, but NULL out-pointers
            // change the outcome, and the check happens after the
            // wait: a NULL cause writes nothing -- not even a non-NULL
            // status -- and returns CELL_EFAULT; a NULL status alone
            // still writes cause and returns CELL_EFAULT. The
            // hardware's answer here is unestablished, so the
            // asymmetry is a witnessed choice.
            // Address 0 may be mapped, so NULL is never written
            // through.
            let code = if cause_ptr == 0 {
                cellgov_ps3_abi::lv2::errno::CELL_EFAULT.into()
            } else {
                self.commit_bytes_at(
                    HostWriter::WakeContinuation,
                    waiter_id,
                    cause_ptr as u64,
                    &cause.to_be_bytes(),
                );
                if status_ptr == 0 {
                    cellgov_ps3_abi::lv2::errno::CELL_EFAULT.into()
                } else {
                    self.commit_bytes_at(
                        HostWriter::WakeContinuation,
                        waiter_id,
                        status_ptr as u64,
                        &status.to_be_bytes(),
                    );
                    code
                }
            };
            self.deliver_syscall_return(waiter_id, code);
            self.registry
                .set_status_override(waiter_id, UnitStatus::Runnable);
        }
    }
}

#[cfg(test)]
#[path = "tests/event_queue_wake_tests.rs"]
mod event_queue_wake_tests;
