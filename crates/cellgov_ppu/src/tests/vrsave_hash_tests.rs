//! VRSAVE is a hash-covered field: a state that differs only there
//! hashes differently, and the zoom diff names it.

use cellgov_compare::{zoom_lookup, ZoomLookup};
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
        fpscr,
        vrsave,
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
        fpscr,
        vrsave,
    });
    w.take_bytes()
}

#[test]
fn states_that_differ_only_in_vrsave_hash_differently() {
    let a = PpuState::new();
    for v in [1u32, 1 << 31, 0xffff_ffff] {
        let mut b = PpuState::new();
        b.set_vrsave(v);
        assert_ne!(a.state_hash(), b.state_hash(), "vrsave 0x{v:08x}");
        match zoom_lookup(&zoom_bytes(&a), &zoom_bytes(&b), 0) {
            ZoomLookup::Found { diffs, .. } => {
                let names: Vec<&str> = diffs.iter().map(|d| d.field).collect();
                assert_eq!(names, ["vrsave"], "vrsave 0x{v:08x}");
                assert_eq!((diffs[0].a, diffs[0].b), (0, u64::from(v)));
            }
            other => panic!("expected Found, got {other:?}"),
        }
    }
}
