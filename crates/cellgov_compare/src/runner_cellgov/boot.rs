//! Boot adapter: converts a finished boot run into the shared
//! [`Observation`] schema.

use crate::observation::{Observation, ObservationMetadata, ObservedOutcome};

use super::region::{extract_regions, RegionDescriptor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BootOutcome {
    ProcessExit,
    Fault,
    MaxSteps,
    /// First PPU write into the RSX command region. Used as a
    /// cross-runner checkpoint for titles whose attract-mode loops
    /// never exit on their own.
    RsxWriteCheckpoint,
    /// A step retired with its PC equal to a caller-supplied PC.
    PcReached(u64),
    /// Internal time counter overflowed; distinct from `Fault` so a
    /// `TimeOverflow` / `Fault` cross-runner pair is not
    /// miscategorized as "both faulted".
    TimeOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootOutcomeParseError {
    UnknownVariant(String),
    /// `PcReached(...)` payload is not `0x`-prefixed hex.
    MalformedPcReached(String),
}

impl std::fmt::Display for BootOutcomeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownVariant(s) => write!(f, "unknown BootOutcome variant: {s:?}"),
            Self::MalformedPcReached(s) => {
                write!(f, "malformed PcReached payload: {s:?} (expected `0x<hex>`)")
            }
        }
    }
}

impl std::error::Error for BootOutcomeParseError {}

impl std::fmt::Display for BootOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProcessExit => f.write_str("ProcessExit"),
            Self::Fault => f.write_str("Fault"),
            Self::MaxSteps => f.write_str("MaxSteps"),
            Self::RsxWriteCheckpoint => f.write_str("RsxWriteCheckpoint"),
            Self::PcReached(addr) => write!(f, "PcReached(0x{addr:x})"),
            Self::TimeOverflow => f.write_str("TimeOverflow"),
        }
    }
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

#[cfg(test)]
mod boot_outcome_round_trip {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn display_then_from_str_recovers_every_variant() {
        let variants = [
            BootOutcome::ProcessExit,
            BootOutcome::Fault,
            BootOutcome::MaxSteps,
            BootOutcome::RsxWriteCheckpoint,
            BootOutcome::PcReached(0x10381ce8),
            BootOutcome::TimeOverflow,
        ];
        for v in variants {
            let s = v.to_string();
            let parsed = BootOutcome::from_str(&s)
                .unwrap_or_else(|e| panic!("round-trip failed for {v:?} ({s:?}): {e}"));
            assert_eq!(parsed, v, "round-trip mismatch for {v:?} via {s:?}");
        }
    }

    #[test]
    fn from_str_unknown_variant_errors() {
        let err = BootOutcome::from_str("WhoKnows").unwrap_err();
        assert!(matches!(err, BootOutcomeParseError::UnknownVariant(_)));
    }

    #[test]
    fn from_str_pc_reached_without_hex_prefix_errors() {
        let err = BootOutcome::from_str("PcReached(1234)").unwrap_err();
        assert!(matches!(err, BootOutcomeParseError::MalformedPcReached(_)));
    }

    #[test]
    fn from_str_pc_reached_non_hex_errors() {
        let err = BootOutcome::from_str("PcReached(0xnothex)").unwrap_err();
        assert!(matches!(err, BootOutcomeParseError::MalformedPcReached(_)));
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
/// - `ProcessExit`, `RsxWriteCheckpoint`, `PcReached` -> `Completed`
/// - `MaxSteps`, `TimeOverflow` -> `Timeout`
/// - `Fault` -> `Fault`
pub fn observe_from_boot(
    final_memory: &[u8],
    outcome: BootOutcome,
    steps_taken: usize,
    regions: &[RegionDescriptor],
    tty_log: &[u8],
) -> Observation {
    let observed_outcome = match outcome {
        BootOutcome::ProcessExit => ObservedOutcome::Completed,
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
mod tests {
    use super::*;

    #[test]
    fn observe_from_boot_maps_process_exit_to_completed() {
        let mem = vec![0u8; 16];
        let obs = observe_from_boot(&mem, BootOutcome::ProcessExit, 1000, &[], &[]);
        assert_eq!(obs.outcome, ObservedOutcome::Completed);
        assert_eq!(obs.metadata.runner, "cellgov-boot");
        assert_eq!(obs.metadata.steps, Some(1000));
        assert!(obs.state_hashes.is_none());
        assert!(obs.tty_log.is_empty());
    }

    #[test]
    fn observe_from_boot_maps_fault_and_max_steps() {
        let mem = vec![0u8; 16];
        let fault = observe_from_boot(&mem, BootOutcome::Fault, 50, &[], &[]);
        assert_eq!(fault.outcome, ObservedOutcome::Fault);
        let timeout = observe_from_boot(&mem, BootOutcome::MaxSteps, 100_000, &[], &[]);
        assert_eq!(timeout.outcome, ObservedOutcome::Timeout);
    }

    #[test]
    fn observe_from_boot_maps_pc_reached_to_completed() {
        let mem = vec![0u8; 16];
        let obs = observe_from_boot(&mem, BootOutcome::PcReached(0x10381ce8), 1402388, &[], &[]);
        assert_eq!(obs.outcome, ObservedOutcome::Completed);
        assert_eq!(obs.metadata.steps, Some(1402388));
    }

    #[test]
    fn observe_from_boot_maps_rsx_write_checkpoint_to_completed() {
        let mem = vec![0u8; 16];
        let obs = observe_from_boot(&mem, BootOutcome::RsxWriteCheckpoint, 12_345, &[], &[]);
        assert_eq!(obs.outcome, ObservedOutcome::Completed);
        assert_eq!(obs.metadata.steps, Some(12_345));
    }

    #[test]
    fn observe_from_boot_passes_tty_log_through() {
        let mem = vec![0u8; 16];
        let tty = b"hello world\n";
        let obs = observe_from_boot(&mem, BootOutcome::ProcessExit, 1, &[], tty);
        assert_eq!(obs.tty_log, tty);
    }
}
