//! Assertions: invariants, golden trace checks, state-equivalence checks.
//!
//! Three kinds:
//!
//! - invariants: no direct mutation, no partial batches visible, blocked
//!   units not scheduled, monotonic event ordering, guest time never
//!   reverses
//! - golden trace: exact equality for the curated core set,
//!   prefix/fragment matching for in-flux scenarios
//! - state equivalence: committed memory, runnable queue, sync state,
//!   unit status hashes
//!
//! All assertions consume structured trace records and hashes. Never
//! assert on human-readable logs.
//!
//! Provides state-equivalence replay assertions
//! ([`assert_deterministic_replay`]), runtime-invariant assertions
//! ([`assert_guest_time_monotonic`], [`assert_epoch_strictly_increasing`],
//! [`assert_finished_units_not_rescheduled`]), and golden-trace
//! assertions (see [`crate::golden`]).

use crate::fixtures::ScenarioFixture;
use crate::runner::{run, ScenarioResult};
use cellgov_trace::{TraceReader, TraceRecord};

/// Build a fresh fixture `runs` times via `factory`, run each via the
/// canonical [`crate::runner::run`] path, and assert that every run
/// produced byte-equal trace bytes and equal final state hashes
/// against the first run.
///
/// The same scenario must reproduce identically across repeated runs.
/// Tests use it as the one-call shorthand for "this scenario is
/// deterministic".
///
/// `runs` must be at least 2; `runs == 0` or `runs == 1` panics
/// because no replay comparison is possible.
///
/// Returns the first run's [`ScenarioResult`] so callers can assert
/// further on the canonical run (steps taken, terminal outcome,
/// specific trace contents) without rebuilding the fixture.
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

/// Assert that guest time in `UnitScheduled` trace records never
/// decreases across the run.
///
/// Guest time must be monotonic across the entire runtime, not
/// per-unit. A violation
/// here means the runtime advanced time backward, which would break
/// ordering guarantees for every downstream consumer (event queue,
/// commit ordering, replay).
///
/// Panics with a descriptive message naming the two records where
/// the regression occurred.
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

/// Assert that epoch values in `CommitApplied` trace records are
/// strictly increasing across the run.
///
/// Epoch advances at commit boundaries. Two commits
/// must never share the same epoch, and the sequence must never go
/// backward. A violation here means the commit pipeline skipped or
/// duplicated an epoch, which would break state-hash checkpoints and
/// replay comparison.
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

/// Assert that no `UnitScheduled` record names a unit whose most
/// recent effective status (as implied by the trace) was `Blocked`
/// or `Finished`.
///
/// This is a trace-level proxy for the rule that blocked units must
/// not be scheduled. It walks `UnitScheduled` and
/// `StepCompleted` records to track the last observed yield reason
/// per unit: `Finished` means the unit should never be scheduled
/// again, and blocking yield reasons (`MailboxAccess`,
/// `DmaSubmitted`, `DmaWait`, `WaitingSync`) mean the unit should
/// stay out of the runnable set until explicitly woken. The check is
/// approximate (the trace does not record explicit wake events as
/// separate records in this slice), so it only flags the most
/// obvious violations: scheduling a unit whose last yield was
/// `Finished`.
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
        // A WritingUnit mutates committed memory; the final memory
        // hash is non-trivial and must still match across runs.
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
