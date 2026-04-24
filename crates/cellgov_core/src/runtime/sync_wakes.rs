//! Pending-response wake protocol for blocked units: consume a
//! `PendingResponse`, commit any continuation payload, and transition
//! the waiter from `Blocked` to `Runnable`.
//!
//! [`Runtime::resolve_sync_wakes`] handles releases on sync primitives
//! (mutex, lwmutex, semaphore, event queue, event flag, cond).
//! [`Runtime::resolve_join_wakes`] handles PPUs parked on
//! `sys_spu_thread_group_join` when the group finishes.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::PendingResponse;

use super::Runtime;

impl Runtime {
    /// Resolve each waiter's `PendingResponse`, commit continuation
    /// payloads, and flip Blocked -> Runnable.
    ///
    /// # Panics
    ///
    /// Panics on `EventQueueReceive { payload: None }` at wake time:
    /// the send-side dispatch forgot to install `response_updates`.
    /// Delivering four zero u64s would be indistinguishable from a
    /// real event.
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
                Some(PendingResponse::CondWakeReacquire { .. }) => {
                    // Full re-acquire belongs with the cond primitive;
                    // for now wake with CELL_OK and do not re-park.
                    self.registry.set_syscall_return(waiter, 0);
                }
                Some(_) | None => {
                    // Defensive: an ill-formed or absent entry still
                    // transitions the waiter back to runnable.
                    self.registry.set_syscall_return(waiter, 0);
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

    /// Notify the LV2 host that `source` finished; if the enclosing
    /// group is fully finished, wake any PPU blocked on its join.
    pub(super) fn resolve_join_wakes(&mut self, source: UnitId) {
        let finished_group = match self.lv2_host.notify_spu_finished(source) {
            Ok(Some(gid)) => gid,
            Ok(None) => return,
            Err(cellgov_lv2::thread_group::NotifySpuFinishedError::UnknownUnit) => return,
            Err(err) => {
                eprintln!(
                    "lv2 host invariant break at resolve_join_wakes.notify_spu_finished: \
                     unit {source:?}: {err:?}",
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
            // `peek` confirmed a matching ThreadGroupJoin; use
            // `take_expected` so an accidental intervening drain
            // panics rather than silently dropping into `else`.
            let pending = self.syscall_responses.take_expected(waiter_id);
            {
                match &pending {
                    PendingResponse::ThreadGroupJoin {
                        code,
                        cause_ptr,
                        status_ptr,
                        cause,
                        status,
                        ..
                    } => {
                        self.registry.set_syscall_return(waiter_id, *code);
                        self.registry
                            .set_status_override(waiter_id, UnitStatus::Runnable);
                        self.commit_bytes_at(*cause_ptr as u64, &cause.to_be_bytes());
                        self.commit_bytes_at(*status_ptr as u64, &status.to_be_bytes());
                    }
                    PendingResponse::ReturnCode { code } => {
                        self.registry.set_syscall_return(waiter_id, *code);
                        self.registry
                            .set_status_override(waiter_id, UnitStatus::Runnable);
                    }
                    PendingResponse::PpuThreadJoin { .. }
                    | PendingResponse::EventQueueReceive { .. }
                    | PendingResponse::CondWakeReacquire { .. }
                    | PendingResponse::EventFlagWake { .. } => {
                        // The SPU thread-group wake path should not
                        // reach these variants; each has its own wake
                        // path. Defensive recovery without writing to
                        // the out pointer.
                        self.registry
                            .set_status_override(waiter_id, UnitStatus::Runnable);
                    }
                }
            }
        }
    }
}
