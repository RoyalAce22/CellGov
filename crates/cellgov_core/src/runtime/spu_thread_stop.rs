//! LV2's reading of a stopped SPU thread: an exit, a yield, or a
//! thread-group error.

use cellgov_event::UnitId;
use cellgov_exec::{StopRegisters, UnitStatus};
use cellgov_lv2::{SpuThreadError, SpuThreadStop};
use cellgov_ps3_abi::hw::spu::{SPU_STATUS_STOP_CODE_SHIFT, SPU_STOP_CODE_MASK};

use super::Runtime;

/// An SPU thread whose stop LV2 turned into a thread-group error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuThreadFailure {
    /// The SPU unit.
    pub unit: UnitId,
    /// The stopped state it reported.
    pub stop: StopRegisters,
    /// Why LV2 does not serve the stop.
    pub error: SpuThreadError,
}

impl core::fmt::Display for SpuThreadFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "SPU thread unit {}: {} (SPU_Status 0x{:08x}, SPU_NPC 0x{:08x})",
            self.unit.raw(),
            self.error,
            self.stop.status,
            self.stop.npc
        )
    }
}

impl Runtime {
    /// Settle a `Finished` yield of `unit`, whose stopped state is
    /// `stopped`.
    ///
    /// A unit no thread group holds, or one that finished without
    /// stopping itself, is a thread finish. An SPU thread's stop is a
    /// request to LV2:
    ///
    /// - an exit records its status from the outbound mailbox and
    ///   finishes the thread, or with a group exit every thread of the
    ///   group;
    /// - a yield resumes the thread;
    /// - anything else leaves the thread unfinished, marks it
    ///   `Faulted`, and records a [`SpuThreadFailure`].
    pub(super) fn settle_finished_unit(&mut self, unit: UnitId, stopped: Option<StopRegisters>) {
        let (Some(stop), Some(_)) = (stopped, self.lv2_host.live_spu_group(unit)) else {
            self.resolve_join_wakes(unit);
            return;
        };
        let code = ((stop.status >> SPU_STATUS_STOP_CODE_SHIFT) & SPU_STOP_CODE_MASK) as u16;
        match SpuThreadStop::from_status(stop.status) {
            SpuThreadStop::Yield => {
                let resumed = self.registry.get_mut(unit).map(|spu| spu.restart().is_ok());
                if resumed != Some(true) {
                    self.lv2_host.log_invariant_break(
                        "runtime.spu_thread_yield_restart_failed",
                        format_args!("the yielding SPU thread {unit:?} did not restart"),
                    );
                }
            }
            SpuThreadStop::ThreadExit => {
                let Some(status) = self.take_exit_status(unit) else {
                    self.fail_spu_thread(unit, stop, SpuThreadError::ExitWithoutStatus(code));
                    return;
                };
                match self.lv2_host.spu_thread_exit(unit, status) {
                    Ok(Some(group)) => self.wake_group_joiners(group),
                    Ok(None) => {}
                    Err(err) => self.lv2_host.log_invariant_break(
                        "runtime.spu_thread_exit_rejected",
                        format_args!("sys_spu_thread_exit from {unit:?} rejected: {err:?}"),
                    ),
                }
            }
            SpuThreadStop::GroupExit => {
                let Some(status) = self.take_exit_status(unit) else {
                    self.fail_spu_thread(unit, stop, SpuThreadError::ExitWithoutStatus(code));
                    return;
                };
                match self.lv2_host.spu_group_exit(unit, status) {
                    Ok((group, others)) => {
                        for other in others {
                            self.registry
                                .set_status_override(other, UnitStatus::Finished);
                        }
                        self.wake_group_joiners(group);
                    }
                    Err(err) => self.lv2_host.log_invariant_break(
                        "runtime.spu_group_exit_rejected",
                        format_args!("sys_spu_thread_group_exit from {unit:?} rejected: {err:?}"),
                    ),
                }
            }
            SpuThreadStop::Error(error) => self.fail_spu_thread(unit, stop, error),
        }
    }

    /// The first SPU thread-group error since the last call, which the
    /// call clears.
    pub fn take_spu_thread_failure(&mut self) -> Option<SpuThreadFailure> {
        self.spu_thread_failure.take()
    }

    fn take_exit_status(&mut self, unit: UnitId) -> Option<u32> {
        self.registry.get_mut(unit)?.read_out_mbox().ok().flatten()
    }

    fn fail_spu_thread(&mut self, unit: UnitId, stop: StopRegisters, error: SpuThreadError) {
        self.registry.set_status_override(unit, UnitStatus::Faulted);
        self.spu_thread_failure
            .get_or_insert(SpuThreadFailure { unit, stop, error });
    }
}

#[cfg(test)]
#[path = "tests/spu_thread_stop_tests.rs"]
mod tests;
