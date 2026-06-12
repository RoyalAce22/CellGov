//! Scenario runner outcomes -- stall vs max-steps, trace contents, and byte-identical reruns.

use super::*;
use crate::world::CountingUnit;
use cellgov_core::Runtime;
use cellgov_time::Budget;
use cellgov_trace::{TraceReader, TraceRecord};

#[test]
fn empty_fixture_stalls_immediately_with_no_steps() {
    let result = run(ScenarioFixture::empty());
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 0);
    assert!(result.trace_bytes.is_empty());
}

#[test]
fn single_unit_runs_to_completion_then_stalls() {
    let result = run(ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(100)
        .register(|rt: &mut Runtime| {
            let r = rt.registry_mut();
            r.register_with(|id| CountingUnit::new(id, 5));
        })
        .build());
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 5);
    let scheduled_count = TraceReader::new(&result.trace_bytes)
        .map(|r| r.expect("decode"))
        .filter(|r| matches!(r, TraceRecord::UnitScheduled { .. }))
        .count();
    assert_eq!(scheduled_count, 5);
}

#[test]
fn max_steps_cap_surfaces_as_max_steps_exceeded() {
    let result = run(ScenarioFixture::builder()
        .memory_size(16)
        .budget(Budget::new(1))
        .max_steps(3)
        .register(|rt: &mut Runtime| {
            let r = rt.registry_mut();
            r.register_with(|id| CountingUnit::new(id, u64::MAX));
        })
        .build());
    assert_eq!(result.outcome, ScenarioOutcome::MaxStepsExceeded);
    assert_eq!(result.steps_taken, 3);
}

#[test]
fn two_runs_of_the_same_fixture_are_byte_identical() {
    fn build_and_run() -> ScenarioResult {
        run(ScenarioFixture::builder()
            .memory_size(16)
            .budget(Budget::new(2))
            .max_steps(100)
            .register(|rt: &mut Runtime| {
                let r = rt.registry_mut();
                r.register_with(|id| CountingUnit::new(id, 4));
                r.register_with(|id| CountingUnit::new(id, 6));
            })
            .build())
    }
    let a = build_and_run();
    let b = build_and_run();
    assert_eq!(a.outcome, b.outcome);
    assert_eq!(a.steps_taken, b.steps_taken);
    assert_eq!(a.trace_bytes, b.trace_bytes);
    assert_eq!(a.final_memory_hash, b.final_memory_hash);
    assert_eq!(a.final_unit_status_hash, b.final_unit_status_hash);
}
