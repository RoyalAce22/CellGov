//! The enum and the label table index each other; nothing but a test
//! keeps them aligned.

use super::*;

const ALL: [Phase; 8] = [
    Phase::Reading,
    Phase::Staging,
    Phase::Proving,
    Phase::Clearing,
    Phase::Committing,
    Phase::Hashing,
    Phase::ClearingStaging,
    Phase::InstallingFirmware,
];

/// The label each variant must carry.
///
/// Exhaustive by construction: a variant added to [`Phase`] fails to
/// compile until it is answered here.
fn expected_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Reading => "reading",
        Phase::Staging => "staging",
        Phase::Proving => "verifying decrypt",
        Phase::Clearing => "clearing old install",
        Phase::Committing => "committing",
        Phase::Hashing => "hashing source",
        Phase::ClearingStaging => "clearing staging",
        Phase::InstallingFirmware => "installing shipped firmware",
    }
}

#[test]
fn every_phase_indexes_its_own_label() {
    assert_eq!(INSTALL_TASK.phases.len(), ALL.len());
    for phase in ALL {
        assert_eq!(
            INSTALL_TASK.phases.get(phase.code() as usize).copied(),
            Some(expected_label(phase)),
            "{phase:?}"
        );
    }
}

#[test]
fn no_label_is_left_unclaimed_by_a_phase() {
    let claimed: std::collections::BTreeSet<usize> =
        ALL.iter().map(|p| p.code() as usize).collect();
    for (i, label) in INSTALL_TASK.phases.iter().enumerate() {
        assert!(
            claimed.contains(&i),
            "no phase claims phases[{i}] ({label})"
        );
    }
}

#[test]
fn the_measured_phase_is_the_one_with_a_byte_denominator() {
    assert_eq!(INSTALL_TASK.measured, Phase::Staging.code());
    assert_eq!(INSTALL_TASK.unit, Unit::Bytes);
    assert!(
        (INSTALL_TASK.measured as usize) < INSTALL_TASK.phases.len(),
        "the measured phase must name a label"
    );
}

const ALL_FIRMWARE: [FirmwarePhase; 8] = [
    FirmwarePhase::Reading,
    FirmwarePhase::ValidatingHmac,
    FirmwarePhase::ClearingStaging,
    FirmwarePhase::Extracting,
    FirmwarePhase::BuildingManifest,
    FirmwarePhase::Clearing,
    FirmwarePhase::Committing,
    FirmwarePhase::UnpackingKernel,
];

/// The label each variant must carry.
///
/// Exhaustive by construction: a variant added to [`FirmwarePhase`]
/// fails to compile until it is answered here.
fn expected_firmware_label(phase: FirmwarePhase) -> &'static str {
    match phase {
        FirmwarePhase::Reading => "reading PUP",
        FirmwarePhase::ValidatingHmac => "validating HMAC",
        FirmwarePhase::ClearingStaging => "clearing staging",
        FirmwarePhase::Extracting => "decrypting packages",
        FirmwarePhase::BuildingManifest => "building manifest",
        FirmwarePhase::Clearing => "clearing old install",
        FirmwarePhase::Committing => "committing",
        FirmwarePhase::UnpackingKernel => "unpacking kernel",
    }
}

#[test]
fn every_firmware_phase_indexes_its_own_label() {
    assert_eq!(FIRMWARE_TASK.phases.len(), ALL_FIRMWARE.len());
    for phase in ALL_FIRMWARE {
        assert_eq!(
            FIRMWARE_TASK.phases.get(phase.code() as usize).copied(),
            Some(expected_firmware_label(phase)),
            "{phase:?}"
        );
    }
}

#[test]
fn no_firmware_label_is_left_unclaimed_by_a_phase() {
    let claimed: std::collections::BTreeSet<usize> =
        ALL_FIRMWARE.iter().map(|p| p.code() as usize).collect();
    for (i, label) in FIRMWARE_TASK.phases.iter().enumerate() {
        assert!(
            claimed.contains(&i),
            "no phase claims phases[{i}] ({label})"
        );
    }
}

/// The caller that starts the bar answers
/// [`crate::sce::section_trace_enabled`] once per run.
#[test]
fn neither_install_task_declares_itself_a_streaming_writer() {
    for task in [&INSTALL_TASK, &FIRMWARE_TASK] {
        assert!(!task.streaming, "{}", task.tag);
    }
}

#[test]
fn the_measured_firmware_phase_is_the_one_the_package_loop_runs_under() {
    assert_eq!(FIRMWARE_TASK.measured, FirmwarePhase::Extracting.code());
    assert_eq!(FIRMWARE_TASK.unit, Unit::Bytes);
    assert!(
        (FIRMWARE_TASK.measured as usize) < FIRMWARE_TASK.phases.len(),
        "the measured phase must name a label"
    );
}
