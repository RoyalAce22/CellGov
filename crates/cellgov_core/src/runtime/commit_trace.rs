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
    ) {
        if self.mode == RuntimeMode::FaultDriven {
            return;
        }

        // One CommitApplied per commit boundary. On validation failure
        // the batch is closed but empty: fault_discarded = true with
        // zero counts.
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
        for (c, _) in due {
            self.trace.record(&TraceRecord::UnitWoken {
                unit: c.issuer(),
                reason: TracedWakeReason::DmaCompletion,
            });
        }

        // State hash checkpoints taken after the commit and DMA firing.
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
}
