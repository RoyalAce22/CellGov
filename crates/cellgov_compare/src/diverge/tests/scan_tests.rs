//! Trace-stream divergence scanning over PPU state-hash records: PC, hash, and length differences.

use super::*;
use cellgov_trace::{StateHash, TraceRecord, TraceWriter};

fn encode(records: &[TraceRecord]) -> Vec<u8> {
    let mut w = TraceWriter::new();
    for r in records {
        w.record(r);
    }
    w.take_bytes()
}

fn h(step: u64, pc: u64, hash: u64) -> TraceRecord {
    TraceRecord::PpuStateHash {
        step,
        pc,
        hash: StateHash::new(hash),
    }
}

#[test]
fn identical_streams_report_identical() {
    let stream = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb), h(2, 0x108, 0xcc)]);
    let r = diverge(&stream, &stream);
    assert_eq!(r, DivergeReport::Identical { count: 3 });
}

#[test]
fn empty_streams_report_identical_zero() {
    assert_eq!(diverge(&[], &[]), DivergeReport::Identical { count: 0 });
}

#[test]
fn pc_difference_at_step_2_localizes_to_step_2() {
    let a = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb), h(2, 0x108, 0xcc)]);
    let b = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb), h(2, 0x10c, 0xcc)]);
    match diverge(&a, &b) {
        DivergeReport::Differs {
            step,
            a_pc,
            b_pc,
            field,
            ..
        } => {
            assert_eq!(step, 2);
            assert_eq!(a_pc, 0x108);
            assert_eq!(b_pc, 0x10c);
            assert_eq!(field, DivergeField::Pc);
        }
        other => panic!("expected PC differ, got {other:?}"),
    }
}

#[test]
fn hash_difference_at_same_pc_reports_field_hash() {
    let a = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb)]);
    let b = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xff)]);
    match diverge(&a, &b) {
        DivergeReport::Differs {
            step,
            a_pc,
            b_pc,
            a_hash,
            b_hash,
            field,
        } => {
            assert_eq!(step, 1);
            assert_eq!(a_pc, b_pc);
            assert_eq!(a_hash, 0xbb);
            assert_eq!(b_hash, 0xff);
            assert_eq!(field, DivergeField::Hash);
        }
        other => panic!("expected hash differ, got {other:?}"),
    }
}

#[test]
fn pc_check_runs_before_hash_check() {
    let a = encode(&[h(0, 0x100, 0xaa)]);
    let b = encode(&[h(0, 0x200, 0xbb)]);
    match diverge(&a, &b) {
        DivergeReport::Differs { field, .. } => assert_eq!(field, DivergeField::Pc),
        other => panic!("expected differ, got {other:?}"),
    }
}

#[test]
fn shorter_a_reports_length_mismatch() {
    let a = encode(&[h(0, 0x100, 0xaa)]);
    let b = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb)]);
    let r = diverge(&a, &b);
    assert_eq!(
        r,
        DivergeReport::LengthDiffers {
            common_count: 1,
            a_count: 1,
            b_count: 2,
        }
    );
}

#[test]
fn shorter_b_reports_length_mismatch() {
    let a = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb)]);
    let b = encode(&[h(0, 0x100, 0xaa)]);
    let r = diverge(&a, &b);
    assert_eq!(
        r,
        DivergeReport::LengthDiffers {
            common_count: 1,
            a_count: 2,
            b_count: 1,
        }
    );
}

#[test]
fn non_state_hash_records_are_ignored() {
    use cellgov_trace::HashCheckpointKind;
    let mut w = TraceWriter::new();
    w.record(&h(0, 0x100, 0xaa));
    w.record(&TraceRecord::StateHashCheckpoint {
        kind: HashCheckpointKind::CommittedMemory,
        hash: StateHash::new(0xdead),
    });
    w.record(&h(1, 0x104, 0xbb));
    let a = w.take_bytes();
    let b = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb)]);
    assert_eq!(diverge(&a, &b), DivergeReport::Identical { count: 2 });
}
