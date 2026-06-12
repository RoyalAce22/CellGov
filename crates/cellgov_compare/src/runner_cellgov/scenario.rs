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
                // Exhaustive: a new TracedEffectKind must declare its
                // observed-event intent at compile time.
                TracedEffectKind::SharedWriteIntent
                | TracedEffectKind::WaitOnEvent
                | TracedEffectKind::WakeUnit
                | TracedEffectKind::SignalUpdate
                | TracedEffectKind::FaultRaised
                | TracedEffectKind::TraceMarker
                | TracedEffectKind::ReservationAcquire
                | TracedEffectKind::ConditionalStore
                | TracedEffectKind::RsxLabelWrite
                | TracedEffectKind::RsxFlipRequest => None,
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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeterminismError {
    /// The two runs produced different outcomes.
    #[error("two runs produced different outcomes")]
    OutcomeMismatch,
    /// The two runs produced different memory region contents.
    #[error("two runs produced different memory contents")]
    MemoryMismatch,
    /// The two runs produced different event sequences.
    #[error("two runs produced different event sequences")]
    EventMismatch,
    /// The two runs produced different state hashes.
    #[error("two runs produced different state hashes")]
    HashMismatch,
}

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
#[path = "tests/scenario_tests.rs"]
mod tests;
