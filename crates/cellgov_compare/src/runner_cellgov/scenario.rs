//! Scenario-runner adapter: `observe` converts a `ScenarioResult` into
//! the shared `Observation` schema, with optional twice-run determinism
//! check.

use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome, ScenarioResult};
use cellgov_trace::{TraceReader, TraceRecord, TracedEffectKind, TracedWakeReason};

use crate::observation::{
    Observation, ObservationMetadata, ObservedEvent, ObservedEventKind, ObservedHashes,
    ObservedOutcome,
};

use super::region::{extract_regions, RegionDescriptor};

/// Convert a `ScenarioResult` into a normalized `Observation`.
///
/// Out-of-bounds regions are filled with zeros; the comparison layer
/// catches the mismatch.
///
/// Outcome mapping: `Stalled` -> `Completed`, `MaxStepsExceeded` -> `Timeout`.
pub fn observe(result: &ScenarioResult, regions: &[RegionDescriptor]) -> Observation {
    let outcome = match result.outcome {
        ScenarioOutcome::Stalled => ObservedOutcome::Completed,
        ScenarioOutcome::MaxStepsExceeded => ObservedOutcome::Timeout,
    };

    let memory_regions = extract_regions(&result.final_memory, regions);
    let events = extract_events(&result.trace_bytes);

    let state_hashes = Some(ObservedHashes {
        memory: result.final_memory_hash,
        unit_status: result.final_unit_status_hash,
        sync: result.final_sync_hash,
    });

    Observation {
        outcome,
        memory_regions,
        events,
        state_hashes,
        metadata: ObservationMetadata {
            runner: "cellgov".into(),
            steps: Some(result.steps_taken),
        },
        // Scenario runner has no LV2 host with a TTY surface; left
        // empty until we add structural-test TTY capture.
        tty_log: Vec::new(),
    }
}

/// Decode the binary trace and coalesce into semantic events.
fn extract_events(trace_bytes: &[u8]) -> Vec<ObservedEvent> {
    let mut events = Vec::new();
    let mut seq: u32 = 0;

    for record in TraceReader::new(trace_bytes).flatten() {
        let maybe = match record {
            TraceRecord::EffectEmitted { unit, kind, .. } => match kind {
                TracedEffectKind::MailboxSend => Some((ObservedEventKind::MailboxSend, unit.raw())),
                TracedEffectKind::MailboxReceiveAttempt => {
                    Some((ObservedEventKind::MailboxReceive, unit.raw()))
                }
                TracedEffectKind::DmaEnqueue => Some((ObservedEventKind::DmaComplete, unit.raw())),
                _ => None,
            },
            TraceRecord::UnitBlocked { unit, .. } => {
                Some((ObservedEventKind::UnitBlock, unit.raw()))
            }
            TraceRecord::UnitWoken { unit, reason } => {
                let kind = match reason {
                    TracedWakeReason::DmaCompletion => ObservedEventKind::DmaComplete,
                    TracedWakeReason::WakeEffect => ObservedEventKind::UnitWake,
                };
                Some((kind, unit.raw()))
            }
            _ => None,
        };

        if let Some((kind, unit)) = maybe {
            events.push(ObservedEvent {
                kind,
                unit,
                sequence: seq,
            });
            seq += 1;
        }
    }

    events
}

/// Why a determinism check failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeterminismError {
    /// The two runs produced different outcomes.
    OutcomeMismatch,
    /// The two runs produced different memory region contents.
    MemoryMismatch,
    /// The two runs produced different event sequences.
    EventMismatch,
    /// The two runs produced different state hashes.
    HashMismatch,
}

impl std::fmt::Display for DeterminismError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutcomeMismatch => f.write_str("two runs produced different outcomes"),
            Self::MemoryMismatch => f.write_str("two runs produced different memory contents"),
            Self::EventMismatch => f.write_str("two runs produced different event sequences"),
            Self::HashMismatch => f.write_str("two runs produced different state hashes"),
        }
    }
}

impl std::error::Error for DeterminismError {}

/// Run a scenario factory twice and verify both observations match;
/// returns the observation, or the first field that diverged.
pub fn observe_with_determinism_check(
    factory: impl Fn() -> ScenarioFixture,
    regions: &[RegionDescriptor],
) -> Result<Observation, DeterminismError> {
    let r1 = runner::run(factory());
    let r2 = runner::run(factory());
    let o1 = observe(&r1, regions);
    let o2 = observe(&r2, regions);

    if o1.outcome != o2.outcome {
        return Err(DeterminismError::OutcomeMismatch);
    }
    if o1.memory_regions != o2.memory_regions {
        return Err(DeterminismError::MemoryMismatch);
    }
    if o1.events != o2.events {
        return Err(DeterminismError::EventMismatch);
    }
    if o1.state_hashes != o2.state_hashes {
        return Err(DeterminismError::HashMismatch);
    }

    Ok(o1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_testkit::fixtures;
    use cellgov_testkit::runner::run;

    #[test]
    fn observe_maps_stalled_to_completed() {
        let result = run(fixtures::round_robin_fairness_scenario(2, 3));
        let obs = observe(&result, &[]);
        assert_eq!(result.outcome, ScenarioOutcome::Stalled);
        assert_eq!(obs.outcome, ObservedOutcome::Completed);
    }

    #[test]
    fn observe_carries_state_hashes() {
        let result = run(fixtures::round_robin_fairness_scenario(2, 3));
        let obs = observe(&result, &[]);
        let hashes = obs.state_hashes.unwrap();
        assert_eq!(hashes.memory, result.final_memory_hash);
        assert_eq!(hashes.unit_status, result.final_unit_status_hash);
        assert_eq!(hashes.sync, result.final_sync_hash);
    }

    #[test]
    fn observe_metadata_says_cellgov() {
        let result = run(fixtures::round_robin_fairness_scenario(1, 1));
        let obs = observe(&result, &[]);
        assert_eq!(obs.metadata.runner, "cellgov");
        assert!(obs.metadata.steps.is_some());
    }

    #[test]
    fn observe_extracts_events_from_mailbox_scenario() {
        let result = run(fixtures::mailbox_send_scenario(3));
        let obs = observe(&result, &[]);
        assert!(
            obs.events
                .iter()
                .any(|e| e.kind == ObservedEventKind::MailboxSend),
            "expected at least one MailboxSend event"
        );
    }

    #[test]
    fn observe_extracts_block_wake_from_dma_scenario() {
        let result = run(fixtures::dma_block_unblock_scenario());
        let obs = observe(&result, &[]);
        assert!(
            obs.events
                .iter()
                .any(|e| e.kind == ObservedEventKind::UnitBlock),
            "expected at least one UnitBlock event"
        );
        assert!(
            obs.events
                .iter()
                .any(|e| e.kind == ObservedEventKind::DmaComplete),
            "expected at least one DmaComplete event"
        );
    }

    #[test]
    fn observe_event_sequences_are_monotonic() {
        let result = run(fixtures::mailbox_roundtrip_scenario(0x42));
        let obs = observe(&result, &[]);
        for (i, event) in obs.events.iter().enumerate() {
            assert_eq!(event.sequence, i as u32);
        }
    }

    #[test]
    fn two_identical_runs_produce_identical_observations() {
        let r1 = run(fixtures::fake_isa_scenario());
        let r2 = run(fixtures::fake_isa_scenario());
        let regions = vec![RegionDescriptor {
            name: "shared".into(),
            addr: 0,
            size: 8,
        }];
        let o1 = observe(&r1, &regions);
        let o2 = observe(&r2, &regions);
        assert_eq!(o1.outcome, o2.outcome);
        assert_eq!(o1.memory_regions, o2.memory_regions);
        assert_eq!(o1.events, o2.events);
        assert_eq!(o1.state_hashes, o2.state_hashes);
    }

    type ScenarioFactory = Box<dyn Fn() -> cellgov_testkit::ScenarioFixture>;

    #[test]
    fn determinism_check_passes_for_all_scenarios() {
        let scenarios: Vec<(&str, ScenarioFactory)> = vec![
            (
                "fairness",
                Box::new(|| fixtures::round_robin_fairness_scenario(3, 5)),
            ),
            (
                "conflict",
                Box::new(|| fixtures::write_conflict_scenario(3)),
            ),
            (
                "mailbox",
                Box::new(|| fixtures::mailbox_roundtrip_scenario(0x42)),
            ),
            ("dma", Box::new(fixtures::dma_block_unblock_scenario)),
            ("send", Box::new(|| fixtures::mailbox_send_scenario(5))),
            ("signal", Box::new(|| fixtures::signal_update_scenario(4))),
            ("isa", Box::new(fixtures::fake_isa_scenario)),
        ];
        for (name, factory) in &scenarios {
            let result = observe_with_determinism_check(factory, &[]);
            assert!(
                result.is_ok(),
                "determinism check failed for {name}: {:?}",
                result.err()
            );
        }
    }
}
