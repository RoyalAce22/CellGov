//! `vrefp`, `vrsqrtefp`, `vexptefp` and `vlogefp` through `execute_vx`,
//! held to the exponent sweeps at the end of the `ppu_vmx_estimate_readout`
//! (positive operands) and `ppu_vmx_estimate_edges` (negative operands)
//! captures, under each NJ mode.

use super::lane_support::pack_u32x4;
use crate::exec::test_support::exec_no_mem;
use crate::exec::ExecuteVerdict;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_NJ;

/// The probes' estimate order.
const OPS: [VxOp; 4] = [VxOp::Vrefp, VxOp::Vrsqrtefp, VxOp::Vexptefp, VxOp::Vlogefp];

/// Every result word of `probe`'s frame.
fn frame_words(probe: &str) -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// The sweep inputs for one estimate: each biased exponent with an empty
/// and then a full fraction, in frame order, under `sign`.
fn sweep_inputs(sign: u32) -> Vec<u32> {
    (0..256u32)
        .flat_map(|e| [sign | e << 23, sign | e << 23 | 0x7F_FFFF])
        .collect()
}

/// Run `op` on four slots under `nj` and return vD's words.
fn run(op: VxOp, nj: bool, b: [u32; 4]) -> u128 {
    let mut s = PpuState::new();
    s.set_vscr(if nj { VSCR_NJ } else { 0 });
    s.set_vr(2, pack_u32x4(b));
    let insn = PpuInstruction::Vx {
        op,
        rc: false,
        vt: 3,
        va: 0,
        vb: 2,
    };
    assert_eq!(
        exec_no_mem(&insn, &mut s),
        ExecuteVerdict::Continue,
        "{op:?}"
    );
    s.vr[3]
}

/// Compare one sweep of `console` words, four slots at a time, and
/// return the slots compared.
fn compare_sweep(op: VxOp, nj: bool, sign: u32, console: &[u32]) -> usize {
    let inputs = sweep_inputs(sign);
    assert_eq!(console.len(), inputs.len());
    for (x, r) in inputs.chunks_exact(4).zip(console.chunks_exact(4)) {
        let x: [u32; 4] = x.try_into().expect("4 slots");
        let r: [u32; 4] = r.try_into().expect("4 slots");
        assert_eq!(run(op, nj, x), pack_u32x4(r), "{op:?} nj={nj} {x:#010x?}");
    }
    inputs.len()
}

#[test]
fn every_positive_sweep_word_matches_the_console_in_both_modes() {
    let words = frame_words("ppu_vmx_estimate_readout");
    assert_eq!(words.len(), 45_056);
    let mut tail = words[45_056 - 4096..].chunks_exact(512);
    let mut compared = 0;
    for nj in [false, true] {
        for op in OPS {
            compared += compare_sweep(op, nj, 0, tail.next().expect("a sweep"));
        }
    }
    assert_eq!(compared, 4096);
}

#[test]
fn every_negative_sweep_word_matches_the_console_in_both_modes() {
    let words = frame_words("ppu_vmx_estimate_edges");
    assert_eq!(words.len(), 2 * 18_432);
    let mut compared = 0;
    for (half, nj) in [false, true].into_iter().enumerate() {
        let end = (half + 1) * 18_432;
        let mut tail = words[end - 2048..end].chunks_exact(512);
        for op in OPS {
            compared += compare_sweep(op, nj, 0x8000_0000, tail.next().expect("a sweep"));
        }
    }
    assert_eq!(compared, 4096);
}
