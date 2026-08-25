//! Scenario-runner adapter: `observe` converts a `ScenarioResult` into
//! the shared `Observation` schema, with optional twice-run determinism
//! check.

use cellgov_testkit::fixtures::ScenarioFixture;
use cellgov_testkit::runner::{self, ScenarioOutcome, ScenarioResult};
use cellgov_trace::{DecodeError, TraceReader, TraceRecord, TracedEffectKind, TracedWakeReason};

use crate::observation::{
    Observation, ObservationMetadata, ObservedEvent, ObservedEventKind, ObservedHashes,
    ObservedOutcome,
};

use super::region::{extract_regions, RegionDescriptor};

/// A record in a run's own trace stream failed to decode.
///
/// The stream is this workspace's encoder's output, so a bad record
/// is a host invariant break: the events before it are a prefix, and
/// a comparison over that prefix could mask or fabricate a divergence
/// at the cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("trace record {index} at byte offset {offset} failed to decode: {source}")]
pub struct TraceDecodeError {
    /// Zero-based index of the record that failed.
    pub index: usize,
    /// Byte offset of that record's first byte in the trace stream.
    pub offset: usize,
    /// The decoder's own reason.
    #[source]
    pub source: DecodeError,
}

/// Convert a `ScenarioResult` into a normalized `Observation`.
///
/// Regions that do not resolve in their space are filled with zeros;
/// the comparison layer catches the mismatch.
///
/// # Errors
///
/// [`TraceDecodeError`] when the run's trace stream does not decode
/// end to end; no observation is produced from a partial stream.
pub fn observe(
    result: &ScenarioResult,
    regions: &[RegionDescriptor],
) -> Result<Observation, TraceDecodeError> {
    let outcome = match result.outcome {
        ScenarioOutcome::Stalled => ObservedOutcome::Completed,
        ScenarioOutcome::MaxStepsExceeded => ObservedOutcome::Timeout,
    };

    let memory_regions = extract_regions(&result.final_spaces, regions);
    let events = extract_events(&result.trace_bytes)?;

    let state_hashes = Some(ObservedHashes {
        memory: result.final_memory_hash,
        unit_status: result.final_unit_status_hash,
        sync: result.final_sync_hash,
    });

    Ok(Observation {
        outcome,
        memory_regions,
        events,
        state_hashes,
        metadata: ObservationMetadata {
            runner: "cellgov".into(),
            steps: Some(result.steps_taken),
        },
        // Scenario runner has no LV2 host with a TTY surface.
        tty_log: Vec::new(),
    })
}

/// Decode the binary trace and coalesce into semantic events, stopping
/// at the first record that does not decode.
fn extract_events(trace_bytes: &[u8]) -> Result<Vec<ObservedEvent>, TraceDecodeError> {
    let mut events = Vec::new();
    let mut seq: u32 = 0;

    let mut reader = TraceReader::new(trace_bytes);
    let mut index = 0usize;
    loop {
        let offset = reader.position();
        let record = match reader.next() {
            None => break,
            Some(Ok(record)) => record,
            Some(Err(source)) => {
                return Err(TraceDecodeError {
                    index,
                    offset,
                    source,
                })
            }
        };
        index += 1;
        let maybe = match record {
            TraceRecord::EffectEmitted { unit, kind, .. } => match kind {
                TracedEffectKind::MailboxSend => Some((ObservedEventKind::MailboxSend, unit.raw())),
                TracedEffectKind::MailboxReceiveAttempt => {
                    Some((ObservedEventKind::MailboxReceive, unit.raw()))
                }
                TracedEffectKind::DmaEnqueue => Some((ObservedEventKind::DmaComplete, unit.raw())),
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
                    TracedWakeReason::WakeEffect | TracedWakeReason::Timer => {
                        ObservedEventKind::UnitWake
                    }
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

    Ok(events)
}

/// Why a determinism check failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeterminismError {
    /// A run's own trace stream did not decode, so there is nothing
    /// whole to compare.
    #[error("trace decode: {0}")]
    TraceDecode(#[from] TraceDecodeError),
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
///
/// # Errors
///
/// [`DeterminismError::TraceDecode`] before any field comparison when
/// either run's trace does not decode; otherwise the first field that
/// differs between the runs.
pub fn observe_with_determinism_check(
    factory: impl Fn() -> ScenarioFixture,
    regions: &[RegionDescriptor],
) -> Result<Observation, DeterminismError> {
    let r1 = runner::run(factory());
    let r2 = runner::run(factory());
    let o1 = observe(&r1, regions)?;
    let o2 = observe(&r2, regions)?;

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
