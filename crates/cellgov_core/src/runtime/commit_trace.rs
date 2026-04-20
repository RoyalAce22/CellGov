//! Commit-boundary trace emission extracted from `commit_step`.
//!
//! One helper -- [`Runtime::emit_commit_trace`] -- owns the trace
//! and hash-checkpoint contract documented at the top of
//! `runtime.rs`:
//!
//! - One `CommitApplied` record per commit boundary (including the
//!   validation-rejection edge, where counts are zero and
//!   `fault_discarded = true`).
//! - One `UnitBlocked` / `UnitWoken` per status transition produced
//!   by this commit, including DMA-completion wakes.
//! - Four `StateHashCheckpoint` records (committed memory, runnable
//!   queue, unit status, sync state) taken *after* the commit and
//!   DMA-completion firing so replay tooling sees the full
//!   post-commit state.
//!
//! All of the above is skipped under `RuntimeMode::FaultDriven`.
//! Keeping the block isolated from `commit_step` makes the trace
//! contract auditable without reading through the commit
//! orchestration logic.

use cellgov_dma::DmaCompletion;
use cellgov_event::UnitId;
use cellgov_trace::{
    HashCheckpointKind, StateHash, TraceRecord, TracedBlockReason, TracedWakeReason,
};

use crate::commit::{BlockReason, CommitError, CommitOutcome};

use super::{Runtime, RuntimeMode};

impl Runtime {
    /// Emit the commit-boundary trace records and the four post-commit
    /// state hash checkpoints. Called from [`Runtime::commit_step`]
    /// after the commit pipeline has run and DMA completions have
    /// fired; no-op under `RuntimeMode::FaultDriven`.
    pub(super) fn emit_commit_trace(
        &mut self,
        source: UnitId,
        outcome: &Result<CommitOutcome, CommitError>,
        due: &[(DmaCompletion, Option<Vec<u8>>)],
    ) {
        if self.mode == RuntimeMode::FaultDriven {
            return;
        }

        // Trace pipeline step 7 (and the validation rejection edge): one
        // CommitApplied record per commit boundary, carrying the
        // post-commit epoch. On validation failure, rejection surfaces
        // as a fault on the originating unit, so
        // we record fault_discarded = true with zero counts -- the
        // batch is closed, just empty.
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

        // State hash checkpoints. Four kinds: committed memory,
        // runnable queue, sync state, and unit status. All four
        // emitted here, taken AFTER the commit (including DMA
        // completion firing) so replay tooling sees post-commit
        // state.
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
