//! Wake protocol for blocked units: consume `PendingResponse`, commit
//! continuation payload, transition Blocked -> Runnable.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::PendingResponse;

use super::Runtime;

impl Runtime {
    /// # Panics
    ///
    /// `EventQueueReceive { payload: None }`: four zero u64s would be
    /// indistinguishable from a real event.
    pub(super) fn resolve_sync_wakes(&mut self, woken_unit_ids: &[UnitId]) {
        for waiter in woken_unit_ids {
            let waiter = *waiter;
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
                    self.commit_bytes_at(out_ptr as u64, &buf);
                    self.registry.set_syscall_return(waiter, 0);
                }
                Some(PendingResponse::EventFlagWake {
                    result_ptr,
                    observed,
                }) => {
                    self.commit_bytes_at(result_ptr as u64, &observed.to_be_bytes());
                    self.registry.set_syscall_return(waiter, 0);
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
                        self.commit_bytes_at(base, &caller.to_be_bytes());
                        self.commit_bytes_at(base + 12, &1u32.to_be_bytes());
                        let waiter_addr = base + 4;
                        let bytes = self.memory.read(
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
                        self.commit_bytes_at(waiter_addr, &next.to_be_bytes());
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
                // Not a debug_assert: this path fires under normal
                // multi-finalize flows (e.g. group teardown after the
                // SPU has already been marked Finished), not only on
                // a thread-table-vs-primitive divergence. Keeping the
                // eprintln until a structured trace event lands.
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
            // than silently falling through. Runtime is single-threaded
            // so peek and take_expected see the same variant; the
            // let-else converts that guarantee into a typed binding.
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
            self.commit_bytes_at(cause_ptr as u64, &cause.to_be_bytes());
            self.commit_bytes_at(status_ptr as u64, &status.to_be_bytes());
            self.registry.set_syscall_return(waiter_id, code);
            self.registry
                .set_status_override(waiter_id, UnitStatus::Runnable);
        }
    }
}
