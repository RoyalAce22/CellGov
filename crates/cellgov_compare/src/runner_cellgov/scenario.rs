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
use crate::trace_decode::TraceDecodeError;

use super::region::{extract_regions, RegionDescriptor, RegionExtractError};

/// Why a scenario run produced no observation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ObserveError {
    /// The run's trace stream did not decode end to end.
    #[error("trace decode: {0}")]
    TraceDecode(#[from] TraceDecodeError),
    /// The extractor refused a requested region.
    #[error("{0}")]
    Region(#[from] RegionExtractError),
}

/// Convert a `ScenarioResult` into a normalized `Observation`.
///
/// # Errors
///
/// - [`ObserveError::Region`] when the extractor refuses one of `regions`.
/// - [`ObserveError::TraceDecode`] when the run's trace stream does not
///   decode end to end.
pub fn observe(
    result: &ScenarioResult,
    regions: &[RegionDescriptor],
) -> Result<Observation, ObserveError> {
    let outcome = match result.outcome {
        ScenarioOutcome::Stalled => ObservedOutcome::Completed,
        ScenarioOutcome::MaxStepsExceeded => ObservedOutcome::Timeout,
    };

    let memory_regions = extract_regions(&result.final_spaces, regions)?;
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
        // A synthetic scenario composes no firmware and no title.
        identity: crate::identity::RunIdentity::default(),
        runner_firmware: None,
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
                | TracedEffectKind::RsxFlipRequest
                | TracedEffectKind::SharedReadIntent => None,
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

/// How the two runs of a determinism check disagreed on whether an
/// observation exists at all.
///
/// Most refusals read run state, so two runs of one factory can
/// refuse differently. Such a refusal is a determinism break.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ObserveDisagreement {
    /// The first run refused to observe; the second observed.
    #[error("the first run produced no observation and the second did: {0}")]
    FirstOnly(ObserveError),
    /// The second run refused to observe; the first observed.
    #[error("the second run produced no observation and the first did: {0}")]
    SecondOnly(ObserveError),
    /// Both runs refused, for different reasons.
    #[error(
        "the two runs refused to observe for different reasons; first: {first}; second: {second}"
    )]
    Both {
        /// Why the first run refused.
        first: ObserveError,
        /// Why the second run refused.
        second: ObserveError,
    },
}

/// Why a determinism check failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeterminismError {
    /// Both runs refused to observe the same way, so there is nothing
    /// whole to compare.
    #[error("{0}")]
    Observe(#[from] ObserveError),
    /// The runs disagreed on whether an observation exists.
    #[error("{0}")]
    ObserveDisagreement(#[from] Box<ObserveDisagreement>),
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

/// A determinism-checked scenario run: what the comparison reads, and
/// what the host recorded about itself while producing it.
#[derive(Debug, Clone)]
pub struct CheckedRun {
    /// The observation both runs agreed on, as the first run produced
    /// it.
    pub observation: Observation,
    /// The first host invariant break of the run [`Self::observation`]
    /// came from, ready for the driver to report. Diagnostic only: it
    /// stays out of [`Observation`], whose fields a cross-runner
    /// comparison reads.
    pub first_invariant_break: Option<String>,
}

/// Run a scenario factory twice and verify both observations match;
/// returns the observation, or the first field that diverged.
///
/// Use [`observe_checked`] where the caller reports the run's first host
/// invariant break.
///
/// # Errors
///
/// See [`observe_checked`].
pub fn observe_with_determinism_check(
    factory: impl Fn() -> ScenarioFixture,
    regions: &[RegionDescriptor],
) -> Result<Observation, DeterminismError> {
    observe_checked(factory, regions).map(|run| run.observation)
}

/// Run a scenario factory twice, verify both observations match, and
/// keep the first run's host invariant break for the caller to report.
///
/// # Errors
///
/// - [`DeterminismError::Observe`] when both runs refuse to observe
///   the same way; this fires before any field comparison.
/// - [`DeterminismError::ObserveDisagreement`] when one run observes
///   and the other refuses, or both refuse for different reasons.
/// - Otherwise, the first field that differs between the runs.
pub fn observe_checked(
    factory: impl Fn() -> ScenarioFixture,
    regions: &[RegionDescriptor],
) -> Result<CheckedRun, DeterminismError> {
    let r1 = runner::run(factory());
    let r2 = runner::run(factory());
    let (o1, o2) = match (observe(&r1, regions), observe(&r2, regions)) {
        (Ok(o1), Ok(o2)) => (o1, o2),
        (Err(first), Err(second)) if first == second => {
            return Err(DeterminismError::Observe(first));
        }
        (Err(first), Err(second)) => {
            return Err(Box::new(ObserveDisagreement::Both { first, second }).into());
        }
        (Err(first), Ok(_)) => {
            return Err(Box::new(ObserveDisagreement::FirstOnly(first)).into());
        }
        (Ok(_), Err(second)) => {
            return Err(Box::new(ObserveDisagreement::SecondOnly(second)).into());
        }
    };

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

    Ok(CheckedRun {
        observation: o1,
        first_invariant_break: r1.first_invariant_break,
    })
}

#[cfg(test)]
#[path = "tests/scenario_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/scenario_disagreement_tests.rs"]
mod disagreement_tests;
