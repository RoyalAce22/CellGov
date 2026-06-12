//! ExecutionUnit trait contract via a minimal counting unit, plus UnitStatus discriminant locking.

use super::*;
use crate::yield_reason::YieldReason;
use crate::LocalDiagnostics;
use cellgov_mem::GuestMemory;
use cellgov_time::InstructionCost;
use strum::VariantArray;

#[test]
fn unit_status_variants_are_distinct() {
    let unique: std::collections::BTreeSet<u8> =
        UnitStatus::VARIANTS.iter().map(|s| *s as u8).collect();
    assert_eq!(unique.len(), UnitStatus::VARIANTS.len());
}

#[derive(Clone)]

struct CountingUnit {
    id: UnitId,
    steps: u64,
    max_steps: u64,
}

impl ExecutionUnit for CountingUnit {
    type Snapshot = u64;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps >= self.max_steps {
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
        let yield_reason = if self.steps >= self.max_steps {
            YieldReason::Finished
        } else {
            YieldReason::BudgetExhausted
        };
        effects.push(Effect::TraceMarker {
            marker: self.steps as u32,
            source: self.id,
        });
        ExecutionStepResult {
            yield_reason,
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

#[test]
fn unit_status_discriminants_locked() {
    assert_eq!(UnitStatus::Runnable as u8, 0);
    assert_eq!(UnitStatus::Blocked as u8, 1);
    assert_eq!(UnitStatus::Faulted as u8, 2);
    assert_eq!(UnitStatus::Finished as u8, 3);
}

#[test]
fn counting_unit_runs_to_completion() {
    let mem = GuestMemory::new(16);
    let ctx = ExecutionContext::new(&mem);
    let mut unit = CountingUnit {
        id: UnitId::new(7),
        steps: 0,
        max_steps: 3,
    };
    assert_eq!(unit.unit_id(), UnitId::new(7));
    assert_eq!(unit.status(), UnitStatus::Runnable);

    let mut effects = Vec::new();
    let r1 = unit.run_until_yield(Budget::new(10), &ctx, &mut effects);
    assert_eq!(r1.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(r1.consumed_cost, InstructionCost::new(10));
    assert_eq!(effects.len(), 1);
    assert_eq!(unit.snapshot(), 1);
    assert_eq!(unit.status(), UnitStatus::Runnable);

    effects.clear();
    let _ = unit.run_until_yield(Budget::new(10), &ctx, &mut effects);
    effects.clear();
    let r3 = unit.run_until_yield(Budget::new(10), &ctx, &mut effects);
    assert_eq!(r3.yield_reason, YieldReason::Finished);
    assert_eq!(unit.snapshot(), 3);
    assert_eq!(unit.status(), UnitStatus::Finished);
}

#[test]
fn snapshot_is_value_data() {
    let mut unit = CountingUnit {
        id: UnitId::new(0),
        steps: 5,
        max_steps: 10,
    };
    let snap_before = unit.snapshot();
    let mem = GuestMemory::new(8);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let _ = unit.run_until_yield(Budget::new(1), &ctx, &mut effects);
    let snap_after = unit.snapshot();
    assert_eq!(snap_before, 5);
    assert_eq!(snap_after, 6);
}
