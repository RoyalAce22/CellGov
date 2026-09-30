//! fceq, fcmeq, fcgt and fcmgt against host `f64` ordering, which is exact
//! for every extended-range single-precision value.

use super::single_float_tests::decode_operand;
use super::*;
use crate::state::SpuState;

/// [SPU-ISA p:28 s:2.3] RR: 11-bit opcode, RB, RA, RT.
fn rr(op: u32, rt: u32, ra: u32, rb: u32) -> u32 {
    op << 21 | rb << 14 | ra << 7 | rt
}

const FCEQ: u32 = 0x3C2;
const FCMEQ: u32 = 0x3CA;
const FCGT: u32 = 0x2C2;
const FCMGT: u32 = 0x2CA;

/// The word's value: zero for a zero exponent, exponent 255 a number.
fn value(bits: u32) -> f64 {
    let (negative, magnitude, exponent, _) = decode_operand(bits);
    let v = magnitude as f64 * 2f64.powi(exponent);
    if negative {
        -v
    } else {
        v
    }
}

/// The oracle's answer for one slot.
fn holds(op: u32, a: u32, b: u32) -> bool {
    let (x, y) = (value(a), value(b));
    match op {
        FCEQ => x == y,
        FCMEQ => x.abs() == y.abs(),
        FCGT => x > y,
        _ => x.abs() > y.abs(),
    }
}

fn run(op: u32, a: [u32; 4], b: [u32; 4]) -> ([u32; 4], u128) {
    let mut s = SpuState::new();
    s.regs[1] = from_words(a);
    s.regs[2] = from_words(b);
    s.regs[3] = [0xAA; 16];
    let insn = crate::decode::decode(rr(op, 3, 1, 2)).expect("decodes");
    assert_eq!(
        execute(&insn, &mut s, UnitId::new(0)),
        SpuStepOutcome::Continue
    );
    (words(s.regs[3]), s.fpscr)
}

fn check(op: u32, a: [u32; 4], b: [u32; 4]) {
    let (got, fpscr) = run(op, a, b);
    let want = std::array::from_fn(|i| if holds(op, a[i], b[i]) { u32::MAX } else { 0 });
    assert_eq!(got, want, "op {op:#x} {a:08x?} {b:08x?}");
    assert_eq!(fpscr, 0, "the compares set no flag");
}

/// Zeros and denormals of both signs, Smin and its neighbours, ordinary
/// values one ulp apart across a binade, exponent 254 and 255, Smax.
const VALUES: [u32; 16] = [
    0x0000_0000,
    0x0000_0001,
    0x007F_FFFF,
    0x0080_0000,
    0x0080_0001,
    0x3F7F_FFFF,
    0x3F80_0000,
    0x3F80_0001,
    0x4049_0FDB,
    0x7F7F_FFFF,
    0x7F80_0000,
    0x7F80_0001,
    0x7FFF_FFFE,
    0x7FFF_FFFF,
    0x0040_0000,
    0x1234_5678,
];

#[test]
fn every_value_pair_orders_as_its_value() {
    let signed: Vec<u32> = VALUES
        .iter()
        .flat_map(|&bits| [bits, bits | 0x8000_0000])
        .collect();
    for op in [FCEQ, FCMEQ, FCGT, FCMGT] {
        for chunk in signed.chunks(4) {
            for &b in &signed {
                check(op, chunk.try_into().expect("four operands"), [b; 4]);
            }
        }
    }
}

/// [SPU-ISA p:231 s:9] two zeros compare equal independent of their fractions and signs; [SPU-ISA p:233 s:9] and never greater.
#[test]
fn every_zero_is_equal_and_never_greater() {
    let zeros = [0x0000_0000, 0x8000_0000, 0x0000_0001, 0x807F_FFFF];
    for op in [FCEQ, FCMEQ] {
        for &b in &zeros {
            assert_eq!(
                run(op, zeros, [b; 4]).0,
                [u32::MAX; 4],
                "op {op:#x} {b:08x}"
            );
        }
    }
    for op in [FCGT, FCMGT] {
        for &b in &zeros {
            assert_eq!(run(op, zeros, [b; 4]).0, [0; 4], "op {op:#x} {b:08x}");
        }
    }
    // A denormal is below Smin.
    assert_eq!(
        run(FCGT, [0x0080_0000; 4], [0x007F_FFFF; 4]).0,
        [u32::MAX; 4]
    );
}

/// [CBE-Handbook p:69 s:3.1.4] exponent 255 is a number, greater than every smaller magnitude.
#[test]
fn exponent_255_orders_as_a_number() {
    let (got, _) = run(
        FCGT,
        [0x7F80_0000, 0x7FFF_FFFF, 0xFF80_0000, 0x7F80_0000],
        [0x7F7F_FFFF, 0x7FFF_FFFE, 0x7F7F_FFFF, 0x7FFF_FFFF],
    );
    assert_eq!(got, [u32::MAX, u32::MAX, 0, 0]);
    // The magnitude forms ignore the sign.
    let (got, _) = run(
        FCMGT,
        [0xFF80_0000, 0x3F80_0001, 0xBF80_0000, 0x3F80_0000],
        [0x7F7F_FFFF, 0xBF80_0000, 0x3F80_0000, 0xBF80_0001],
    );
    assert_eq!(got, [u32::MAX, u32::MAX, 0, 0]);
    let (got, _) = run(
        FCMEQ,
        [0xBF80_0000, 0x7FFF_FFFF, 0x3F80_0000, 0x3F80_0000],
        [0x3F80_0000, 0xFFFF_FFFF, 0x3F80_0001, 0x3F80_0000],
    );
    assert_eq!(got, [u32::MAX, u32::MAX, 0, u32::MAX]);
}

#[test]
#[ignore = "exhaustive over 2^32 first operands; run with --release -- --ignored"]
fn every_first_operand_orders_as_its_value() {
    let seconds = [0x0000_0000, 0x8040_0000, 0x3F80_0000, 0xFF7F_FFFF];
    for op in [FCEQ, FCMEQ, FCGT, FCMGT] {
        let insn = crate::decode::decode(rr(op, 3, 1, 2)).expect("decodes");
        let mut s = SpuState::new();
        s.regs[2] = from_words(seconds);
        for a in 0..=u32::MAX {
            s.regs[1] = from_words([a; 4]);
            execute(&insn, &mut s, UnitId::new(0));
            let want: [u32; 4] = std::array::from_fn(|i| {
                if holds(op, a, seconds[i]) {
                    u32::MAX
                } else {
                    0
                }
            });
            assert_eq!(words(s.regs[3]), want, "op {op:#x} {a:#010x}");
        }
        assert_eq!(s.fpscr, 0);
    }
}
