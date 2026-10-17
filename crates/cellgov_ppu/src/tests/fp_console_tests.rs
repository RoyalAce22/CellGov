//! The NaNs the FPU generates and propagates, held to the
//! `ppu_granule_nan` capture from the reference console. Each case runs
//! the probe's instruction on the probe's operands and compares the
//! result FPR with the one the console stored.

use super::*;
use crate::instruction::ops::{Fp59Op, Fp63Op};
use cellgov_ps3_abi::hw::ppc_isa::{PPC_F64_QUIET_BIT, PPC_GENERATED_QNAN_F64};

const INF: u64 = 0x7FF0_0000_0000_0000;
const NEG_INF: u64 = 0xFFF0_0000_0000_0000;
const ONE: u64 = 0x3FF0_0000_0000_0000;
const NEG_ONE: u64 = 0xBFF0_0000_0000_0000;
const QNAN_A: u64 = 0x7FF8_0000_0000_0AAA;
const QNAN_C: u64 = 0xFFF8_0000_0000_0BBB;
const SNAN_B: u64 = 0x7FF0_0000_0000_0123;

/// The probe's payload, as big-endian u32 words.
fn words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_granule_nan/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// The FPR the console stored for case `c`.
fn console_fpr(w: &[u32], c: usize) -> u64 {
    (u64::from(w[2 + 2 * c]) << 32) | u64::from(w[3 + 2 * c])
}

/// One probe case: primary opcode 59 or 63, XO, and fra, frb, frc.
struct Case {
    what: &'static str,
    single: bool,
    xo: u16,
    fra: u64,
    frb: u64,
    frc: u64,
}

const fn case(what: &'static str, single: bool, xo: u16, fra: u64, frb: u64, frc: u64) -> Case {
    Case {
        what,
        single,
        xo,
        fra,
        frb,
        frc,
    }
}

/// The probe's cases, in payload order. fdiv 18, fsub 20, fadd 21,
/// fsqrt 22, fmul 25 (fra * frc), fmadd 29 (fra * frc + frb).
const CASES: [Case; 14] = [
    case("fdiv 0/0", false, 18, 0, 0, 0),
    case("fdiv inf/inf", false, 18, INF, INF, 0),
    case("fsub inf-inf", false, 20, INF, INF, 0),
    case("fadd inf+-inf", false, 21, INF, NEG_INF, 0),
    case("fmul 0*inf", false, 25, 0, 0, INF),
    case("fsqrt -1", false, 22, 0, NEG_ONE, 0),
    case("fmadd inf*0+1", false, 29, INF, ONE, 0),
    case("fdivs 0/0", true, 18, 0, 0, 0),
    case("fadds inf+-inf", true, 21, INF, NEG_INF, 0),
    case("fsqrts -1", true, 22, 0, NEG_ONE, 0),
    case("fmadd 1*1+snan", false, 29, ONE, SNAN_B, ONE),
    case("fadd snan+1", false, 21, SNAN_B, ONE, 0),
    case("fadd qnan_c+qnan_a", false, 21, QNAN_C, QNAN_A, 0),
    case("fadd qnan_a+qnan_c", false, 21, QNAN_A, QNAN_C, 0),
];

fn run(c: &Case) -> u64 {
    let mut s = PpuState::new();
    s.set_fpr(1, c.fra);
    s.set_fpr(2, c.frb);
    s.set_fpr(3, c.frc);
    if c.single {
        let op = Fp59Op::from_xo(c.xo).expect("a documented xo");
        execute_fp59(&mut s, op, 0, 1, 2, 3);
    } else {
        let op = Fp63Op::from_xo(c.xo).expect("a documented xo");
        execute_fp63(&mut s, op, 0, 1, 2, 3);
    }
    s.fpr[0]
}

#[test]
fn every_case_leaves_the_fpr_the_console_stored() {
    let w = words();
    assert_eq!(w[0], 0, "the probe ran to its end");
    for (c, case) in CASES.iter().enumerate() {
        assert_eq!(run(case), console_fpr(&w, c), "case {c}: {}", case.what);
    }
}

#[test]
fn the_generated_qnan_and_the_quiet_bit_are_the_console_s() {
    let w = words();
    for (c, case) in CASES.iter().enumerate().take(10) {
        assert_eq!(
            console_fpr(&w, c),
            PPC_GENERATED_QNAN_F64,
            "case {c}: {}",
            case.what
        );
    }
    assert_eq!(
        console_fpr(&w, 11),
        SNAN_B | PPC_F64_QUIET_BIT,
        "an SNaN gains the quiet bit"
    );
}
