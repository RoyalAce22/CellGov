//! BootOutcome mapping into observations and Display/FromStr round-trips.

use super::*;
use crate::identity::RunIdentity;
use crate::runner_cellgov::region::SpaceSnapshots;
use cellgov_core::AddressSpaceId;
use cellgov_mem::GuestMemory;

fn boot_only(size: usize) -> SpaceSnapshots {
    SpaceSnapshots::from([(AddressSpaceId::BOOT, GuestMemory::new(size))])
}

#[test]
fn observe_from_boot_maps_process_exit_to_process_exit() {
    let mem = boot_only(16);
    let obs = observe_from_boot(
        &mem,
        BootOutcome::ProcessExit,
        1000,
        &[],
        &[],
        RunIdentity::default(),
    );
    assert_eq!(obs.outcome, ObservedOutcome::ProcessExit);
    assert_eq!(obs.metadata.runner, "cellgov-boot");
    assert_eq!(obs.metadata.steps, Some(1000));
    assert!(obs.state_hashes.is_none());
    assert!(obs.tty_log.is_empty());
}

#[test]
fn observe_from_boot_maps_fault_and_max_steps() {
    let mem = boot_only(16);
    let fault = observe_from_boot(
        &mem,
        BootOutcome::Fault,
        50,
        &[],
        &[],
        RunIdentity::default(),
    );
    assert_eq!(fault.outcome, ObservedOutcome::Fault);
    let timeout = observe_from_boot(
        &mem,
        BootOutcome::MaxSteps,
        100_000,
        &[],
        &[],
        RunIdentity::default(),
    );
    assert_eq!(timeout.outcome, ObservedOutcome::Timeout);
}

#[test]
fn observe_from_boot_maps_pc_reached_to_completed() {
    let mem = boot_only(16);
    let obs = observe_from_boot(
        &mem,
        BootOutcome::PcReached(0x10381ce8),
        1402388,
        &[],
        &[],
        RunIdentity::default(),
    );
    assert_eq!(obs.outcome, ObservedOutcome::Completed);
    assert_eq!(obs.metadata.steps, Some(1402388));
}

#[test]
fn observe_from_boot_maps_rsx_write_checkpoint_to_completed() {
    let mem = boot_only(16);
    let obs = observe_from_boot(
        &mem,
        BootOutcome::RsxWriteCheckpoint,
        12_345,
        &[],
        &[],
        RunIdentity::default(),
    );
    assert_eq!(obs.outcome, ObservedOutcome::Completed);
    assert_eq!(obs.metadata.steps, Some(12_345));
}

#[test]
fn observe_from_boot_passes_tty_log_through() {
    let mem = boot_only(16);
    let tty = b"hello world\n";
    let obs = observe_from_boot(
        &mem,
        BootOutcome::ProcessExit,
        1,
        &[],
        tty,
        RunIdentity::default(),
    );
    assert_eq!(obs.tty_log, tty);
}

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
