//! The step-entry fire of a due event while units are still runnable.

use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, InstructionCost};
use cellgov_trace::HostWriter;

use crate::runtime::{AddressSpaceId, HostWrite, Runtime};

const DMA_SRC: u64 = 0x100;
const DMA_DST: u64 = 0x200;

fn range(addr: u64, len: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), len).expect("in-range test address")
}

/// Enqueues one transfer on its first step and keeps running without
/// waiting on it.
#[derive(Clone)]
struct DmaIssuer {
    id: UnitId,
    steps: u64,
}

impl ExecutionUnit for DmaIssuer {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps += 1;
        if self.steps == 1 {
            let request = DmaRequest::new(
                DmaDirection::Put,
                range(DMA_SRC, 8),
                range(DMA_DST, 8),
                self.id,
            )
            .expect("a put between two mapped, disjoint ranges");
            effects.push(Effect::DmaEnqueue {
                request,
                payload: None,
            });
        }
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) -> u64 {
        self.steps
    }
}

/// Runs without effects; its clock holds the transfer back.
#[derive(Clone)]
struct Idle {
    id: UnitId,
}

impl ExecutionUnit for Idle {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

#[test]
fn a_pre_select_fire_lands_a_host_write_and_no_published_completion() {
    let mut rt = Runtime::new(GuestMemory::new(4096), Budget::new(100), 100);
    let issuer = rt.register_unit_with(|id| DmaIssuer { id, steps: 0 });
    let idle = rt.register_unit_with(|id| Idle { id });

    // The issuer enqueues and moves ahead; the idle unit's clock stays
    // behind the transfer's tick at every commit.
    for _ in 0..3 {
        let mut step = rt.step().expect("both units are runnable");
        rt.commit_step(&mut step).expect("the step commits");
    }
    assert!(
        rt.registry().clock(idle) < rt.registry().clock(issuer),
        "the premise: the idle unit lags the issuer",
    );
    assert!(
        !rt.dma_queue().is_empty(),
        "the premise: no commit fired the transfer",
    );

    // Parking the lagging unit leaves the issuer's clock past the
    // transfer's tick, so the next step fires it at entry.
    rt.set_unit_status_override(idle, UnitStatus::Blocked);
    let mut step = rt.step().expect("the issuer is runnable");
    assert_eq!(step.unit, issuer);
    assert!(
        rt.dma_queue().is_empty(),
        "the step fired the transfer before it ran a unit",
    );
    rt.commit_step(&mut step).expect("the step commits");

    assert!(
        rt.observations().host_writes.contains(&HostWrite {
            writer: HostWriter::DmaCompletion,
            space: AddressSpaceId::BOOT,
            range: range(DMA_DST, 8),
        }),
        "the landing reaches the host-write list: {:?}",
        rt.observations().host_writes,
    );
    assert!(
        rt.observations().dma_completions.is_empty(),
        "a completion fired at step entry reaches no commit's list: {:?}",
        rt.observations().dma_completions,
    );
}
