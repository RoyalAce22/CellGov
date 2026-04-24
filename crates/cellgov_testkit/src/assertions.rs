//! Typed assertion helpers for scenario traces and state hashes.
//!
//! Assertions consume structured trace records and hashes; never human-readable
//! logs.

use crate::fixtures::ScenarioFixture;
use crate::runner::{run, ScenarioResult};
use cellgov_trace::{TraceReader, TraceRecord};

/// Run `factory` `runs` times and assert every run's trace bytes and final
/// hashes match the first.
///
/// Returns the first run's [`ScenarioResult`].
///
/// # Panics
///
/// Panics if `runs < 2` (no comparison possible), or on any cross-run mismatch.
pub fn assert_deterministic_replay<F>(mut factory: F, runs: usize) -> ScenarioResult
where
    F: FnMut() -> ScenarioFixture,
{
    assert!(
        runs >= 2,
        "assert_deterministic_replay requires at least 2 runs, got {runs}"
    );
    let first = run(factory());
    for i in 1..runs {
        let nth = run(factory());
        assert_eq!(
            nth.outcome, first.outcome,
            "run {i} terminated with a different outcome than run 0"
        );
        assert_eq!(
            nth.steps_taken, first.steps_taken,
            "run {i} took {} steps, run 0 took {}",
            nth.steps_taken, first.steps_taken
        );
        assert_eq!(
            nth.trace_bytes, first.trace_bytes,
            "run {i} produced a different trace byte sequence than run 0"
        );
        assert_eq!(
            nth.final_memory_hash, first.final_memory_hash,
            "run {i} produced a different final committed-memory hash than run 0"
        );
        assert_eq!(
            nth.final_unit_status_hash, first.final_unit_status_hash,
            "run {i} produced a different final unit-status hash than run 0"
        );
        assert_eq!(
            nth.final_sync_hash, first.final_sync_hash,
            "run {i} produced a different final sync-state hash than run 0"
        );
    }
    first
}

/// Assert guest time in `UnitScheduled` records never decreases.
///
/// # Panics
///
/// Panics naming the two records on regression.
pub fn assert_guest_time_monotonic(trace_bytes: &[u8]) {
    let mut prev_time = None;
    let mut prev_index = 0usize;
    for (i, record) in TraceReader::new(trace_bytes)
        .map(|r| r.expect("decode"))
        .enumerate()
    {
        if let TraceRecord::UnitScheduled { time, .. } = record {
            if let Some(prev) = prev_time {
                assert!(
                    time.raw() >= prev,
                    "guest time regressed: record {prev_index} had time {prev}, \
                     record {i} has time {}",
                    time.raw()
                );
            }
            prev_time = Some(time.raw());
            prev_index = i;
        }
    }
}

/// Assert `CommitApplied.epoch_after` values are strictly increasing.
pub fn assert_epoch_strictly_increasing(trace_bytes: &[u8]) {
    let mut prev_epoch = None;
    let mut prev_index = 0usize;
    for (i, record) in TraceReader::new(trace_bytes)
        .map(|r| r.expect("decode"))
        .enumerate()
    {
        if let TraceRecord::CommitApplied { epoch_after, .. } = record {
            if let Some(prev) = prev_epoch {
                assert!(
                    epoch_after.raw() > prev,
                    "epoch did not strictly increase: record {prev_index} had epoch {prev}, \
                     record {i} has epoch {}",
                    epoch_after.raw()
                );
            }
            prev_epoch = Some(epoch_after.raw());
            prev_index = i;
        }
    }
}

/// Assert no `UnitScheduled` record names a unit whose last `StepCompleted`
/// yield was `Finished`.
///
/// Approximate: only catches the most obvious reschedule-after-finish
/// violations, since the trace does not record explicit wake events as
/// separate records.
pub fn assert_finished_units_not_rescheduled(trace_bytes: &[u8]) {
    use cellgov_trace::TracedYieldReason;
    use std::collections::BTreeMap;

    let mut last_yield: BTreeMap<u64, TracedYieldReason> = BTreeMap::new();

    for record in TraceReader::new(trace_bytes).map(|r| r.expect("decode")) {
        match record {
            TraceRecord::UnitScheduled { unit, .. } => {
                if let Some(&reason) = last_yield.get(&unit.raw()) {
                    assert_ne!(
                        reason,
                        TracedYieldReason::Finished,
                        "unit {} was scheduled after it yielded Finished",
                        unit.raw()
                    );
                }
            }
            TraceRecord::StepCompleted {
                unit, yield_reason, ..
            } => {
                last_yield.insert(unit.raw(), yield_reason);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
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
}
