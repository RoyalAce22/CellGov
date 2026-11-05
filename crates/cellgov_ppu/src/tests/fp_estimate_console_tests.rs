//! `fres` and `frsqrte` held to the `ppu_estimate_specials` capture from
//! the reference console. Each case runs the probe's instruction on the
//! probe's operand and compares FRT with the one the console stored.

use super::*;
use crate::instruction::ops::{Fp59Op, Fp63Op};

/// The probe's payload, as big-endian u32 words.
fn words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_estimate_specials/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// The FRT the console stored for case `c`.
fn console_frt(w: &[u32], c: usize) -> u64 {
    (u64::from(w[2 + 2 * c]) << 32) | u64::from(w[3 + 2 * c])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Fres,
    Frsqrte,
}

fn run(op: Op, frb: u64) -> u64 {
    let mut s = PpuState::new();
    s.set_fpr(2, frb);
    match op {
        Op::Fres => execute_fp59(&mut s, Fp59Op::Fres, 0, 1, 2, 3),
        Op::Frsqrte => execute_fp63(&mut s, Fp63Op::Frsqrte, 0, 1, 2, 3),
    };
    s.fpr[0]
}

/// The probe's cases, in payload order: the instruction, FRB, and
/// whether the book's special-operand table fixes the result.
const CASES: [(Op, u64, bool); 24] = [
    (Op::Fres, 0xFFF0_0000_0000_0000, true),
    (Op::Fres, 0x8000_0000_0000_0000, true),
    (Op::Fres, 0x0000_0000_0000_0000, true),
    (Op::Fres, 0x7FF0_0000_0000_0000, true),
    (Op::Fres, 0x7FF0_0000_0000_5678, true),
    (Op::Fres, 0xFFF8_0000_0000_1234, true),
    (Op::Fres, 0x4000_0000_0000_0000, false),
    (Op::Fres, 0xBFD0_0000_0000_0000, false),
    (Op::Fres, 0x3FF0_0000_1000_0001, false),
    (Op::Fres, 0x3FF0_0000_0000_0000, false),
    (Op::Fres, 0x4008_0000_0000_0000, false),
    // 2^-200: its reciprocal overflows single precision.
    (Op::Fres, 0x2370_0000_0000_0000, true),
    (Op::Frsqrte, 0xFFF0_0000_0000_0000, true),
    (Op::Frsqrte, 0xBFF0_0000_0000_0000, true),
    (Op::Frsqrte, 0x8000_0000_0000_0001, true),
    (Op::Frsqrte, 0x8000_0000_0000_0000, true),
    (Op::Frsqrte, 0x0000_0000_0000_0000, true),
    (Op::Frsqrte, 0x7FF0_0000_0000_0000, true),
    (Op::Frsqrte, 0x7FF0_0000_0000_5678, true),
    (Op::Frsqrte, 0xFFF8_0000_0000_1234, true),
    (Op::Frsqrte, 0x4010_0000_0000_0000, false),
    (Op::Frsqrte, 0x3FF0_0000_0000_0000, false),
    (Op::Frsqrte, 0x4000_0000_0000_0000, false),
    (Op::Frsqrte, 0x4008_0000_0000_0000, false),
];

#[test]
fn every_special_operand_gives_the_frt_the_console_stored() {
    let w = words();
    assert_eq!(w[0], 0, "the probe ran to its end");
    let mut special = 0;
    for (c, (op, frb, fixed)) in CASES.iter().enumerate() {
        if *fixed {
            assert_eq!(
                run(*op, *frb),
                console_frt(&w, c),
                "case {c}: {op:?} 0x{frb:016x}"
            );
            special += 1;
        }
    }
    assert_eq!(special, 15);
}

/// The special operands of the `ppu_fp_estimate_readout` capture.
///
/// Each estimate starts from a cleared FPSCR. The capture holds four
/// doublewords for each operand:
/// 1. the FRT of fres
/// 2. the FPSCR after fres
/// 3. the FRT of frsqrte
/// 4. the FPSCR after frsqrte
const READOUT_SPECIALS: [u64; 16] = [
    0x0000_0000_0000_0000,
    0x8000_0000_0000_0000,
    0x7FF0_0000_0000_0000,
    0xFFF0_0000_0000_0000,
    0x7FF8_0000_0000_0000,
    0x7FF4_0000_0000_0001,
    0x0000_0000_0000_0001,
    0x800F_FFFF_FFFF_FFFF,
    0x3FF0_0000_0000_0000,
    0xBFF0_0000_0000_0000,
    0x47F0_0000_0000_0000,
    0x3800_0000_0000_0000,
    0x7FEF_FFFF_FFFF_FFFF,
    0x0010_0000_0000_0000,
    0x36A0_0000_0000_0000,
    0x47EF_FFFF_E000_0000,
];

/// The readout capture as big-endian doublewords.
fn readout() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_fp_estimate_readout/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

#[test]
fn every_special_operand_leaves_the_fpscr_the_console_read() {
    let d = readout();
    assert_eq!(d.len(), 36_928);
    let specials = &d[d.len() - 4 * READOUT_SPECIALS.len()..];
    let mut compared = 0;
    for (i, &frb) in READOUT_SPECIALS.iter().enumerate() {
        for (k, op) in [Op::Fres, Op::Frsqrte].into_iter().enumerate() {
            let mut s = PpuState::new();
            s.set_fpr(2, frb);
            match op {
                Op::Fres => execute_fp59(&mut s, Fp59Op::Fres, 0, 1, 2, 3),
                Op::Frsqrte => execute_fp63(&mut s, Fp63Op::Frsqrte, 0, 1, 2, 3),
            };
            let at = 4 * i + 2 * k;
            assert_eq!(s.fpr[0], specials[at], "{op:?} 0x{frb:016x} FRT");
            assert_eq!(
                s.fpscr(),
                specials[at + 1] as u32,
                "{op:?} 0x{frb:016x}: CellGov 0x{:08x}, console 0x{:08x}",
                s.fpscr(),
                specials[at + 1] as u32
            );
            compared += 1;
        }
    }
    assert_eq!(compared, 32);
}

#[test]
fn every_ordinary_operand_gives_the_frt_the_console_stored() {
    let w = words();
    let mut ordinary = 0;
    for (c, (op, frb, fixed)) in CASES.iter().enumerate() {
        if *fixed {
            continue;
        }
        assert_eq!(
            run(*op, *frb),
            console_frt(&w, c),
            "case {c}: {op:?} 0x{frb:016x}"
        );
        ordinary += 1;
    }
    assert_eq!(ordinary, 9);
}
