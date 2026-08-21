//! Wake protocol for blocked units: consume `PendingResponse`, commit
//! continuation payload, transition Blocked -> Runnable.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::PendingResponse;

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
            // later release can still pick the unit. On PS3 an exited
            // process's threads are gone before any wake can reach
            // them (RPCS3 sys_process.cpp _sys_process_exit stops
            // every thread), so the Runnable transition below would
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
                    self.registry.set_syscall_return(waiter, code);
                }
                Some(PendingResponse::EventQueueReceive { out_ptr, payload }) => {
                    let payload = payload.unwrap_or_else(|| {
                        panic!(
                            "EventQueueReceive wake for {waiter:?} with unfilled payload \
                             (release-side dispatch forgot response_updates)"
                        )
                    });
                    let mut buf = [0u8; 32];
                    buf[0..8].copy_from_slice(&payload.source.to_be_bytes());
                    buf[8..16].copy_from_slice(&payload.data1.to_be_bytes());
                    buf[16..24].copy_from_slice(&payload.data2.to_be_bytes());
                    buf[24..32].copy_from_slice(&payload.data3.to_be_bytes());
                    self.commit_bytes_at(waiter_space, out_ptr as u64, &buf);
                    self.registry.set_syscall_return(waiter, 0);
                }
                Some(PendingResponse::EventFlagWake {
                    result_ptr,
                    observed,
                }) => {
                    // The kernel stores the observed pattern only
                    // through a non-null result pointer (RPCS3
                    // sys_event_flag.cpp sys_event_store_result); a
                    // NULL pointer waiter wakes with r3 alone.
                    if result_ptr != 0 {
                        self.commit_bytes_at(
                            waiter_space,
                            result_ptr as u64,
                            &observed.to_be_bytes(),
                        );
                    }
                    self.registry.set_syscall_return(waiter, 0);
                }
                Some(PendingResponse::EventFlagCancelWake {
                    result_ptr,
                    observed,
                }) => {
                    // RPCS3 sys_event_flag.cpp sys_event_flag_cancel:
                    // each drained waiter stores the captured pattern
                    // through its own non-null result pointer and
                    // returns CELL_ECANCELED.
                    if result_ptr != 0 {
                        self.commit_bytes_at(
                            waiter_space,
                            result_ptr as u64,
                            &observed.to_be_bytes(),
                        );
                    }
                    self.registry.set_syscall_return(
                        waiter,
                        cellgov_ps3_abi::cell_errors::CELL_ECANCELED.into(),
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
                        self.commit_bytes_at(waiter_space, base, &caller.to_be_bytes());
                        self.commit_bytes_at(waiter_space, base + 12, &1u32.to_be_bytes());
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
                        self.commit_bytes_at(waiter_space, waiter_addr, &next.to_be_bytes());
                    }
                    if let Some(tid) = self.lv2_host.ppu_thread_id_for_unit(waiter) {
                        self.lv2_host.lwmutex_holds_inc(tid);
                    }
                    self.registry.set_syscall_return(waiter, 0);
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
        let finished_group = match self.lv2_host.notify_spu_finished(source) {
            Ok(Some(gid)) => gid,
            Ok(None) => return,
            Err(cellgov_lv2::thread_group::NotifySpuFinishedError::UnknownUnit) => return,
            Err(err) => {
                // This path fires under normal multi-finalize flows
                // (e.g. group teardown after the SPU has already been
                // marked Finished), so it cannot be an assertion.
                #[allow(
                    clippy::print_stderr,
                    reason = "diagnostic for an LV2 host invariant break; one line per offending unit per host instance"
                )]
                {
                    eprintln!(
                        "lv2 host invariant break at resolve_join_wakes.notify_spu_finished: \
                         unit {source:?}: {err:?}",
                    );
                }
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
            // change the outcome (RPCS3 sys_spu.cpp
            // sys_spu_thread_group_join checks the pointers after the
            // wait): a NULL cause writes nothing -- not even a
            // non-NULL status -- and returns CELL_EFAULT; a NULL
            // status alone still writes cause and returns CELL_EFAULT.
            // Address 0 may be mapped, so NULL is never written
            // through.
            let waiter_space = self.spaces.space_of(waiter_id);
            let code = if cause_ptr == 0 {
                cellgov_ps3_abi::cell_errors::CELL_EFAULT.into()
            } else {
                self.commit_bytes_at(waiter_space, cause_ptr as u64, &cause.to_be_bytes());
                if status_ptr == 0 {
                    cellgov_ps3_abi::cell_errors::CELL_EFAULT.into()
                } else {
                    self.commit_bytes_at(waiter_space, status_ptr as u64, &status.to_be_bytes());
                    code
                }
            };
            self.registry.set_syscall_return(waiter_id, code);
            self.registry
                .set_status_override(waiter_id, UnitStatus::Runnable);
        }
    }
}
