//! The lanes `lvebx`, `lvehx` and `lvewx` leave in vD, held to the
//! `ppu_lve_lanes` capture from the reference console.
//!
//! [AltiVec-PEM p:4-24 s:4.2.3] The lanes an element load does not name
//! are boundedly undefined. The probe loads from a line of bytes
//! 0x10..0x1F at every offset, with vD preset to bytes 0xE0..0xEF through
//! a register copy and through an `lvx` just before the load. Each case
//! runs twice. CellGov runs the same load over the same preset.

use super::*;

const LINE: u64 = 0x1000;
const OPS: usize = 3;
const OFFSETS: usize = 16;
const VARIANTS: usize = 2;
const RUNS: usize = 2;

/// The probe's payload: 16 header bytes, then one vD per run.
fn payload() -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_lve_lanes/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let cases = OPS * OFFSETS * VARIANTS * RUNS;
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(
        (len, frame.len() - 8),
        (16 + 16 * cases, len),
        "frame length"
    );
    let p = frame[8..].to_vec();
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!(word(0), 0x4C56_4531, "every case ran");
    assert_eq!(word(4) as usize, cases, "the case count");
    p
}

/// vD after instruction `op` at `offset` in variant `variant`, run `run`.
fn console_vd(p: &[u8], op: usize, offset: usize, variant: usize, run: usize) -> u128 {
    let at = 16 + 16 * (((op * OFFSETS + offset) * VARIANTS + variant) * RUNS + run);
    u128::from_be_bytes(p[at..at + 16].try_into().expect("16 bytes"))
}

/// CellGov's vD after instruction `op` at `offset` over the probe's line
/// and preset.
fn cellgov_vd(op: usize, offset: usize) -> u128 {
    let mut mem = vec![0u8; 0x1100];
    for i in 0..16 {
        mem[LINE as usize + i] = 0x10 + i as u8;
    }
    let (vt, ra, rb) = (1, 10, 4);
    let insn = match op {
        0 => PpuInstruction::Lvebx { vt, ra, rb },
        1 => PpuInstruction::Lvehx { vt, ra, rb },
        _ => PpuInstruction::Lvewx { vt, ra, rb },
    };
    let mut s = PpuState::new();
    s.set_gpr(10, LINE);
    s.set_gpr(4, offset as u64);
    s.set_vr(
        1,
        u128::from_be_bytes(std::array::from_fn(|i| 0xE0 + i as u8)),
    );
    let mut effects = Vec::new();
    assert_eq!(
        exec_with_mem(&insn, &mut s, 0, &mem, &mut effects),
        ExecuteVerdict::Continue
    );
    s.vr[1]
}

#[test]
fn each_element_load_leaves_vd_as_on_the_console() {
    let p = payload();
    for op in 0..OPS {
        for offset in 0..OFFSETS {
            let cellgov = cellgov_vd(op, offset);
            for variant in 0..VARIANTS {
                for run in 0..RUNS {
                    assert_eq!(
                        cellgov,
                        console_vd(&p, op, offset, variant, run),
                        "op {op} offset {offset} variant {variant} run {run}"
                    );
                }
            }
        }
    }
}
