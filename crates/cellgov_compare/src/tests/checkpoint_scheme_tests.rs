//! The checkpoint scheme a trace stream names beside its PPU scheme.

use crate::{
    diverge, trace_scheme, DivergeReport, TraceSchemes, CHECKPOINT_HASH_SCHEME,
    LEGACY_CHECKPOINT_HASH_SCHEME,
};
use cellgov_ppu::multilinear::SCHEME_ID;
use cellgov_ppu::state::FNV1A_SCHEME_ID;
use cellgov_trace::{
    HashCheckpointKind, StateHash, TraceRecord, TraceWriter, TRACE_FORMAT_VERSION,
};

/// A header, a scheme record with `checkpoint`, then two commits' worth
/// of `PpuStateHash` and `StateHashCheckpoint` records.
///
/// The hash records do not depend on `checkpoint`, so two streams differ
/// only in their scheme record.
fn trace(checkpoint: u64) -> Vec<u8> {
    let mut w = TraceWriter::new();
    w.record_header(&TraceRecord::RunIdentity {
        format_version: TRACE_FORMAT_VERSION,
        firmware: 1,
        game: 2,
        overrides: 0,
    });
    w.record(&TraceRecord::StateHashScheme {
        ppu: SCHEME_ID,
        checkpoint,
    });
    for step in 0..2 {
        w.record(&TraceRecord::PpuStateHash {
            step,
            pc: 0x100 + 4 * step,
            hash: StateHash::new(0xaa + step),
        });
        w.record(&TraceRecord::StateHashCheckpoint {
            kind: HashCheckpointKind::UnitStatus,
            hash: StateHash::new(0xbb + step),
        });
    }
    w.take_bytes()
}

#[test]
fn a_stream_names_its_checkpoint_scheme() {
    assert_eq!(
        trace_scheme(&trace(CHECKPOINT_HASH_SCHEME)),
        TraceSchemes {
            ppu: SCHEME_ID,
            checkpoint: CHECKPOINT_HASH_SCHEME,
        }
    );
}

#[test]
fn an_unstamped_stream_reads_as_the_legacy_checkpoint_scheme() {
    let mut w = TraceWriter::new();
    w.record(&TraceRecord::PpuStateHash {
        step: 0,
        pc: 0x100,
        hash: StateHash::new(1),
    });
    let unstamped = TraceSchemes {
        ppu: FNV1A_SCHEME_ID,
        checkpoint: LEGACY_CHECKPOINT_HASH_SCHEME,
    };
    assert_eq!(trace_scheme(&w.take_bytes()), unstamped);
    assert_eq!(trace_scheme(&[]), unstamped);
    assert_eq!(TraceSchemes::UNSTAMPED, unstamped);
}

#[test]
fn a_checkpoint_scheme_mismatch_leaves_the_ppu_scan_standing() {
    let a = trace(LEGACY_CHECKPOINT_HASH_SCHEME);
    let b = trace(CHECKPOINT_HASH_SCHEME);
    assert_eq!(diverge(&a, &b), DivergeReport::Identical { count: 2 });
    let (sa, sb) = (trace_scheme(&a), trace_scheme(&b));
    assert_eq!(sa.ppu, sb.ppu);
    assert_ne!(sa.checkpoint, sb.checkpoint);
}
