//! The DMA block/unblock fixture.

use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::ByteRange;
use cellgov_time::{Budget, GuestTicks, InstructionCost};
use std::cell::Cell;

/// Two-stage DMA block/unblock probe: seed source, submit Put, block;
/// then on wake emit a `TraceMarker` and finish.
#[derive(Clone)]
pub struct DmaSubmitter {
    id: UnitId,
    source: ByteRange,
    destination: ByteRange,
    seed_bytes: Vec<u8>,
    phase: Cell<u8>,
}

impl DmaSubmitter {
    /// Construct a submitter writing `seed_bytes` to `source` then
    /// enqueuing a DMA Put from `source` to `destination`.
    ///
    /// # Panics
    ///
    /// Panics if `seed_bytes.len() != source.length()`.
    pub fn new(id: UnitId, source: ByteRange, destination: ByteRange, seed_bytes: Vec<u8>) -> Self {
        assert_eq!(
            seed_bytes.len() as u64,
            source.length(),
            "seed_bytes length must match source range"
        );
        Self {
            id,
            source,
            destination,
            seed_bytes,
            phase: Cell::new(0),
        }
    }
}

impl ExecutionUnit for DmaSubmitter {
    type Snapshot = u8;
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.phase.get() >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let p = self.phase.get();
        self.phase.set(p + 1);
        match p {
            0 => {
                let req =
                    DmaRequest::new(DmaDirection::Put, self.source, self.destination, self.id)
                        .expect("source and destination lengths match");
                effects.push(Effect::shared_write(
                    self.source,
                    WritePayload::new(self.seed_bytes.clone()),
                    self.id,
                    GuestTicks::ZERO,
                ));
                effects.push(Effect::DmaEnqueue {
                    request: req,
                    payload: None,
                });
                effects.push(Effect::WaitOnEvent {
                    target: cellgov_effects::WaitTarget::Barrier(cellgov_sync::BarrierId::new(0)),
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::DmaSubmitted,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
            _ => {
                effects.push(Effect::TraceMarker {
                    marker: 0xd0d0,
                    source: self.id,
                });
                ExecutionStepResult {
                    yield_reason: YieldReason::Finished,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::empty(),
                    fault: None,
                    syscall_args: None,
                }
            }
        }
    }
    fn snapshot(&self) -> u8 {
        self.phase.get()
    }
}
