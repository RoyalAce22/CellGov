//! Every `StepError` that comes before the unit runs leaves the
//! per-step records empty.

use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks, InstructionCost};
use cellgov_trace::HostWriter;
use strum::VariantArray;

use crate::runtime::{AddressSpaceId, HostWrite, Runtime, StepError};

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).expect("in-range test address")
}

/// Runnable until it has run `max` steps; charges its whole budget.
#[derive(Clone)]
struct Counter {
    id: UnitId,
    runs: u64,
    max: u64,
}

impl ExecutionUnit for Counter {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.runs >= self.max {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.runs += 1;
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.runs
    }
}

/// Fills every record with a value no refusal writes.
fn seed_records(rt: &mut Runtime) {
    let unit = UnitId::new(7);
    let request = DmaRequest::new(DmaDirection::Put, range(0x10, 8), range(0x20, 8), unit)
        .expect("a put between two disjoint ranges");
    rt.observations.dma_completions =
        vec![(DmaCompletion::new(request, GuestTicks::new(1)), false)];
    rt.observations.host_writes = vec![HostWrite {
        writer: HostWriter::Placement,
        space: AddressSpaceId::BOOT,
        range: range(0, 4),
    }];
    rt.observations.lv2_effects = vec![Effect::TraceMarker {
        marker: 1,
        source: unit,
    }];
    rt.observations.problem_state_accesses = vec![(unit, range(0, 4))];
}

fn assert_records_empty(rt: &Runtime, refusal: StepError) {
    assert!(
        rt.observations().dma_completions.is_empty(),
        "{refusal:?}: dma completions"
    );
    assert!(
        rt.observations().host_writes.is_empty(),
        "{refusal:?}: host writes"
    );
    assert!(
        rt.observations().lv2_effects.is_empty(),
        "{refusal:?}: lv2 effects"
    );
    assert!(
        rt.observations().problem_state_accesses.is_empty(),
        "{refusal:?}: problem-state accesses"
    );
}

/// A runtime whose next step answers `refusal`, with no event queued.
fn refusing(refusal: StepError) -> Runtime {
    match refusal {
        StepError::NoRunnableUnit => Runtime::new(GuestMemory::new(64), Budget::new(4), 100),
        StepError::AllBlocked => {
            let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 100);
            let id = rt.register_unit_with(|id| Counter {
                id,
                runs: 0,
                max: 9,
            });
            rt.set_unit_status_override(id, UnitStatus::Blocked);
            rt
        }
        StepError::MaxStepsExceeded => Runtime::new(GuestMemory::new(64), Budget::new(4), 0),
        StepError::TimeOverflow => {
            let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(u64::MAX - 5), 100);
            rt.register_unit_with(|id| Counter {
                id,
                runs: 0,
                max: 9,
            });
            let mut step = rt.step().expect("the first step fits");
            rt.commit_step(&mut step).expect("the step commits");
            rt
        }
        StepError::SchedulerNotReinstalled => {
            let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(4), 100);
            let snap = rt.snapshot();
            rt.restore_into(&snap);
            rt
        }
    }
}

#[test]
fn every_refusal_before_the_unit_runs_leaves_the_step_records_empty() {
    for &refusal in StepError::VARIANTS {
        let mut rt = refusing(refusal);
        seed_records(&mut rt);
        assert_eq!(rt.step().unwrap_err(), refusal);
        assert_records_empty(&rt, refusal);
    }
}
