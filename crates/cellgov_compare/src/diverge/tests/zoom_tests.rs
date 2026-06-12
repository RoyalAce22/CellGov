//! Per-step register diffing of full PPU state records, including missing-step reporting.

use super::*;
use cellgov_trace::{TraceRecord, TraceWriter};

/// Trip-wire: `GPR_FIELD_NAMES[i]` must equal `format!("gpr{i}")`
/// for `i in 0..32`. A typo (e.g. `"grp19"`) would silently
/// mislabel a divergence field in the `RegDiff` report stream.
#[test]
fn gpr_field_names_match_index() {
    for (i, name) in GPR_FIELD_NAMES.iter().enumerate() {
        assert_eq!(*name, format!("gpr{i}"), "GPR_FIELD_NAMES[{i}] mismatch");
    }
}

fn encode(records: &[TraceRecord]) -> Vec<u8> {
    let mut w = TraceWriter::new();
    for r in records {
        w.record(r);
    }
    w.take_bytes()
}

fn full(step: u64, pc: u64, gpr: [u64; 32]) -> TraceRecord {
    TraceRecord::PpuStateFull {
        step,
        pc,
        gpr,
        lr: 0,
        ctr: 0,
        xer: 0,
        cr: 0,
    }
}

#[test]
fn zoom_lookup_finds_step_and_lists_diffs() {
    let mut a_gpr = [0u64; 32];
    a_gpr[3] = 7;
    a_gpr[4] = 11;
    let mut b_gpr = [0u64; 32];
    b_gpr[3] = 9; // different from a
    b_gpr[4] = 11; // same
    let a = encode(&[full(5, 0x100, a_gpr)]);
    let b = encode(&[full(5, 0x100, b_gpr)]);
    match zoom_lookup(&a, &b, 5) {
        ZoomLookup::Found {
            step,
            a_pc,
            b_pc,
            diffs,
        } => {
            assert_eq!(step, 5);
            assert_eq!(a_pc, 0x100);
            assert_eq!(b_pc, 0x100);
            assert_eq!(diffs.len(), 1);
            assert_eq!(diffs[0].field, "gpr3");
            assert_eq!(diffs[0].a, 7);
            assert_eq!(diffs[0].b, 9);
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn zoom_lookup_with_identical_full_states_reports_empty_diffs() {
    let gpr = [0u64; 32];
    let a = encode(&[full(5, 0x100, gpr)]);
    let b = encode(&[full(5, 0x100, gpr)]);
    match zoom_lookup(&a, &b, 5) {
        ZoomLookup::Found { diffs, .. } => assert!(diffs.is_empty()),
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn zoom_lookup_missing_step_on_a_side_reports_missing() {
    let a = encode(&[full(0, 0x100, [0u64; 32])]);
    let b = encode(&[full(0, 0x100, [0u64; 32]), full(5, 0x200, [0u64; 32])]);
    let r = zoom_lookup(&a, &b, 5);
    assert!(matches!(
        r,
        ZoomLookup::MissingStep {
            step: 5,
            a_missing: true,
            b_missing: false
        }
    ));
}

#[test]
fn zoom_lookup_missing_step_on_both_sides_reports_both_missing() {
    let a = encode(&[full(0, 0x100, [0u64; 32])]);
    let b = encode(&[full(0, 0x100, [0u64; 32])]);
    let r = zoom_lookup(&a, &b, 5);
    assert!(matches!(
        r,
        ZoomLookup::MissingStep {
            step: 5,
            a_missing: true,
            b_missing: true
        }
    ));
}

#[test]
fn zoom_lookup_diffs_include_lr_ctr_xer_cr() {
    let gpr = [0u64; 32];
    let a = encode(&[TraceRecord::PpuStateFull {
        step: 0,
        pc: 0,
        gpr,
        lr: 1,
        ctr: 2,
        xer: 3,
        cr: 4,
    }]);
    let b = encode(&[TraceRecord::PpuStateFull {
        step: 0,
        pc: 0,
        gpr,
        lr: 100,
        ctr: 200,
        xer: 300,
        cr: 400,
    }]);
    match zoom_lookup(&a, &b, 0) {
        ZoomLookup::Found { diffs, .. } => {
            let names: Vec<&str> = diffs.iter().map(|d| d.field).collect();
            assert_eq!(names, vec!["lr", "ctr", "xer", "cr"]);
        }
        other => panic!("expected Found, got {other:?}"),
    }
}
