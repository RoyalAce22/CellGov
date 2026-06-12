//! Determinism-replay and trace-invariant assertions exercised across the stock scenario fixtures.

use super::*;
use crate::fixtures::ScenarioFixture;
use crate::runner::ScenarioOutcome;
use crate::world::{CountingUnit, WritingUnit};
use cellgov_core::Runtime;
use cellgov_time::Budget;

#[test]
fn empty_fixture_replays_identically() {
    let result = assert_deterministic_replay(ScenarioFixture::empty, 5);
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 0);
}

#[test]
fn counting_unit_scenario_replays_identically() {
    let result = assert_deterministic_replay(
        || {
            ScenarioFixture::builder()
                .memory_size(16)
                .budget(Budget::new(1))
                .max_steps(100)
                .register(|rt: &mut Runtime| {
                    let r = rt.registry_mut();
                    r.register_with(|id| CountingUnit::new(id, 5));
                    r.register_with(|id| CountingUnit::new(id, 7));
                })
                .build()
        },
        3,
    );
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 12);
}

#[test]
fn writing_unit_scenario_replays_identically() {
    let result = assert_deterministic_replay(
        || {
            ScenarioFixture::builder()
                .memory_size(16)
                .budget(Budget::new(1))
                .max_steps(100)
                .register(|rt: &mut Runtime| {
                    let r = rt.registry_mut();
                    r.register_with(|id| WritingUnit::at_zero(id, 4));
                })
                .build()
        },
        4,
    );
    assert_eq!(result.outcome, ScenarioOutcome::Stalled);
    assert_eq!(result.steps_taken, 4);
}

#[test]
#[should_panic(expected = "requires at least 2 runs")]
fn replay_with_one_run_panics() {
    assert_deterministic_replay(ScenarioFixture::empty, 1);
}

#[test]
fn guest_time_monotonic_holds_for_fairness_scenario() {
    use crate::fixtures::round_robin_fairness_scenario;
    let result = run(round_robin_fairness_scenario(3, 5));
    assert_guest_time_monotonic(&result.trace_bytes);
}

#[test]
fn epoch_strictly_increasing_holds_for_fairness_scenario() {
    use crate::fixtures::round_robin_fairness_scenario;
    let result = run(round_robin_fairness_scenario(3, 5));
    assert_epoch_strictly_increasing(&result.trace_bytes);
}

#[test]
fn finished_units_not_rescheduled_in_fairness_scenario() {
    use crate::fixtures::round_robin_fairness_scenario;
    let result = run(round_robin_fairness_scenario(3, 5));
    assert_finished_units_not_rescheduled(&result.trace_bytes);
}

#[test]
fn all_invariants_hold_for_mailbox_roundtrip() {
    use crate::fixtures::mailbox_roundtrip_scenario;
    let result = run(mailbox_roundtrip_scenario(0x42));
    assert_guest_time_monotonic(&result.trace_bytes);
    assert_epoch_strictly_increasing(&result.trace_bytes);
    assert_finished_units_not_rescheduled(&result.trace_bytes);
}

#[test]
fn all_invariants_hold_for_dma_block_unblock() {
    use crate::fixtures::dma_block_unblock_scenario;
    let result = run(dma_block_unblock_scenario());
    assert_guest_time_monotonic(&result.trace_bytes);
    assert_epoch_strictly_increasing(&result.trace_bytes);
    assert_finished_units_not_rescheduled(&result.trace_bytes);
}

#[test]
fn all_invariants_hold_for_fake_isa() {
    use crate::fixtures::fake_isa_scenario;
    let result = run(fake_isa_scenario());
    assert_guest_time_monotonic(&result.trace_bytes);
    assert_epoch_strictly_increasing(&result.trace_bytes);
    assert_finished_units_not_rescheduled(&result.trace_bytes);
}
