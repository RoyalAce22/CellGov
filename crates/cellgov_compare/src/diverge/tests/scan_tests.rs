//! Trace-stream divergence scanning over the per-step state-hash records: PC, hash, and length
//! differences, and the PPU and SPU streams compared apart.

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
            stream,
            step,
            a_pc,
            b_pc,
            a_hash,
            b_hash,
            field,
        } => {
            assert_eq!(stream, StateStream::Ppu);
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
            stream: StateStream::Ppu,
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
            stream: StateStream::Ppu,
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

/// Byte offset of the `n`th record, walking the scanner's own decoder.
fn record_offset(trace: &[u8], n: usize) -> usize {
    let mut reader = cellgov_trace::TraceReader::new(trace);
    for _ in 0..n {
        reader
            .next()
            .expect("record exists")
            .expect("record decodes");
    }
    reader.position()
}

fn bad_tag(trace: &mut [u8], record: usize) -> usize {
    let offset = record_offset(trace, record);
    trace[offset] = 0xff;
    offset
}

#[test]
fn a_corrupt_record_on_one_side_reports_the_cut_not_a_verdict() {
    let clean = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb), h(2, 0x108, 0xcc)]);
    let mut b = clean.clone();
    let offset = bad_tag(&mut b, 2);
    assert_eq!(
        diverge(&clean, &b),
        DivergeReport::CorruptTrace {
            common_count: 2,
            a_error: None,
            b_error: Some(TraceDecodeError {
                index: 2,
                offset,
                source: cellgov_trace::DecodeError::UnknownTag(0xff),
            }),
        }
    );
}

#[test]
fn a_mid_record_end_reports_truncation_at_the_cut_record() {
    let clean = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb), h(2, 0x108, 0xcc)]);
    let mut a = clean.clone();
    let offset = record_offset(&a, 1);
    a.truncate(offset + 1);
    assert_eq!(
        diverge(&a, &clean),
        DivergeReport::CorruptTrace {
            common_count: 1,
            a_error: Some(TraceDecodeError {
                index: 1,
                offset,
                source: cellgov_trace::DecodeError::Truncated,
            }),
            b_error: None,
        }
    );
}

#[test]
fn a_corrupt_tail_past_the_shorter_side_is_not_a_length_difference() {
    let mut a = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb), h(2, 0x108, 0xcc)]);
    let offset = bad_tag(&mut a, 2);
    let b = encode(&[h(0, 0x100, 0xaa)]);
    assert_eq!(
        diverge(&a, &b),
        DivergeReport::CorruptTrace {
            common_count: 1,
            a_error: Some(TraceDecodeError {
                index: 2,
                offset,
                source: cellgov_trace::DecodeError::UnknownTag(0xff),
            }),
            b_error: None,
        }
    );
}

#[test]
fn both_sides_failing_at_the_same_pull_are_both_named() {
    let clean = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb)]);
    let mut a = clean.clone();
    let mut b = clean.clone();
    let a_offset = bad_tag(&mut a, 1);
    let b_offset = record_offset(&b, 1);
    b.truncate(b_offset + 1);
    match diverge(&a, &b) {
        DivergeReport::CorruptTrace {
            common_count,
            a_error: Some(a_error),
            b_error: Some(b_error),
        } => {
            assert_eq!(common_count, 1);
            assert_eq!(a_error.offset, a_offset);
            assert_eq!(b_error.offset, b_offset);
            assert_eq!(b_error.source, cellgov_trace::DecodeError::Truncated);
        }
        other => panic!("expected both sides corrupt, got {other:?}"),
    }
}

#[test]
fn the_failing_index_counts_every_record_kind() {
    use cellgov_trace::HashCheckpointKind;
    let mut w = TraceWriter::new();
    w.record(&h(0, 0x100, 0xaa));
    w.record(&TraceRecord::StateHashCheckpoint {
        kind: HashCheckpointKind::CommittedMemory,
        hash: StateHash::new(0xdead),
    });
    w.record(&h(1, 0x104, 0xbb));
    let mut a = w.take_bytes();
    let offset = bad_tag(&mut a, 2);
    let b = encode(&[h(0, 0x100, 0xaa), h(1, 0x104, 0xbb)]);
    match diverge(&a, &b) {
        DivergeReport::CorruptTrace {
            common_count: 1,
            a_error: Some(error),
            b_error: None,
        } => {
            assert_eq!(
                error.index, 2,
                "the checkpoint record counts toward the index"
            );
            assert_eq!(error.offset, offset);
        }
        other => panic!("expected side A corrupt after one matched record, got {other:?}"),
    }
}

fn spu(unit: u64, step: u64, pc: u64, hash: u64) -> TraceRecord {
    TraceRecord::SpuStateHash {
        unit: cellgov_event::UnitId::new(unit),
        step,
        pc,
        hash: StateHash::new(hash),
    }
}

#[test]
fn an_spu_only_divergence_names_its_unit_while_the_ppu_matches() {
    let a = encode(&[
        h(0, 0x100, 0xaa),
        spu(3, 0, 0x0, 0x10),
        h(1, 0x104, 0xbb),
        spu(3, 1, 0x4, 0x11),
    ]);
    let b = encode(&[
        h(0, 0x100, 0xaa),
        spu(3, 0, 0x0, 0x10),
        h(1, 0x104, 0xbb),
        spu(3, 1, 0x4, 0x99),
    ]);
    assert_eq!(
        diverge(&a, &b),
        DivergeReport::Differs {
            stream: StateStream::Spu(cellgov_event::UnitId::new(3)),
            step: 1,
            a_pc: 0x4,
            b_pc: 0x4,
            a_hash: 0x11,
            b_hash: 0x99,
            field: DivergeField::Hash,
        }
    );
}

#[test]
fn two_runs_that_interleave_their_units_differently_still_match() {
    let a = encode(&[
        spu(1, 0, 0, 1),
        spu(2, 0, 0, 2),
        spu(1, 1, 4, 3),
        spu(2, 1, 4, 4),
    ]);
    let b = encode(&[
        spu(2, 0, 0, 2),
        spu(2, 1, 4, 4),
        spu(1, 0, 0, 1),
        spu(1, 1, 4, 3),
    ]);
    assert_eq!(diverge(&a, &b), DivergeReport::Identical { count: 4 });
}

#[test]
fn the_disagreement_first_in_side_a_is_reported() {
    // Unit 1 disagrees at its step 1 (record 2 of side A); unit 2 at its
    // step 0 (record 1).
    let a = encode(&[spu(1, 0, 0, 1), spu(2, 0, 0, 2), spu(1, 1, 4, 3)]);
    let b = encode(&[spu(1, 0, 0, 1), spu(2, 0, 0, 7), spu(1, 1, 4, 8)]);
    match diverge(&a, &b) {
        DivergeReport::Differs { stream, step, .. } => {
            assert_eq!(stream, StateStream::Spu(cellgov_event::UnitId::new(2)));
            assert_eq!(step, 0);
        }
        other => panic!("expected a differ, got {other:?}"),
    }
}

#[test]
fn a_shorter_spu_stream_reports_its_unit() {
    let a = encode(&[h(0, 0x100, 0xaa), spu(5, 0, 0, 1), spu(5, 1, 4, 2)]);
    let b = encode(&[h(0, 0x100, 0xaa), spu(5, 0, 0, 1)]);
    assert_eq!(
        diverge(&a, &b),
        DivergeReport::LengthDiffers {
            stream: StateStream::Spu(cellgov_event::UnitId::new(5)),
            common_count: 1,
            a_count: 2,
            b_count: 1,
        }
    );
}

#[test]
fn a_unit_on_one_side_only_is_a_length_difference() {
    let a = encode(&[h(0, 0x100, 0xaa), spu(6, 0, 0, 1)]);
    let b = encode(&[h(0, 0x100, 0xaa)]);
    assert_eq!(
        diverge(&a, &b),
        DivergeReport::LengthDiffers {
            stream: StateStream::Spu(cellgov_event::UnitId::new(6)),
            common_count: 0,
            a_count: 1,
            b_count: 0,
        }
    );
}

#[test]
fn two_spu_schemes_are_a_scheme_mismatch() {
    let with_scheme = |spu_scheme: u64| {
        let mut w = TraceWriter::new();
        w.record(&TraceRecord::StateHashScheme {
            ppu: 1,
            checkpoint: 2,
            spu: spu_scheme,
        });
        w.record(&spu(0, 0, 0, 1));
        w.take_bytes()
    };
    assert_eq!(
        diverge(&with_scheme(7), &with_scheme(8)),
        DivergeReport::SchemeMismatch {
            kind: StateHashKind::Spu,
            a: 7,
            b: 8,
        }
    );
}

#[test]
fn a_stream_displays_as_its_kind_and_unit() {
    assert_eq!(StateStream::Ppu.to_string(), "ppu");
    assert_eq!(
        StateStream::Spu(cellgov_event::UnitId::new(12)).to_string(),
        "spu:12"
    );
}
