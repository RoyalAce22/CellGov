//! A refused DMA enqueue leaves its issuer faulted, not parked.
//!
//! One batch can carry both a `DmaEnqueue` and the tag-status read
//! that waits on it. The unit a refused enqueue marks `Faulted` is then
//! the same unit the `DmaWait` park reaches. The mark has to win: a
//! refused enqueue queues no completion, so nothing publishes the tag
//! bit a parked issuer waits for.

use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, InstructionCost};

use crate::commit::{BlockReason, CommitError};
use crate::runtime::state::Runtime;
use crate::runtime::StepError;

/// Inside the runtime's only region.
const SRC: u64 = 0x20;
const MAPPED_DST: u64 = 0x40;

/// Past the end of that region, so the destination check refuses it.
const UNMAPPED_DST: u64 = 0x2000;

const LEN: u64 = 4;

fn range(addr: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), LEN).expect("a 4-byte range")
}

/// Enqueues one payload-less `DmaPut` and waits on its tag in the same
/// step.
///
/// That is what puts the refusal and the park on one unit.
#[derive(Clone)]
struct WaitingEmitter {
    id: UnitId,
    destination: u64,
    steps: u64,
}

impl ExecutionUnit for WaitingEmitter {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps >= 2 {
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
        self.steps += 1;
        let yield_reason = if self.steps == 1 {
            let dst = range(self.destination);
            let request = DmaRequest::new(DmaDirection::Put, range(SRC), dst, self.id)
                .expect("equal-length ends");
            effects.push(Effect::DmaEnqueue {
                request,
                payload: None,
            });
            YieldReason::DmaWait
        } else {
            YieldReason::Finished
        };
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

fn runtime_with_emitter(destination: u64) -> (Runtime, UnitId) {
    let mut rt = Runtime::new(GuestMemory::new(0x100), Budget::new(4), 100);
    let unit = rt.registry_mut().register_with(|id| WaitingEmitter {
        id,
        destination,
        steps: 0,
    });
    (rt, unit)
}

#[test]
fn a_refused_enqueue_leaves_its_waiting_issuer_faulted() {
    let (mut rt, unit) = runtime_with_emitter(UNMAPPED_DST);
    let step = rt.step().expect("the emitter runs");
    let err = rt
        .commit_step(&step.result, &step.effects)
        .expect_err("the destination is past the end of the only region");
    assert!(
        matches!(err, CommitError::DmaDestinationOutOfRange { .. }),
        "the destination is the end that fails: {err:?}",
    );
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Faulted),
        "the refusal's mark has to outlive the park, or the issuer waits \
         on a tag bit no completion will publish",
    );
    assert_eq!(
        rt.step().expect_err("nothing can run"),
        StepError::NoRunnableUnit,
        "a faulted issuer ends the run; AllBlocked would mean it parked",
    );
}

/// The enqueue itself resolves, so nothing marks the issuer `Faulted`.
/// The refusal discards the whole batch, so the queue holds no
/// completion to wake a park with.
#[test]
fn a_batch_refused_after_a_valid_enqueue_does_not_park_its_issuer() {
    let (mut rt, unit) = runtime_with_emitter(MAPPED_DST);
    let mut step = rt.step().expect("the emitter runs");
    // An unregistered wake target refuses the batch without touching the
    // enqueue, which validated one effect earlier.
    step.effects.push(Effect::WakeUnit {
        target: UnitId::new(99),
        source: unit,
    });
    let err = rt
        .commit_step(&step.result, &step.effects)
        .expect_err("the wake names a unit the registry does not hold");
    assert!(
        matches!(err, CommitError::UnknownWakeTarget { .. }),
        "the wake is what fails, not the enqueue: {err:?}",
    );
    assert!(
        rt.dma_queue().is_empty(),
        "a refused batch queues nothing, so there is no completion to \
         wake a parked issuer",
    );
    assert_ne!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Blocked),
        "parking here waits on a completion the refusal threw away",
    );
}

#[test]
fn an_accepted_enqueue_still_parks_its_waiting_issuer() {
    let (mut rt, unit) = runtime_with_emitter(MAPPED_DST);
    let step = rt.step().expect("the emitter runs");
    let outcome = rt
        .commit_step(&step.result, &step.effects)
        .expect("both ends resolve");
    let blocked = outcome.blocked_units;
    assert!(
        blocked.contains(&(unit, BlockReason::DmaWait)),
        "the park is what the completion wakes: {blocked:?}",
    );
}
