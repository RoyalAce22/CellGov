//! Commit-boundary trace emission. One helper --
//! [`Runtime::emit_commit_trace`] -- writes one `CommitApplied`, one
//! `UnitBlocked` / `UnitWoken` per status transition (including
//! DMA-completion wakes), and the four `StateHashCheckpoint` records
//! (committed memory, runnable queue, unit status, sync state) taken
//! after the commit and DMA firing. Skipped under
//! `RuntimeMode::FaultDriven`.

use cellgov_dma::DmaCompletion;
use cellgov_event::UnitId;
use cellgov_trace::{
    HashCheckpointKind, StateHash, TraceRecord, TracedBlockReason, TracedWakeReason,
};

use crate::commit::{BlockReason, CommitError, CommitOutcome};

use super::{Runtime, RuntimeMode};

impl Runtime {
    /// No-op under `RuntimeMode::FaultDriven`; called from
    /// [`Runtime::commit_step`] after the commit pipeline runs and DMA
    /// completions fire.
    pub(super) fn emit_commit_trace(
        &mut self,
        source: UnitId,
        outcome: &Result<CommitOutcome, CommitError>,
        due: &[(DmaCompletion, Option<Vec<u8>>)],
        timer_due: &[crate::timer_queue::TimerWake],
    ) {
        if self.mode == RuntimeMode::FaultDriven {
            return;
        }

        // One CommitApplied per commit boundary; validation failure
        // closes the batch as `fault_discarded = true` with zero counts.
        let record = match outcome {
            Ok(o) => TraceRecord::CommitApplied {
                unit: source,
                writes_committed: o.writes_committed as u32,
                effects_deferred: o.effects_deferred as u32,
                fault_discarded: o.fault_discarded,
                epoch_after: self.epoch,
            },
            Err(_) => TraceRecord::CommitApplied {
                unit: source,
                writes_committed: 0,
                effects_deferred: 0,
                fault_discarded: true,
                epoch_after: self.epoch,
            },
        };
        self.trace.record(&record);

        if let Ok(o) = outcome {
            for &(unit, ref reason) in &o.blocked_units {
                let traced_reason = match reason {
                    BlockReason::MailboxEmpty => TracedBlockReason::MailboxEmpty,
                    BlockReason::WaitOnEvent => TracedBlockReason::WaitOnEvent,
                    BlockReason::DmaWait => TracedBlockReason::DmaWait,
                };
                self.trace.record(&TraceRecord::UnitBlocked {
                    unit,
                    reason: traced_reason,
                });
            }
            for &unit in &o.woken_units {
                self.trace.record(&TraceRecord::UnitWoken {
                    unit,
                    reason: TracedWakeReason::WakeEffect,
                });
            }
        }
        self.record_dma_wakes(due);
        self.record_timer_wakes(timer_due);

        // State hash checkpoints taken after the commit and timer/DMA firing.
        let mem_hash = StateHash::new(self.memory.content_hash());
        self.trace.record(&TraceRecord::StateHashCheckpoint {
            kind: HashCheckpointKind::CommittedMemory,
            hash: mem_hash,
        });
        let rq_hash = StateHash::new(self.registry.runnable_queue_hash());
        self.trace.record(&TraceRecord::StateHashCheckpoint {
            kind: HashCheckpointKind::RunnableQueue,
            hash: rq_hash,
        });
        let status_hash = StateHash::new(self.registry.status_hash());
        self.trace.record(&TraceRecord::StateHashCheckpoint {
            kind: HashCheckpointKind::UnitStatus,
            hash: status_hash,
        });
        let sync_hash = StateHash::new(self.sync_state_hash());
        self.trace.record(&TraceRecord::StateHashCheckpoint {
            kind: HashCheckpointKind::SyncState,
            hash: sync_hash,
        });
    }

    /// Trace records for completions fired by [`Runtime::step`]'s
    /// no-runnable-unit time-warp. Same UnitWoken records as the
    /// commit-boundary path; no CommitApplied or state-hash checkpoints
    /// because no step ran.
    pub(super) fn emit_time_warp_trace(
        &mut self,
        due: &[(DmaCompletion, Option<Vec<u8>>)],
        timer_due: &[crate::timer_queue::TimerWake],
    ) {
        if self.mode == RuntimeMode::FaultDriven {
            return;
        }
        self.record_dma_wakes(due);
        self.record_timer_wakes(timer_due);
    }

    /// A fired completion is not always a wake: the fire-side guard in
    /// [`Runtime::fire_dma_completions`] skips the Runnable override
    /// for a Finished or Faulted issuer, so the unit had no status
    /// transition and gets no `UnitWoken` record. Mirrors
    /// [`Self::record_timer_wakes`].
    fn record_dma_wakes(&mut self, due: &[(DmaCompletion, Option<Vec<u8>>)]) {
        for (c, _) in due {
            if self.registry.effective_status(c.issuer())
                != Some(cellgov_exec::UnitStatus::Runnable)
            {
                continue;
            }
            self.trace.record(&TraceRecord::UnitWoken {
                unit: c.issuer(),
                reason: TracedWakeReason::DmaCompletion,
            });
        }
    }

    /// A popped deadline is not always a wake: a timed cond wait whose
    /// companion mutex is contended re-parks untimed with ETIMEDOUT
    /// staged ([`cellgov_lv2::Lv2Host::expire_wait`]), leaving the unit
    /// Blocked. Record `UnitWoken` only for units the firing made
    /// runnable.
    fn record_timer_wakes(&mut self, timer_due: &[crate::timer_queue::TimerWake]) {
        for wake in timer_due {
            if self.registry.effective_status(wake.unit) != Some(cellgov_exec::UnitStatus::Runnable)
            {
                continue;
            }
            self.trace.record(&TraceRecord::UnitWoken {
                unit: wake.unit,
                reason: TracedWakeReason::Timer,
            });
        }
    }
}
