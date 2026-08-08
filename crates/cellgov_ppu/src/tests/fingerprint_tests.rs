//! Coverage lock between `state_hash` and the zoom diff: every field
//! the hash folds must be nameable by `zoom_lookup`, so a per-step
//! hash divergence can never surface as an empty register diff.

use cellgov_compare::{zoom_lookup, ZoomLookup};
use cellgov_sync::ReservedLine;
use cellgov_trace::{TraceRecord, TraceWriter};

use super::*;

fn zoom_bytes(state: &PpuState) -> Vec<u8> {
    let cellgov_exec::PpuFingerprint {
        gpr,
        lr,
        ctr,
        xer,
        cr,
        reservation_line,
    } = state.fingerprint();
    let mut w = TraceWriter::new();
    w.record(&TraceRecord::PpuStateFull {
        step: 0,
        pc: 0,
        gpr,
        lr,
        ctr,
        xer,
        cr,
        reservation_line,
    });
    w.take_bytes()
}

/// Two states differing in one hash-covered field must both move the
/// hash and name exactly that field in the zoom diff.
fn assert_hash_and_diff_agree(a: &PpuState, b: &PpuState, expected: &str) {
    assert_ne!(
        a.state_hash(),
        b.state_hash(),
        "{expected}: mutation must move state_hash"
    );
    match zoom_lookup(&zoom_bytes(a), &zoom_bytes(b), 0) {
        ZoomLookup::Found { diffs, .. } => {
            let names: Vec<&str> = diffs.iter().map(|d| d.field).collect();
            assert_eq!(
                names,
                vec![expected],
                "hash-covered field must be nameable in the zoom diff"
            );
        }
        other => panic!("expected Found, got {other:?}"),
    }
}

#[test]
fn gpr_divergence_is_hash_visible_and_diff_nameable() {
    let a = PpuState::new();
    let mut b = PpuState::new();
    b.set_gpr(0, 1);
    assert_hash_and_diff_agree(&a, &b, "gpr0");
    let mut b = PpuState::new();
    b.set_gpr(31, 1);
    assert_hash_and_diff_agree(&a, &b, "gpr31");
}

#[test]
fn lr_divergence_is_hash_visible_and_diff_nameable() {
    let a = PpuState::new();
    let mut b = PpuState::new();
    b.set_lr(1);
    assert_hash_and_diff_agree(&a, &b, "lr");
}

#[test]
fn ctr_divergence_is_hash_visible_and_diff_nameable() {
    let a = PpuState::new();
    let mut b = PpuState::new();
    b.set_ctr(1);
    assert_hash_and_diff_agree(&a, &b, "ctr");
}

#[test]
fn xer_divergence_is_hash_visible_and_diff_nameable() {
    let a = PpuState::new();
    let mut b = PpuState::new();
    b.set_xer(1 << 29);
    assert_hash_and_diff_agree(&a, &b, "xer");
}

#[test]
fn cr_divergence_is_hash_visible_and_diff_nameable() {
    let a = PpuState::new();
    let mut b = PpuState::new();
    b.set_cr(0b0100 << 28);
    assert_hash_and_diff_agree(&a, &b, "cr");
}

#[test]
fn reservation_held_divergence_is_hash_visible_and_diff_nameable() {
    let a = PpuState::new();
    let mut b = PpuState::new();
    b.set_reservation(Some(ReservedLine::containing(0x1000)));
    assert_hash_and_diff_agree(&a, &b, "resv_held");
}

#[test]
fn reservation_line_divergence_is_hash_visible_and_diff_nameable() {
    let mut a = PpuState::new();
    a.set_reservation(Some(ReservedLine::containing(0x1000)));
    let mut b = PpuState::new();
    b.set_reservation(Some(ReservedLine::containing(0x2000)));
    assert_hash_and_diff_agree(&a, &b, "resv_line");
}

#[test]
fn equal_fingerprints_imply_equal_hashes() {
    let mut a = PpuState::new();
    let mut b = PpuState::new();
    a.set_gpr(3, 7);
    b.set_gpr(3, 7);
    a.set_reservation(Some(ReservedLine::containing(0x1000)));
    b.set_reservation(Some(ReservedLine::containing(0x1000)));
    // Hash-excluded state may differ freely.
    a.pc = 0x100;
    b.pc = 0x200;
    a.tb = 5;
    b.tb = 9;
    assert_eq!(a.fingerprint(), b.fingerprint());
    assert_eq!(a.state_hash(), b.state_hash());
}
