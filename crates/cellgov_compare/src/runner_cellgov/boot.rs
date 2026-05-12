//! Long-running boot adapter: converts a finished boot run into the
//! shared `Observation` schema. Separate from the scenario adapter
//! because the testkit runner has no notion of process-exit, hard
//! faults, or HLE-driven termination that a boot reports.

use crate::observation::{Observation, ObservationMetadata, ObservedOutcome};

use super::region::{extract_regions, RegionDescriptor};

/// How a long-running boot terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootOutcome {
    /// Guest reached `sys_process_exit` cleanly.
    ProcessExit,
    /// PPU raised a hard fault (decode error, unimplemented, etc).
    Fault,
    /// Max-step cap reached without termination.
    MaxSteps,
    /// First PPU write into the RSX command region was attempted;
    /// used as a cross-runner checkpoint for titles whose attract-mode
    /// loops never exit on their own.
    RsxWriteCheckpoint,
    /// A step retired with its PC equal to a caller-supplied checkpoint PC.
    PcReached(u64),
    /// Internal time counter overflowed. Distinct from `Fault` so a pair
    /// yielding `TimeOverflow` on one side and `Fault` on the other is
    /// not miscategorized as "both faulted".
    TimeOverflow,
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
