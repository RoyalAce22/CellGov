//! Wake protocol for blocked units: consume `PendingResponse`, commit
//! continuation payload, transition Blocked -> Runnable.

use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_lv2::{CallbackReturnStage, PendingResponse};

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
                        let current = bytes
                            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
                            .unwrap_or(0);
                        let next = current.saturating_sub(1);
                        self.commit_bytes_at(waiter_addr, &next.to_be_bytes());
                    }
                    // Mirror the increment HLE lwmutex_lock does on the
                    // uncontended fast path.
                    if let Some(tid) = self.lv2_host.ppu_thread_id_for_unit(waiter) {
                        self.lv2_host.lwmutex_holds_inc(tid);
                    }
                    self.registry.set_syscall_return(waiter, 0);
                }
                Some(PendingResponse::CondWakeReacquire { .. }) => {
                    self.registry.set_syscall_return(waiter, 0);
                }
                Some(PendingResponse::CallbackReturn { stage, args }) => {
                    self.resume_callback_return(waiter, stage, args);
                }
                Some(_) | None => {
                    self.registry.set_syscall_return(waiter, 0);
                }
            }
            self.registry
                .set_status_override(waiter, UnitStatus::Runnable);
        }
    }

    /// Stage dispatcher for a callback worker's wake.
    ///
    /// Each parkable HLE handler that schedules a worker via
    /// `Lv2Host::call_guest_callback_sync` must land a matching
    /// `CallbackReturnStage` arm here; the wildcard `unimplemented!`
    /// catches new variants because `CallbackReturnStage` is
    /// `#[non_exhaustive]` cross-crate.
    ///
    /// `args` is the worker's `r3..=r10` captured at trampoline entry
    /// (PPC64 ELFv1: `args[0]` is r3).
    fn resume_callback_return(
        &mut self,
        waiter: UnitId,
        stage: CallbackReturnStage,
        args: [u64; 8],
    ) {
        match stage {
            CallbackReturnStage::Synthetic => {
                self.registry.set_syscall_return(waiter, args[0]);
            }
            CallbackReturnStage::AutoLoadAfterStat {
                cb_result_addr,
                stat_get_addr,
                stat_set_addr,
                func_file_opd,
            } => {
                crate::hle::cell_save_data::resume_after_stat(
                    self,
                    waiter,
                    cb_result_addr,
                    stat_get_addr,
                    stat_set_addr,
                    func_file_opd,
                    args,
                );
            }
            _ => {
                unimplemented!("resume_callback_return: stage {stage:?} has no resume arm");
            }
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
                #[allow(
                    clippy::print_stderr,
                    reason = "diagnostic for an LV2 host invariant break reachable only when thread-table state diverges from primitive state; one line per offending unit per host instance"
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
            // than silently falling through.
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
                    | PendingResponse::EventFlagWake { .. }
                    | PendingResponse::LwMutexWake { .. }
                    | PendingResponse::CallbackReturn { .. } => {
                        // Each variant has its own wake path; recover
                        // without writing to the out pointer.
                        self.registry
                            .set_status_override(waiter_id, UnitStatus::Runnable);
                    }
                }
            }
        }
    }
}
