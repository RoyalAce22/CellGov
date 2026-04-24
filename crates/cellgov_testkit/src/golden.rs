//! Golden-trace assertions: pin the exact record sequence a curated scenario
//! produces.
//!
//! Replay assertions only compare two runs against each other; goldens pin
//! against a known baseline and catch structural drift no replay check
//! observes.

use cellgov_trace::{TraceReader, TraceRecord};

/// Decode `actual_bytes` and assert exact equality against `expected`.
///
/// # Panics
///
/// Panics with scenario name, divergent record index, and the two records
/// on mismatch.
pub fn assert_golden_trace(scenario: &str, actual_bytes: &[u8], expected: &[TraceRecord]) {
    let actual: Vec<TraceRecord> = TraceReader::new(actual_bytes)
        .map(|r| r.expect("golden trace decode failed"))
        .collect();

    if actual.len() != expected.len() {
        panic!(
            "golden trace mismatch for '{scenario}': \
             expected {exp} records, got {act}",
            exp = expected.len(),
            act = actual.len()
        );
    }

    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(a, e, "golden trace mismatch for '{scenario}' at record {i}");
    }
}

/// Like [`assert_golden_trace`], but only checks the first `expected.len()`
/// records.
///
/// For scenarios whose tail is in flux but whose prefix is stable.
pub fn assert_golden_trace_prefix(
    scenario: &str,
    actual_bytes: &[u8],
    expected_prefix: &[TraceRecord],
) {
    let actual: Vec<TraceRecord> = TraceReader::new(actual_bytes)
        .map(|r| r.expect("golden trace decode failed"))
        .collect();

    if actual.len() < expected_prefix.len() {
        panic!(
            "golden trace prefix mismatch for '{scenario}': \
             expected at least {exp} records, got {act}",
            exp = expected_prefix.len(),
            act = actual.len()
        );
    }

    for (i, (a, e)) in actual.iter().zip(expected_prefix.iter()).enumerate() {
        assert_eq!(
            a, e,
            "golden trace prefix mismatch for '{scenario}' at record {i}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::fake_isa_scenario;
    use crate::runner::run;
    use cellgov_event::UnitId;
    use cellgov_time::{Budget, Epoch, GuestTicks};
    use cellgov_trace::{HashCheckpointKind, StateHash, TracedEffectKind, TracedYieldReason};

    /// Expected golden record sequence for `fake_isa_scenario`.
    ///
    /// Hash fields are `StateHash::ZERO` placeholders; the test patches them
    /// in from the actual run. Any structural drift fails the test.
    fn fake_isa_golden_records() -> Vec<TraceRecord> {
        // Program: LoadImm(0xAB), SharedStore{0,4}, MailboxSend{0}, End.
        // 4 steps, budget=1, time advances 1 per step.
        let u0 = UnitId::new(0);
        let b1 = Budget::new(1);
        vec![
            // Step 1: LoadImm(0xAB)
            TraceRecord::UnitScheduled {
                unit: u0,
                granted_budget: b1,
                time: GuestTicks::new(0),
                epoch: Epoch::new(0),
            },
            TraceRecord::StepCompleted {
                unit: u0,
                yield_reason: TracedYieldReason::BudgetExhausted,
                consumed_budget: b1,
                time_after: GuestTicks::new(1),
            },
            TraceRecord::CommitApplied {
                unit: u0,
                writes_committed: 0,
                effects_deferred: 0,
                fault_discarded: false,
                epoch_after: Epoch::new(1),
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::CommittedMemory,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::RunnableQueue,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::UnitStatus,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::SyncState,
                hash: StateHash::ZERO,
            },
            // Step 2: SharedStore{addr:0, len:4}
            TraceRecord::UnitScheduled {
                unit: u0,
                granted_budget: b1,
                time: GuestTicks::new(1),
                epoch: Epoch::new(1),
            },
            TraceRecord::StepCompleted {
                unit: u0,
                yield_reason: TracedYieldReason::BudgetExhausted,
                consumed_budget: b1,
                time_after: GuestTicks::new(2),
            },
            TraceRecord::EffectEmitted {
                unit: u0,
                sequence: 0,
                kind: TracedEffectKind::SharedWriteIntent,
            },
            TraceRecord::CommitApplied {
                unit: u0,
                writes_committed: 1,
                effects_deferred: 0,
                fault_discarded: false,
                epoch_after: Epoch::new(2),
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::CommittedMemory,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::RunnableQueue,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::UnitStatus,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::SyncState,
                hash: StateHash::ZERO,
            },
            // Step 3: MailboxSend{mailbox:0}
            TraceRecord::UnitScheduled {
                unit: u0,
                granted_budget: b1,
                time: GuestTicks::new(2),
                epoch: Epoch::new(2),
            },
            TraceRecord::StepCompleted {
                unit: u0,
                yield_reason: TracedYieldReason::MailboxAccess,
                consumed_budget: b1,
                time_after: GuestTicks::new(3),
            },
            TraceRecord::EffectEmitted {
                unit: u0,
                sequence: 0,
                kind: TracedEffectKind::MailboxSend,
            },
            TraceRecord::CommitApplied {
                unit: u0,
                writes_committed: 0,
                effects_deferred: 0,
                fault_discarded: false,
                epoch_after: Epoch::new(3),
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::CommittedMemory,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::RunnableQueue,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::UnitStatus,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::SyncState,
                hash: StateHash::ZERO,
            },
            // Step 4: End
            TraceRecord::UnitScheduled {
                unit: u0,
                granted_budget: b1,
                time: GuestTicks::new(3),
                epoch: Epoch::new(3),
            },
            TraceRecord::StepCompleted {
                unit: u0,
                yield_reason: TracedYieldReason::Finished,
                consumed_budget: b1,
                time_after: GuestTicks::new(4),
            },
            TraceRecord::CommitApplied {
                unit: u0,
                writes_committed: 0,
                effects_deferred: 0,
                fault_discarded: false,
                epoch_after: Epoch::new(4),
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::CommittedMemory,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::RunnableQueue,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::UnitStatus,
                hash: StateHash::ZERO,
            },
            TraceRecord::StateHashCheckpoint {
                kind: HashCheckpointKind::SyncState,
                hash: StateHash::ZERO,
            },
        ]
    }

    #[test]
    fn fake_isa_golden_trace_structure() {
        // Patch hash placeholders from the run so the test pins structure
        // (record types, ids, yield reasons, epoch/time progression) only.
        let result = run(fake_isa_scenario());
        let actual: Vec<TraceRecord> = TraceReader::new(&result.trace_bytes)
            .map(|r| r.expect("decode"))
            .collect();

        let mut expected = fake_isa_golden_records();
        assert_eq!(actual.len(), expected.len());

        for (a, e) in actual.iter().zip(expected.iter_mut()) {
            if let (
                TraceRecord::StateHashCheckpoint { hash: ah, kind: ak },
                TraceRecord::StateHashCheckpoint { hash: eh, kind: ek },
            ) = (a, e)
            {
                assert_eq!(ak, ek);
                *eh = *ah;
            }
        }

        assert_golden_trace("fake-isa", &result.trace_bytes, &expected);
    }

    #[test]
    fn prefix_match_succeeds_with_shorter_expected() {
        let result = run(fake_isa_scenario());
        let actual: Vec<TraceRecord> = TraceReader::new(&result.trace_bytes)
            .map(|r| r.expect("decode"))
            .collect();
        assert_golden_trace_prefix("fake-isa-prefix", &result.trace_bytes, &actual[..3]);
    }

    #[test]
    #[should_panic(expected = "golden trace mismatch")]
    fn exact_match_fails_on_wrong_record() {
        let result = run(fake_isa_scenario());
        let wrong = vec![TraceRecord::CommitApplied {
            unit: UnitId::new(99),
            writes_committed: 0,
            effects_deferred: 0,
            fault_discarded: false,
            epoch_after: Epoch::new(0),
        }];
        assert_golden_trace("should-fail", &result.trace_bytes, &wrong);
    }
}
