//! The boot overrides a run identity carries survive the compare-side
//! records that embed the identity -- a boot summary and a cross-runner
//! summary -- and a state trace written under the old header format is
//! refused by the divergence scanner.

use crate::identity::{BootOverrides, RunIdentity};
use crate::test_support::identity;
use crate::trace_identity;

fn overridden(overrides: BootOverrides) -> RunIdentity {
    RunIdentity {
        overrides,
        ..identity("4.91", "NPAA00001", "base")
    }
}

fn every_override() -> BootOverrides {
    BootOverrides {
        skip_module_start: true,
        force_system_authid: true,
        prx_base: Some(0x3000_0000),
        disable_module_start_hle_stubs: true,
    }
}

#[test]
fn a_boot_summary_keeps_the_overrides_it_was_measured_under() {
    let mut summary = crate::BootSummary::new(
        crate::CheckpointKind::ProcessExit,
        crate::BootOutcome::ProcessExit,
        10,
        cellgov_time::Budget::new(1),
    )
    .unwrap();
    summary.identity = overridden(every_override());
    let text = serde_json::to_string(&summary).unwrap();
    let back: crate::BootSummary = serde_json::from_str(&text).unwrap();
    assert_eq!(back.identity.overrides, every_override());
}

#[test]
fn a_cross_runner_summary_keeps_the_overrides_it_was_written_under() {
    let summary = crate::CrossRunnerSummary {
        convergence: crate::Convergence::Yes,
        byte_parity: crate::ByteParity::Equivalent,
        per_class_bytes: std::collections::BTreeMap::new(),
        unclassified_bytes: 0,
        unclassified_runs: Vec::new(),
        lowest_offset_class: None,
        identity: overridden(every_override()),
        rpcs3_firmware: Some("4.91".to_string()),
    };
    let text = serde_json::to_string(&summary).unwrap();
    let back: crate::CrossRunnerSummary = serde_json::from_str(&text).unwrap();
    assert_eq!(back.identity.overrides, every_override());
}

#[test]
fn a_state_trace_under_the_format_2_header_is_refused_by_its_format() {
    let format_2 = |hash| {
        // Tag, version, and the two u64 fingerprints format 2 carried.
        let mut bytes = vec![cellgov_trace::TraceRecord::RunIdentity {
            format_version: 0,
            firmware: 0,
            game: 0,
            overrides: 0,
        }
        .tag()];
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 16]);
        cellgov_trace::TraceRecord::PpuStateHash {
            step: 0,
            pc: 0x1_0000,
            hash: cellgov_trace::StateHash::new(hash),
        }
        .encode(&mut bytes);
        bytes
    };
    let (a, b) = (format_2(1), format_2(2));
    assert_eq!(trace_identity(&a), None);
    let unsupported = cellgov_trace::DecodeError::UnsupportedFormatVersion(2);
    match crate::diverge(&a, &b) {
        crate::DivergeReport::CorruptTrace {
            common_count: 0,
            a_error: Some(a_error),
            b_error: Some(b_error),
        } => {
            assert_eq!(a_error.source, unsupported);
            assert_eq!(b_error.source, unsupported);
        }
        other => panic!("a format-2 stream was framed at this format's width: {other:?}"),
    }
}
