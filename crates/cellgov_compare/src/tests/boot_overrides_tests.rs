//! The boot overrides a run identity carries: wire form, fingerprint,
//! and the cross-identity reports.

use super::*;
use crate::test_support::identity;

fn overridden(overrides: BootOverrides) -> RunIdentity {
    RunIdentity {
        overrides,
        ..identity("4.91", "NPAA00001", "base")
    }
}

fn skip() -> BootOverrides {
    BootOverrides {
        skip_module_start: true,
        ..BootOverrides::default()
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
fn an_identity_with_no_overrides_serializes_no_overrides_key() {
    let text = serde_json::to_string(&identity("4.91", "NPAA00001", "base")).unwrap();
    assert!(!text.contains("overrides"), "{text}");
}

#[test]
fn an_unset_override_is_left_out_of_the_wire_form() {
    let text = serde_json::to_string(&overridden(skip())).unwrap();
    assert!(
        text.contains(r#""overrides":{"skip_module_start":true}"#),
        "{text}"
    );
}

#[test]
fn every_override_round_trips_through_the_sentinel_line() {
    let id = overridden(every_override());
    let line = id.render_sentinel_line().unwrap();
    assert_eq!(RunIdentity::parse_sentinel_lines(&line).unwrap(), Some(id));
}

#[test]
fn the_override_set_is_spelled_on_the_wire_by_its_field_names() {
    let id = RunIdentity {
        overrides: every_override(),
        ..RunIdentity::default()
    };
    let line = format!(
        r#"{RUN_IDENTITY_SENTINEL} {{"overrides":{{"skip_module_start":true,"force_system_authid":true,"prx_base":805306368,"disable_module_start_hle_stubs":true}}}}"#
    );
    assert_eq!(id.render_sentinel_line().unwrap(), line);
    assert_eq!(RunIdentity::parse_sentinel_lines(&line).unwrap(), Some(id));
}

#[test]
fn an_unknown_override_key_is_refused() {
    let line = |key: &str| format!(r#"{RUN_IDENTITY_SENTINEL} {{"overrides":{{"{key}":true}}}}"#);
    assert_eq!(
        RunIdentity::parse_sentinel_lines(&line("skip_module_start")).unwrap(),
        Some(RunIdentity {
            overrides: skip(),
            ..RunIdentity::default()
        })
    );
    match RunIdentity::parse_sentinel_lines(&line("skip_everything")) {
        Err(SentinelParseError::Malformed { source, .. }) => {
            assert!(source.to_string().contains("skip_everything"), "{source}");
        }
        other => panic!("an unknown override key parsed: {other:?}"),
    }
}

#[test]
fn an_identity_naming_only_overrides_is_not_empty() {
    let id = RunIdentity {
        overrides: skip(),
        ..RunIdentity::default()
    };
    assert!(!id.is_empty());
}

#[test]
fn no_overrides_fingerprint_as_zero() {
    assert_eq!(
        identity("4.91", "NPAA00001", "base").overrides_fingerprint(),
        0
    );
}

#[test]
fn each_override_fingerprints_apart() {
    let one = |overrides| overridden(overrides).overrides_fingerprint();
    let prints = [
        one(skip()),
        one(BootOverrides {
            force_system_authid: true,
            ..BootOverrides::default()
        }),
        one(BootOverrides {
            prx_base: Some(0x3000_0000),
            ..BootOverrides::default()
        }),
        one(BootOverrides {
            prx_base: Some(0x3001_0000),
            ..BootOverrides::default()
        }),
        one(BootOverrides {
            disable_module_start_hle_stubs: true,
            ..BootOverrides::default()
        }),
        one(every_override()),
    ];
    for (i, a) in prints.iter().enumerate() {
        assert_ne!(*a, 0, "override set {i} fingerprints as none");
        for (j, b) in prints.iter().enumerate().skip(i + 1) {
            assert_ne!(a, b, "override sets {i} and {j} fingerprint alike");
        }
    }
}

#[test]
fn the_trace_header_carries_the_override_fingerprint() {
    let id = overridden(every_override());
    match id.trace_header() {
        cellgov_trace::TraceRecord::RunIdentity { overrides, .. } => {
            assert_ne!(overrides, 0, "the header names no override");
            assert_eq!(overrides, id.overrides_fingerprint());
        }
        other => panic!("the header is {other:?}"),
    }
}

#[test]
fn an_override_only_difference_warns() {
    let clean = identity("4.91", "NPAA00001", "base");
    let lines = cross_identity_warning(&clean, "a.json", &overridden(skip()), "b.json");
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("cross-override"), "{lines:?}");
    assert!(
        lines[0].contains("a.json ran no boot overrides"),
        "{lines:?}"
    );
    assert!(
        lines[0].contains("b.json ran boot overrides skip_module_start"),
        "{lines:?}"
    );
    assert!(lines[1].contains("not a regression"), "{lines:?}");
}

/// The identity header a stream written under `id` leads with.
fn header_of(id: &RunIdentity) -> Option<TraceIdentity> {
    let mut writer = cellgov_trace::TraceWriter::new();
    writer.record_header(&id.trace_header());
    trace_identity(writer.bytes())
}

#[test]
fn two_trace_headers_that_disagree_only_on_overrides_warn() {
    let clean = identity("4.91", "NPAA00001", "base");
    let skipped = overridden(skip());
    let lines =
        cross_trace_identity_warning(header_of(&clean), "a.state", header_of(&skipped), "b.state");
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[0].contains("disagree on boot overrides"),
        "only the override set moved: {lines:?}"
    );
    assert!(
        lines[1].starts_with("  a.state: ") && lines[1].ends_with(" overrides=0x0000000000000000"),
        "{lines:?}"
    );
    let skipped_print = format!(" overrides=0x{:016x}", skipped.overrides_fingerprint());
    assert!(
        lines[2].starts_with("  b.state: ") && lines[2].ends_with(&skipped_print),
        "{lines:?}"
    );
}

#[test]
fn two_runs_that_name_only_their_overrides_warn_when_the_sets_differ() {
    let only = |overrides| RunIdentity {
        overrides,
        ..RunIdentity::default()
    };
    let (a, b) = (only(skip()), only(every_override()));
    let json = cross_identity_warning(&a, "a.json", &b, "b.json");
    assert!(
        json.first().is_some_and(|l| l.contains("cross-override")),
        "{json:?}"
    );
    let trace = cross_trace_identity_warning(header_of(&a), "a.state", header_of(&b), "b.state");
    assert!(
        trace
            .first()
            .is_some_and(|l| l.contains("disagree on boot overrides")),
        "{trace:?}"
    );
}

#[test]
fn the_report_names_the_overrides_only_when_the_run_applied_some() {
    let clean = identity("4.91", "NPAA00001", "base");
    assert!(!clean
        .render_lines()
        .iter()
        .any(|l| l.starts_with("override")));
    let lines = overridden(every_override()).render_lines();
    assert_eq!(
        lines.last().map(String::as_str),
        Some(
            "override skip_module_start force_system_authid prx_base=0x30000000 \
             disable_module_start_hle_stubs"
        ),
        "{lines:?}"
    );
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
