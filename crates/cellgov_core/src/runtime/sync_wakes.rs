//! Pending-response wake protocol for blocked units.
//!
//! Two resolution paths, sharing the same mental model: each walks
//! `syscall_responses`, consumes a `PendingResponse`, writes any
//! continuation payload through `SharedWriteIntent`-style commits,
//! and transitions the waiter from `Blocked` back to `Runnable`.
//!
//! - [`Runtime::resolve_sync_wakes`] fires when an LV2 release
//!   (`sys_mutex_unlock`, `sys_lwmutex_unlock`, `sys_semaphore_post`,
//!   `sys_event_queue_send`, event-flag set, cond-signal) produces a
//!   `WakeAndReturn` or `BlockAndWake` dispatch with a list of waiter
//!   ids.
//! - [`Runtime::resolve_join_wakes`] fires when an SPU finishes and
//!   the LV2 host reports the enclosing thread group has gone fully
//!   finished; any PPU parked on `sys_spu_thread_group_join` for that
//!   group is released.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::PendingResponse;

use super::Runtime;

impl Runtime {
    /// Apply the pending-response wake protocol for a set of units
    /// parked on a synchronization primitive (lwmutex, mutex,
    /// semaphore, event queue, or cond).
    ///
    /// For each unit the runtime:
    ///   * Takes its `PendingResponse` from `syscall_responses`.
    ///   * Resolves the response variant:
    ///       - `ReturnCode { code }`: set r3 = code.
    ///       - `EventQueueReceive { out_ptr, payload: Some(..) }`:
    ///         write the 32-byte payload via a `SharedWriteIntent`
    ///         commit and set r3 = 0.
    ///       - `CondWakeReacquire { .. }`: handled by a later
    ///         phase; the current implementation wakes with r3 = 0
    ///         and does not re-park on a mutex.
    ///       - Other variants: defensive fallback -- set r3 = 0.
    ///   * Transitions the unit from `Blocked` to `Runnable`.
    ///
    /// # Panics
    /// Reaching an `EventQueueReceive { payload: None }` at wake
    /// time is a host-level invariant break -- the send-side
    /// dispatch forgot to install `response_updates`. Panics rather
    /// than delivering four zero u64s the guest cannot distinguish
    /// from a real event.
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
                    // The full re-acquire path lands with the
                    // cond primitive. For now treat the wake as a
                    // plain CELL_OK wake.
                    self.registry.set_syscall_return(waiter, 0);
                }
                Some(_) | None => {
                    // Defensive: an ill-formed pending or an absent
                    // entry still transitions the waiter back to
                    // runnable so it is not stranded.
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

    /// When an SPU finishes, notify the LV2 host. If the group is
    /// fully finished, find and wake the PPU blocked on that group's
    /// join with its pending response.
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
            // `peek` above just confirmed the waiter holds a
            // matching ThreadGroupJoin -- presence is a contract
            // here, not a hopeful check. Use `take_expected` so a
            // future change that accidentally drains the entry
            // between the peek and the take (a refactor that adds
            // an intervening call, say) fails loudly with a panic
            // rather than silently dropping into the `else` branch.
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
                        // SPU thread-group wake path should not
                        // reach a PPU-thread-join / event-queue /
                        // cond-wake waiter. The phase that wires
                        // each of those primitives installs its
                        // own wake path; recover defensively here
                        // by setting the waiter runnable without
                        // writing to the out pointer.
                        self.registry
                            .set_status_override(waiter_id, UnitStatus::Runnable);
                    }
                }
            }
        }
    }
}
