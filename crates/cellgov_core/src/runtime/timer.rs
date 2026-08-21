//! Timer-wake firing. [`Runtime::fire_timer_wakes`] runs per commit and
//! in the all-blocked time-warp, resuming units whose deadline arrived.

use cellgov_time::GuestTicks;

use crate::timer_queue::{TimerWake, TimerWakeKind};

use super::Runtime;

impl Runtime {
    /// Pop and resolve timer wakes whose deadline has arrived; returns
    /// the fired list for trace recording. Due entries whose unit is
    /// already Finished (process-exit residue) are dropped unfired.
    pub(super) fn fire_timer_wakes(&mut self) -> Vec<TimerWake> {
        if self.timer_wakes.is_empty() {
            // Breaks buffered by earlier work in this commit still
            // trace at this boundary even with nothing to fire.
            self.drain_invariant_breaks_to_trace();
            return Vec::new();
        }
        let due = self.timer_wakes.pop_due(self.time);
        let mut fired = Vec::with_capacity(due.len());
        for wake in due {
            // A due entry for a Finished unit is process-exit residue:
            // the exit sweep finishes every unit and drops its parked
            // response without touching the timer queue, and this
            // firing pass runs in the same commit. Firing anyway would
            // resurrect the unit (the Runnable override replaces
            // Finished) and trip the missing-response invariant break.
            // handle_process_exit_child finishes every one of the
            // pid's units at the same exit, so a due sleep or timed
            // wait never resumes its thread after exit. (RPCS3 is not
            // an oracle here: its _sys_process_exit calls Emu.Kill()
            // and tears down the whole emulator rather than reclaiming
            // one process.) Dropped wakes are excluded from the
            // returned list so no UnitWoken record is traced for a
            // unit that stays Finished.
            if self.registry.effective_status(wake.unit) == Some(cellgov_exec::UnitStatus::Finished)
            {
                continue;
            }
            match wake.kind {
                TimerWakeKind::Sleep => {
                    self.resolve_sync_wakes(&[wake.unit]);
                }
                TimerWakeKind::SyncWait(reason) => {
                    let expired = self.lv2_host.expire_wait(reason, wake.unit, self.time);
                    // Expiry effects repair structs the expiring
                    // waiter's syscall named, so they land in its space.
                    let waiter_space = self.spaces.space_of(wake.unit);
                    self.apply_lv2_effects(&expired.effects, waiter_space);
                    if cfg!(debug_assertions) {
                        crate::runtime::lv2_dispatch::check_response_updates(
                            "fire_timer_wakes",
                            &self.syscall_responses,
                            &expired.woken_unit_ids,
                            &expired.response_updates,
                        );
                    }
                    for (waiter, response) in expired.response_updates {
                        // The expiring unit's own entry is already
                        // popped; this covers hypothetical peers.
                        self.timer_wakes.cancel(waiter);
                        let _ = self.syscall_responses.try_take(waiter);
                        let _ = self.syscall_responses.insert(waiter, response);
                    }
                    self.resolve_sync_wakes(&expired.woken_unit_ids);
                }
            }
            fired.push(wake);
        }
        // Breaks logged by expire_wait (or the sync-wake resolver)
        // during this firing pass trace at this boundary, not at the
        // next syscall dispatch.
        self.drain_invariant_breaks_to_trace();
        fired
    }

    /// Register a wake-at-deadline for a parked timed wait. Timeout 0
    /// is wait-forever per the PS3 ABI; `None` is a non-wait request.
    pub(super) fn register_wait_deadline(
        &mut self,
        source: cellgov_event::UnitId,
        reason: cellgov_lv2::Lv2BlockReason,
        timeout_usec: Option<u64>,
    ) {
        let Some(usec) = timeout_usec else { return };
        if usec == 0 {
            return;
        }
        let deadline = self.deadline_after_usec(usec);
        self.timer_wakes
            .insert(deadline, source, TimerWakeKind::SyncWait(reason));
    }

    /// Absolute wake deadline for a wait of `usec` microseconds from
    /// the current guest time, saturating at the tick ceiling.
    ///
    /// 1 tick = 1 ns per
    /// [`cellgov_time::SIMULATED_INSTRUCTIONS_PER_SECOND`], so the
    /// conversion is `usec * 1000`.
    pub(super) fn deadline_after_usec(&self, usec: u64) -> GuestTicks {
        let delta_ticks = usec.saturating_mul(1_000);
        GuestTicks::new(self.time.raw().saturating_add(delta_ticks))
    }
}
