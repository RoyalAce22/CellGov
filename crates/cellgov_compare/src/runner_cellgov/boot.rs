//! Boot adapter: converts a finished boot run into the shared
//! [`Observation`] schema.

use crate::observation::{Observation, ObservationMetadata, ObservedOutcome};

use super::region::{extract_regions, RegionDescriptor};

/// How a boot run terminated. Maps to [`ObservedOutcome`] via
/// [`observe_from_boot`]; variants here carry more detail than the
/// shared observation enum so the caller can distinguish between
/// step-budget vs internal-time-counter stops, and between an
/// RSX-write checkpoint vs a generic PC checkpoint.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error,
)]
pub enum BootOutcome {
    /// Title called `sys_process_exit`.
    #[error("ProcessExit")]
    ProcessExit,
    /// Explicit runtime or architectural fault.
    #[error("Fault")]
    Fault,
    /// Step budget exhausted before any other stop condition fired.
    #[error("MaxSteps")]
    MaxSteps,
    /// First PPU write into the RSX command region. Used as a
    /// cross-runner checkpoint for titles whose attract-mode loops
    /// never exit on their own.
    #[error("RsxWriteCheckpoint")]
    RsxWriteCheckpoint,
    /// A step retired with its PC equal to a caller-supplied PC.
    #[error("PcReached(0x{0:x})")]
    PcReached(u64),
    /// Internal time counter overflowed; distinct from `Fault` so a
    /// `TimeOverflow` / `Fault` cross-runner pair is not
    /// miscategorized as "both faulted".
    #[error("TimeOverflow")]
    TimeOverflow,
}

/// Why parsing a [`BootOutcome`] from its `Display` form failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BootOutcomeParseError {
    /// The token did not match any known variant name.
    #[error("unknown BootOutcome variant: {0:?}")]
    UnknownVariant(String),
    /// `PcReached(...)` payload is not `0x`-prefixed hex.
    #[error("malformed PcReached payload: {0:?} (expected `0x<hex>`)")]
    MalformedPcReached(String),
}

impl std::str::FromStr for BootOutcome {
    type Err = BootOutcomeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ProcessExit" => Ok(Self::ProcessExit),
            "Fault" => Ok(Self::Fault),
            "MaxSteps" => Ok(Self::MaxSteps),
            "RsxWriteCheckpoint" => Ok(Self::RsxWriteCheckpoint),
            "TimeOverflow" => Ok(Self::TimeOverflow),
            other => {
                let payload = other
                    .strip_prefix("PcReached(")
                    .and_then(|s| s.strip_suffix(')'))
                    .ok_or_else(|| BootOutcomeParseError::UnknownVariant(other.to_string()))?;
                let addr_hex = payload.strip_prefix("0x").ok_or_else(|| {
                    BootOutcomeParseError::MalformedPcReached(payload.to_string())
                })?;
                u64::from_str_radix(addr_hex, 16)
                    .map(Self::PcReached)
                    .map_err(|_| BootOutcomeParseError::MalformedPcReached(payload.to_string()))
            }
        }
    }
}

/// Build an `Observation` from a completed boot run.
///
/// State hashes are `None`: the boot path does not retain the per-step
/// hashes the scenario runner accumulates.
///
/// `tty_log` is the captured `sys_tty_write` byte stream from the
/// boot's `Lv2Host`; pass an empty slice when the caller has no TTY
/// data to record.
///
/// Outcome mapping:
/// - `ProcessExit` -> `ProcessExit` (the title called sys_process_exit;
///   may be a legitimate title-side shutdown or a synthesized exit
///   from a fault path, e.g. an unresolved import returning EINVAL).
/// - `RsxWriteCheckpoint`, `PcReached` -> `Completed` (the harness
///   stopped the run at a designated checkpoint).
/// - `MaxSteps`, `TimeOverflow` -> `Timeout`.
/// - `Fault` -> `Fault`.
pub fn observe_from_boot(
    final_memory: &[u8],
    outcome: BootOutcome,
    steps_taken: usize,
    regions: &[RegionDescriptor],
    tty_log: &[u8],
) -> Observation {
    let observed_outcome = match outcome {
        BootOutcome::ProcessExit => ObservedOutcome::ProcessExit,
        BootOutcome::Fault => ObservedOutcome::Fault,
        BootOutcome::MaxSteps => ObservedOutcome::Timeout,
        BootOutcome::RsxWriteCheckpoint => ObservedOutcome::Completed,
        BootOutcome::PcReached(_) => ObservedOutcome::Completed,
        BootOutcome::TimeOverflow => ObservedOutcome::Timeout,
    };

    Observation {
        outcome: observed_outcome,
        memory_regions: extract_regions(final_memory, regions),
        events: Vec::new(),
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: "cellgov-boot".into(),
            steps: Some(steps_taken),
        },
        tty_log: tty_log.to_vec(),
    }
}

#[cfg(test)]
#[path = "tests/boot_tests.rs"]
mod tests;
