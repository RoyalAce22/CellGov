//! Observation-vs-observation diffing across regions, steps, events, and hashes, with pinned output formats.

use super::*;
use crate::observation::{ObservationMetadata, ObservedEvent, ObservedEventKind, ObservedOutcome};
use cellgov_trace::StateHash;

fn obs(
    outcome: ObservedOutcome,
    regions: Vec<NamedMemoryRegion>,
    runner: &str,
    steps: Option<usize>,
) -> Observation {
    Observation {
        outcome,
        memory_regions: regions,
        events: Vec::new(),
        state_hashes: None,
        metadata: ObservationMetadata {
            runner: runner.to_string(),
            steps,
        },
        tty_log: Vec::new(),
    }
}

fn region(name: &str, addr: u64, data: Vec<u8>) -> NamedMemoryRegion {
    NamedMemoryRegion {
        name: name.to_string(),
        addr,
        data,
    }
}

fn evt(kind: ObservedEventKind, unit: u64, sequence: u32) -> ObservedEvent {
    ObservedEvent {
        kind,
        unit,
        sequence,
    }
}

fn hashes(memory: u64, unit_status: u64, sync: u64) -> ObservedHashes {
    ObservedHashes {
        memory: StateHash::new(memory),
        unit_status: StateHash::new(unit_status),
        sync: StateHash::new(sync),
    }
}

#[test]
fn identical_observations_match() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![1, 2, 3, 4])],
        "cellgov",
        Some(100),
    );
    let b = a.clone();
    let r = compare_observations(&a, &b);
    assert!(!r.has_divergence());
    let out = format_observation_compare_human(&r);
    assert!(out.contains("MATCH outcome=Completed"));
    assert!(out.contains("1 regions (4 bytes) identical"));
    assert!(out.contains("steps Some(100) vs Some(100)"));
}

#[test]
fn outcome_mismatch_is_divergence() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let b = obs(ObservedOutcome::Fault, vec![], "rpcs3", Some(1));
    let r = compare_observations(&a, &b);
    assert!(r.has_divergence());
    let out = format_observation_compare_human(&r);
    assert!(out.starts_with("DIVERGE outcome: cellgov=Completed vs rpcs3=Fault\n"));
}

#[test]
fn outcome_mismatch_with_region_divergence_renders_both_lines() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 4])],
        "cellgov",
        Some(1),
    );
    let mut b_data = vec![0u8; 4];
    b_data[2] = 0xAA;
    let b = obs(
        ObservedOutcome::Fault,
        vec![region("code", 0x10000, b_data)],
        "rpcs3",
        Some(1),
    );
    let r = compare_observations(&a, &b);
    assert!(r.has_divergence());
    let out = format_observation_compare_human(&r);
    assert!(
        out.contains("DIVERGE outcome:"),
        "outcome line should render alongside region line: {out}"
    );
    assert!(
        out.contains("DIVERGE region code: byte at offset 0x2"),
        "region byte line should render after outcome line: {out}"
    );
    assert!(!out.contains("MATCH"));
}

#[test]
fn region_count_mismatch_is_divergence() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("a", 0, vec![0])],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("a", 0, vec![0]), region("b", 1, vec![0])],
        "rpcs3",
        Some(1),
    );
    let r = compare_observations(&a, &b);
    assert!(r.has_divergence());
    let out = format_observation_compare_human(&r);
    assert_eq!(out, "DIVERGE region count: 1 vs 2\n");
}

#[test]
fn identity_mismatch_diverges_with_prior_format() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0])],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("data", 0x20000, vec![0])],
        "rpcs3",
        Some(1),
    );
    let r = compare_observations(&a, &b);
    let out = format_observation_compare_human(&r);
    assert_eq!(
        out,
        "DIVERGE region identity: code@0x10000 vs data@0x20000\n"
    );
}

#[test]
fn length_mismatch_diverges_with_prior_format() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0; 4])],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0; 8])],
        "rpcs3",
        Some(1),
    );
    let r = compare_observations(&a, &b);
    let out = format_observation_compare_human(&r);
    assert_eq!(out, "DIVERGE region code: length 4 vs 8 bytes\n");
}

#[test]
fn single_byte_divergence_renders_byte_format() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0x00; 0x40])],
        "cellgov",
        Some(1),
    );
    let mut b_data = vec![0x00u8; 0x40];
    b_data[0x17] = 0x01;
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, b_data)],
        "rpcs3",
        Some(1),
    );
    let r = compare_observations(&a, &b);
    assert!(r.has_divergence());
    let out = format_observation_compare_human(&r);
    assert_eq!(
        out,
        "DIVERGE region code: byte at offset 0x17 (guest 0x10017) -- 00 vs 01\n"
    );
}

#[test]
fn divergence_at_offset_zero_renders() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0x00; 4])],
        "cellgov",
        Some(1),
    );
    let mut b_data = vec![0x00u8; 4];
    b_data[0] = 0xFF;
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, b_data)],
        "rpcs3",
        Some(1),
    );
    let out = format_observation_compare_human(&compare_observations(&a, &b));
    assert_eq!(
        out,
        "DIVERGE region code: byte at offset 0x0 (guest 0x10000) -- 00 vs ff\n"
    );
}

#[test]
fn divergence_at_last_byte_renders() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0x00; 4])],
        "cellgov",
        Some(1),
    );
    let mut b_data = vec![0x00u8; 4];
    b_data[3] = 0x55;
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, b_data)],
        "rpcs3",
        Some(1),
    );
    let out = format_observation_compare_human(&compare_observations(&a, &b));
    assert_eq!(
        out,
        "DIVERGE region code: byte at offset 0x3 (guest 0x10003) -- 00 vs 55\n"
    );
}

#[test]
fn empty_region_pair_matches() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("empty", 0x10000, vec![])],
        "cellgov",
        Some(1),
    );
    let b = a.clone();
    let r = compare_observations(&a, &b);
    assert!(!r.has_divergence());
    assert_eq!(r.region_compare.matched_regions(), 1);
    assert_eq!(r.region_compare.matched_bytes(), 0);
}

#[test]
fn contiguous_run_coalesces_into_one_byte_divergence() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0x00; 8])],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0x01; 8])],
        "rpcs3",
        Some(1),
    );
    let r = compare_observations(&a, &b);
    let pair = &r.region_compare.pairs[0];
    match pair {
        RegionPairOutcome::ByteDivergence { bytes, .. } => {
            assert_eq!(bytes.len(), 1, "contiguous run coalesces to one entry");
            assert_eq!(bytes[0].offset, 0);
            assert_eq!(bytes[0].length, 8);
            assert_eq!(bytes[0].a_byte, 0x00);
            assert_eq!(bytes[0].b_byte, 0x01);
        }
        other => panic!("expected ByteDivergence, got {other:?}"),
    }
}

#[test]
fn run_renders_with_length_and_first_pair() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("data", 0x80000, vec![0xAA; 4])],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("data", 0x80000, vec![0xBB; 4])],
        "rpcs3",
        Some(1),
    );
    let out = format_observation_compare_human(&compare_observations(&a, &b));
    assert_eq!(
        out,
        "DIVERGE region data: run of 4 bytes at offset 0x0..0x4 (guest 0x80000..0x80004) -- first pair aa vs bb\n"
    );
}

#[test]
fn non_contiguous_divergences_become_separate_runs() {
    let mut a_data = vec![0u8; 16];
    let mut b_data = vec![0u8; 16];
    b_data[1] = 0x10;
    b_data[2] = 0x10;
    b_data[5] = 0x20;
    b_data[10] = 0x30;
    b_data[11] = 0x30;
    b_data[12] = 0x30;
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0x100, a_data.clone())],
        "cellgov",
        Some(1),
    );
    a_data[0] = 0;
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0x100, b_data)],
        "rpcs3",
        Some(1),
    );
    let r = compare_observations(&a, &b);
    let pair = &r.region_compare.pairs[0];
    match pair {
        RegionPairOutcome::ByteDivergence { bytes, .. } => {
            assert_eq!(bytes.len(), 3, "three separate runs");
            assert_eq!(bytes[0].offset, 1);
            assert_eq!(bytes[0].length, 2);
            assert_eq!(bytes[1].offset, 5);
            assert_eq!(bytes[1].length, 1);
            assert_eq!(bytes[2].offset, 10);
            assert_eq!(bytes[2].length, 3);
        }
        other => panic!("expected ByteDivergence, got {other:?}"),
    }
}

#[test]
fn three_runs_in_one_region_render_three_diverge_lines_in_offset_order() {
    let mut b_data = vec![0u8; 16];
    b_data[1] = 0x10;
    b_data[5] = 0x20;
    b_data[10] = 0x30;
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0x100, vec![0u8; 16])],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0x100, b_data)],
        "rpcs3",
        Some(1),
    );
    let out = format_observation_compare_human(&compare_observations(&a, &b));
    let off1 = out.find("offset 0x1 ").expect("first run line missing");
    let off5 = out.find("offset 0x5 ").expect("second run line missing");
    let off10 = out.find("offset 0xa ").expect("third run line missing");
    assert!(off1 < off5 && off5 < off10, "ascending offset order: {out}");
}

#[test]
fn multi_region_divergences_are_all_walked() {
    let mut a_code = vec![0u8; 8];
    let mut b_code = vec![0u8; 8];
    b_code[3] = 0x55;
    let mut a_data = vec![0u8; 4];
    let mut b_data = vec![0u8; 4];
    b_data[0] = 0xCC;
    b_data[1] = 0xDD;
    let a = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, a_code.clone()),
            region("data", 0x80000, a_data.clone()),
        ],
        "cellgov",
        Some(1),
    );
    a_code[0] = 0;
    a_data[0] = 0;
    let b = obs(
        ObservedOutcome::Completed,
        vec![
            region("code", 0x10000, b_code),
            region("data", 0x80000, b_data),
        ],
        "rpcs3",
        Some(1),
    );
    let r = compare_observations(&a, &b);
    assert!(r.has_divergence());
    assert_eq!(r.region_compare.pairs.len(), 2);
    assert!(matches!(
        r.region_compare.pairs[0],
        RegionPairOutcome::ByteDivergence { .. }
    ));
    assert!(matches!(
        r.region_compare.pairs[1],
        RegionPairOutcome::ByteDivergence { .. }
    ));
    let out = format_observation_compare_human(&r);
    assert!(
        out.contains("DIVERGE region code: byte at offset 0x3"),
        "got: {out}"
    );
    assert!(
        out.contains("DIVERGE region data: run of 2 bytes"),
        "got: {out}"
    );
    assert!(!out.contains("MATCH"));
}

#[test]
fn length_mismatch_in_one_region_does_not_block_subsequent_regions() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![
            region("first", 0x10000, vec![0u8; 4]),
            region("second", 0x20000, vec![0xAA; 4]),
        ],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![
            region("first", 0x10000, vec![0u8; 8]),
            region("second", 0x20000, vec![0xBB; 4]),
        ],
        "rpcs3",
        Some(1),
    );
    let r = compare_observations(&a, &b);
    assert_eq!(r.region_compare.pairs.len(), 2);
    assert!(matches!(
        r.region_compare.pairs[0],
        RegionPairOutcome::LengthMismatch { .. }
    ));
    assert!(matches!(
        r.region_compare.pairs[1],
        RegionPairOutcome::ByteDivergence { .. }
    ));
}

#[test]
fn matching_regions_before_diverging_region_are_recorded() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![
            region("first", 0x10000, vec![0xAA; 4]),
            region("second", 0x20000, vec![0xBB; 4]),
        ],
        "cellgov",
        Some(1),
    );
    let b = obs(
        ObservedOutcome::Completed,
        vec![
            region("first", 0x10000, vec![0xAA; 4]),
            region("second", 0x20000, vec![0xCC; 4]),
        ],
        "rpcs3",
        Some(1),
    );
    let r = compare_observations(&a, &b);
    assert_eq!(r.region_compare.pairs.len(), 2);
    assert!(matches!(
        r.region_compare.pairs[0],
        RegionPairOutcome::Match { .. }
    ));
    assert!(matches!(
        r.region_compare.pairs[1],
        RegionPairOutcome::ByteDivergence { .. }
    ));
}

#[test]
fn same_runner_step_mismatch_renders_diverge_without_match_summary() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(100));
    let b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(200));
    let r = compare_observations(&a, &b);
    assert!(r.has_divergence());
    let out = format_observation_compare_human(&r);
    assert!(out.contains("DIVERGE step count: 100 vs 200 within runner 'cellgov'"));
    assert!(!out.contains("MATCH outcome="));
}

#[test]
fn cross_runner_step_mismatch_is_note_not_divergence() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(100));
    let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(200));
    let r = compare_observations(&a, &b);
    assert!(!r.has_divergence());
    assert_eq!(r.cross_runner_step_note(), Some((100, 200)));
    let out = format_observation_compare_human(&r);
    assert!(out.contains("MATCH outcome=Completed"));
    assert!(!out.contains("DIVERGE"));
}

#[test]
fn zero_regions_both_sides_is_vacuous() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
    let r = compare_observations(&a, &b);
    assert!(!r.has_divergence());
    assert!(r.is_vacuous());
}

#[test]
fn nonempty_regions_are_not_vacuous() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0, vec![0])],
        "cellgov",
        Some(1),
    );
    let b = a.clone();
    let r = compare_observations(&a, &b);
    assert!(!r.is_vacuous());
}

#[test]
fn step_compare_no_step_info() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", None);
    let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", None);
    let r = compare_observations(&a, &b);
    assert_eq!(r.step_compare, StepCompare::NoStepInfo);
    assert!(!r.has_divergence());
}

#[test]
fn step_compare_b_missing() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(50));
    let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", None);
    let r = compare_observations(&a, &b);
    assert_eq!(
        r.step_compare,
        StepCompare::OneMissing {
            a: Some(50),
            b: None
        }
    );
    assert!(!r.has_divergence());
}

#[test]
fn step_compare_a_missing() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", None);
    let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(50));
    let r = compare_observations(&a, &b);
    assert_eq!(
        r.step_compare,
        StepCompare::OneMissing {
            a: None,
            b: Some(50)
        }
    );
    assert!(!r.has_divergence());
}

#[test]
fn step_compare_equal_steps() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(42));
    let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(42));
    let r = compare_observations(&a, &b);
    assert_eq!(r.step_compare, StepCompare::Equal { steps: 42 });
}

#[test]
fn events_equal_when_sequences_match() {
    let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
    a.events = vec![
        evt(ObservedEventKind::MailboxSend, 1, 0),
        evt(ObservedEventKind::DmaComplete, 2, 1),
    ];
    b.events = a.events.clone();
    let r = compare_observations(&a, &b);
    assert!(!r.has_divergence());
    assert_eq!(r.event_compare, EventCompare::Equal { count: 2 });
}

#[test]
fn events_length_differs_is_divergence() {
    let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
    a.events = vec![evt(ObservedEventKind::MailboxSend, 1, 0)];
    b.events = vec![
        evt(ObservedEventKind::MailboxSend, 1, 0),
        evt(ObservedEventKind::UnitWake, 1, 1),
    ];
    let r = compare_observations(&a, &b);
    assert!(r.has_divergence());
    assert_eq!(r.event_compare, EventCompare::LengthMismatch { a: 1, b: 2 });
    let out = format_observation_compare_human(&r);
    assert!(out.contains("DIVERGE event count: 1 vs 2"), "got: {out}");
}

#[test]
fn events_differ_at_index_is_divergence() {
    let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
    a.events = vec![
        evt(ObservedEventKind::MailboxSend, 1, 0),
        evt(ObservedEventKind::MailboxReceive, 2, 1),
        evt(ObservedEventKind::DmaComplete, 3, 2),
    ];
    b.events = vec![
        evt(ObservedEventKind::MailboxSend, 1, 0),
        evt(ObservedEventKind::MailboxReceive, 2, 1),
        evt(ObservedEventKind::UnitBlock, 3, 2),
    ];
    let r = compare_observations(&a, &b);
    assert!(r.has_divergence());
    assert!(matches!(
        r.event_compare,
        EventCompare::FirstIndexDiffers { index: 2, .. }
    ));
    let out = format_observation_compare_human(&r);
    assert!(out.contains("DIVERGE event at index 2"), "got: {out}");
}

#[test]
fn state_hash_equal_when_present_and_matching() {
    let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let mut b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    a.state_hashes = Some(hashes(1, 2, 3));
    b.state_hashes = a.state_hashes;
    let r = compare_observations(&a, &b);
    assert!(!r.has_divergence());
    assert_eq!(r.state_hash_compare, StateHashCompare::Equal);
}

#[test]
fn state_hash_same_runner_mismatch_is_divergence() {
    let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let mut b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    a.state_hashes = Some(hashes(1, 2, 3));
    b.state_hashes = Some(hashes(1, 2, 4));
    let r = compare_observations(&a, &b);
    assert!(r.has_divergence());
    assert!(matches!(
        r.state_hash_compare,
        StateHashCompare::SameRunnerMismatch { .. }
    ));
    let out = format_observation_compare_human(&r);
    assert!(
        out.contains("DIVERGE state hashes within runner 'cellgov'"),
        "got: {out}"
    );
}

#[test]
fn state_hash_cross_runner_mismatch_is_note() {
    let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
    a.state_hashes = Some(hashes(1, 2, 3));
    b.state_hashes = Some(hashes(9, 9, 9));
    let r = compare_observations(&a, &b);
    // Different runners: not a divergence (state-hash shape is
    // CellGov-defined; RPCS3 would normally set None).
    assert!(!r.has_divergence());
    assert!(matches!(
        r.state_hash_compare,
        StateHashCompare::CrossRunnerNote { .. }
    ));
}

#[test]
fn state_hash_one_missing_is_not_divergence() {
    let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
    a.state_hashes = Some(hashes(1, 2, 3));
    let r = compare_observations(&a, &b);
    assert!(!r.has_divergence());
    assert!(matches!(
        r.state_hash_compare,
        StateHashCompare::OneMissing {
            a_present: true,
            b_present: false,
        },
    ));
}

#[test]
fn tty_log_differences_do_not_diverge() {
    let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
    a.tty_log = b"hello\n".to_vec();
    b.tty_log = b"hello\r\n".to_vec();
    let r = compare_observations(&a, &b);
    assert!(!r.has_divergence());
}

#[test]
fn json_round_trip_preserves_structure() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, vec![0u8; 4])],
        "cellgov",
        Some(100),
    );
    let mut b_data = vec![0u8; 4];
    b_data[1] = 0xFF;
    b_data[2] = 0xFE;
    let b = obs(
        ObservedOutcome::Completed,
        vec![region("code", 0x10000, b_data)],
        "rpcs3",
        Some(200),
    );
    let r = compare_observations(&a, &b);
    let json = format_observation_compare_json(&r).unwrap();
    let parsed: ObservationCompareResult = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, r);
    assert!(json.contains("\"a_runner\": \"cellgov\""));
    assert!(json.contains("\"b_runner\": \"rpcs3\""));
    assert!(json.contains("\"length\": 2"));
    assert!(json.contains("\"a_byte\": 0"));
    assert!(json.contains("\"b_byte\": 255"));
    assert!(json.contains("\"event_compare\""));
    assert!(json.contains("\"state_hash_compare\""));
}

#[test]
fn json_is_pretty_printed_for_human_inspection() {
    let a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let b = a.clone();
    let json = format_observation_compare_json(&compare_observations(&a, &b)).unwrap();
    assert!(
        json.contains('\n'),
        "pretty-printed JSON must include newlines"
    );
    assert!(
        json.contains("  \""),
        "pretty-printed JSON must indent fields"
    );
}

#[test]
fn match_line_byte_format_is_pinned() {
    let a = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0x10000, vec![0u8; 8])],
        "cellgov",
        Some(7),
    );
    let b = a.clone();
    let out = format_observation_compare_human(&compare_observations(&a, &b));
    assert_eq!(
        out,
        "MATCH outcome=Completed, 1 regions (8 bytes) identical, 0 events, no state hashes, steps Some(7) vs Some(7)\n"
    );
}

#[test]
fn match_line_carries_event_count_when_events_are_present() {
    let mut a = obs(
        ObservedOutcome::Completed,
        vec![region("r", 0x10000, vec![0u8; 4])],
        "cellgov",
        Some(7),
    );
    a.events = vec![
        evt(ObservedEventKind::MailboxSend, 1, 0),
        evt(ObservedEventKind::DmaComplete, 2, 1),
        evt(ObservedEventKind::UnitWake, 1, 2),
    ];
    let b = a.clone();
    let out = format_observation_compare_human(&compare_observations(&a, &b));
    assert!(out.contains("3 events"), "got: {out}");
}

#[test]
fn match_line_labels_state_hashes_equal_when_both_present_and_matching() {
    let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let mut b = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    a.state_hashes = Some(hashes(1, 2, 3));
    b.state_hashes = a.state_hashes;
    let out = format_observation_compare_human(&compare_observations(&a, &b));
    assert!(out.contains("state hashes equal"), "got: {out}");
}

#[test]
fn match_line_labels_state_hashes_one_sided_when_only_one_runner_has_them() {
    let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
    a.state_hashes = Some(hashes(1, 2, 3));
    let out = format_observation_compare_human(&compare_observations(&a, &b));
    assert!(out.contains("state hashes one-sided"), "got: {out}");
}

#[test]
fn match_line_labels_state_hashes_cross_runner_when_both_present_and_differ() {
    let mut a = obs(ObservedOutcome::Completed, vec![], "cellgov", Some(1));
    let mut b = obs(ObservedOutcome::Completed, vec![], "rpcs3", Some(1));
    a.state_hashes = Some(hashes(1, 2, 3));
    b.state_hashes = Some(hashes(9, 9, 9));
    let out = format_observation_compare_human(&compare_observations(&a, &b));
    assert!(
        out.contains("state hashes differ (cross-runner)"),
        "got: {out}"
    );
}

/// Exhaustive label fn for `RegionPairOutcome`.
fn region_pair_kind(v: &RegionPairOutcome) -> &'static str {
    match v {
        RegionPairOutcome::Match { .. } => "match",
        RegionPairOutcome::IdentityMismatch { .. } => "identity_mismatch",
        RegionPairOutcome::LengthMismatch { .. } => "length_mismatch",
        RegionPairOutcome::ByteDivergence { .. } => "byte_divergence",
    }
}

#[test]
fn region_pair_outcome_serde_round_trips_per_variant() {
    let variants: Vec<RegionPairOutcome> = vec![
        RegionPairOutcome::Match {
            name: "code".into(),
            addr: 0x10000,
            length: 4,
        },
        RegionPairOutcome::IdentityMismatch {
            a_name: "code".into(),
            a_addr: 0x10000,
            b_name: "data".into(),
            b_addr: 0x20000,
        },
        RegionPairOutcome::LengthMismatch {
            name: "code".into(),
            a_length: 4,
            b_length: 8,
        },
        RegionPairOutcome::ByteDivergence {
            name: "code".into(),
            addr: 0x10000,
            length: 4,
            bytes: vec![ByteDivergence {
                offset: 0,
                length: 1,
                a_byte: 0xAA,
                b_byte: 0xBB,
            }],
        },
    ];
    let kinds: std::collections::BTreeSet<&'static str> =
        variants.iter().map(region_pair_kind).collect();
    assert_eq!(
        kinds.len(),
        variants.len(),
        "hand-built variant list contains duplicates per kind label",
    );
    for v in &variants {
        let json = serde_json::to_string(v).expect("serialize");
        let back: RegionPairOutcome = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*v, back, "round-trip mismatch for {v:?} via {json}");
        assert!(
            json.contains(&format!(r#""kind":"{}""#, region_pair_kind(v))),
            "tagged JSON missing kind discriminator for {v:?}: {json}",
        );
    }
}

/// Exhaustive label fn for `StepCompare`.
fn step_compare_kind(v: &StepCompare) -> &'static str {
    match v {
        StepCompare::NoStepInfo => "no_step_info",
        StepCompare::Equal { .. } => "equal",
        StepCompare::SameRunnerMismatch { .. } => "same_runner_mismatch",
        StepCompare::CrossRunnerNote { .. } => "cross_runner_note",
        StepCompare::OneMissing { .. } => "one_missing",
    }
}

#[test]
fn step_compare_serde_round_trips_per_variant() {
    let variants: Vec<StepCompare> = vec![
        StepCompare::NoStepInfo,
        StepCompare::Equal { steps: 100 },
        StepCompare::SameRunnerMismatch { a: 100, b: 101 },
        StepCompare::CrossRunnerNote { a: 100, b: 200 },
        StepCompare::OneMissing {
            a: Some(100),
            b: None,
        },
    ];
    let kinds: std::collections::BTreeSet<&'static str> =
        variants.iter().map(step_compare_kind).collect();
    assert_eq!(kinds.len(), variants.len());
    for v in &variants {
        let json = serde_json::to_string(v).expect("serialize");
        let back: StepCompare = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*v, back, "round-trip mismatch for {v:?} via {json}");
        assert!(
            json.contains(&format!(r#""kind":"{}""#, step_compare_kind(v))),
            "tagged JSON missing kind discriminator for {v:?}: {json}",
        );
    }
}

/// Exhaustive label fn for `EventCompare`.
fn event_compare_kind(v: &EventCompare) -> &'static str {
    match v {
        EventCompare::Equal { .. } => "equal",
        EventCompare::LengthMismatch { .. } => "length_mismatch",
        EventCompare::FirstIndexDiffers { .. } => "first_index_differs",
    }
}

#[test]
fn event_compare_serde_round_trips_per_variant() {
    let variants: Vec<EventCompare> = vec![
        EventCompare::Equal { count: 5 },
        EventCompare::LengthMismatch { a: 3, b: 5 },
        EventCompare::FirstIndexDiffers {
            index: 2,
            a: evt(ObservedEventKind::MailboxSend, 1, 2),
            b: evt(ObservedEventKind::MailboxReceive, 1, 2),
        },
    ];
    let kinds: std::collections::BTreeSet<&'static str> =
        variants.iter().map(event_compare_kind).collect();
    assert_eq!(kinds.len(), variants.len());
    for v in &variants {
        let json = serde_json::to_string(v).expect("serialize");
        let back: EventCompare = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*v, back, "round-trip mismatch for {v:?} via {json}");
        assert!(
            json.contains(&format!(r#""kind":"{}""#, event_compare_kind(v))),
            "tagged JSON missing kind discriminator for {v:?}: {json}",
        );
    }
}

/// Exhaustive label fn for `StateHashCompare`.
fn state_hash_compare_kind(v: &StateHashCompare) -> &'static str {
    match v {
        StateHashCompare::NoHashInfo => "no_hash_info",
        StateHashCompare::Equal => "equal",
        StateHashCompare::OneMissing { .. } => "one_missing",
        StateHashCompare::SameRunnerMismatch { .. } => "same_runner_mismatch",
        StateHashCompare::CrossRunnerNote { .. } => "cross_runner_note",
    }
}

#[test]
fn state_hash_compare_serde_round_trips_per_variant() {
    let variants: Vec<StateHashCompare> = vec![
        StateHashCompare::NoHashInfo,
        StateHashCompare::Equal,
        StateHashCompare::OneMissing {
            a_present: true,
            b_present: false,
        },
        StateHashCompare::SameRunnerMismatch {
            a: hashes(1, 2, 3),
            b: hashes(4, 5, 6),
        },
        StateHashCompare::CrossRunnerNote {
            a: hashes(1, 2, 3),
            b: hashes(4, 5, 6),
        },
    ];
    let kinds: std::collections::BTreeSet<&'static str> =
        variants.iter().map(state_hash_compare_kind).collect();
    assert_eq!(kinds.len(), variants.len());
    for v in &variants {
        let json = serde_json::to_string(v).expect("serialize");
        let back: StateHashCompare = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*v, back, "round-trip mismatch for {v:?} via {json}");
        assert!(
            json.contains(&format!(r#""kind":"{}""#, state_hash_compare_kind(v))),
            "tagged JSON missing kind discriminator for {v:?}: {json}",
        );
    }
}
