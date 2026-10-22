//! csflt, cflts, cuflt and cfltu against an integer oracle, at the range
//! borders, over every defined scale, and at the undefined scales as the
//! console answers them.

use super::single_float_tests::{decode_operand, fpscr_of, truncate, SpFlags};
use super::*;
use crate::exec::lanes::{from_words, words};
use crate::exec::SpuFault;
use crate::state::SpuState;
use cellgov_ps3_abi::hw::spu_fpscr::fpscr_field;
use cellgov_ps3_abi::hw::spu_isa::{TO_FLOAT_SCALE_BIAS, TO_INTEGER_SCALE_BIAS};

/// [SPU-ISA p:220 s:9] RI8: 10-bit opcode, I8, RA, RT.
fn ri8(op: u32, rt: u32, ra: u32, imm: u32) -> u32 {
    op << 22 | imm << 14 | ra << 7 | rt
}

const CFLTS: u32 = 0x1D8;
const CFLTU: u32 = 0x1D9;
const CSFLT: u32 = 0x1DA;
const CUFLT: u32 = 0x1DB;

/// The I8 of scale 0 for each direction.
const TO_FLOAT_BIAS: u32 = TO_FLOAT_SCALE_BIAS as u32;
const TO_INTEGER_BIAS: u32 = TO_INTEGER_SCALE_BIAS as u32;

/// The oracle for one slot: the result word and its flags.
fn oracle(op: u32, word: u32, scale: i32) -> (u32, SpFlags) {
    match op {
        CSFLT => {
            let value = i64::from(word as i32);
            truncate(value < 0, value.unsigned_abs() as u128, -scale, false)
        }
        CUFLT => truncate(false, u128::from(word), -scale, false),
        _ => {
            let (negative, magnitude, exponent, _) = decode_operand(word);
            let shift = exponent + scale;
            // Truncated toward zero; anything past 2^40 saturates either way.
            let truncated = if magnitude == 0 {
                0
            } else if shift >= 0 {
                if shift > 40 {
                    1i128 << 80
                } else {
                    magnitude << shift
                }
            } else if shift < -64 {
                0
            } else {
                magnitude >> -shift
            };
            let value = if negative { -truncated } else { truncated };
            let bits = if op == CFLTS {
                value.clamp(i128::from(i32::MIN), i128::from(i32::MAX)) as i32 as u32
            } else {
                value.clamp(0, i128::from(u32::MAX)) as u32
            };
            (bits, SpFlags::default())
        }
    }
}

fn bias(op: u32) -> u32 {
    if matches!(op, CSFLT | CUFLT) {
        TO_FLOAT_BIAS
    } else {
        TO_INTEGER_BIAS
    }
}

fn run(op: u32, imm: u32, a: [u32; 4]) -> (SpuStepOutcome, SpuState) {
    let mut s = SpuState::new();
    s.set_reg(1, from_words(a));
    s.set_reg(3, [0xAA; 16]);
    let insn = crate::decode::decode(ri8(op, 3, 1, imm)).expect("decodes");
    (execute(&insn, &mut s, UnitId::new(0)), s)
}

fn check(op: u32, scale: i32, a: [u32; 4]) {
    let imm = bias(op) - scale as u32;
    let (outcome, s) = run(op, imm, a);
    assert_eq!(outcome, SpuStepOutcome::Continue);
    let expected: [(u32, SpFlags); 4] = std::array::from_fn(|i| oracle(op, a[i], scale));
    let context = format!("op {op:#x} scale {scale} {a:08x?}");
    assert_eq!(
        words(s.regs[3]),
        expected.map(|(bits, _)| bits),
        "{context}"
    );
    assert_eq!(s.fpscr(), fpscr_of(expected.map(|(_, f)| f)), "{context}");
}

/// Integers at the edges of both ranges, and small values.
const INTEGERS: [u32; 16] = [
    0x0000_0000,
    0x0000_0001,
    0x0000_0003,
    0x00FF_FFFF,
    0x0100_0001,
    0x7FFF_FF7F,
    0x7FFF_FFBF,
    0x7FFF_FFFF,
    0x8000_0000,
    0x8000_0001,
    0x8000_0080,
    0xFF00_0001,
    0xFFFF_FFFE,
    0xFFFF_FFFF,
    0x1234_5678,
    0xDEAD_BEEF,
];

/// Values just within, on and just outside each integer range at scale 0,
/// with -0, denormals, fractions and exponent 255.
const FLOATS: [u32; 24] = [
    0x4EFF_FFFF, // 2^31 - 128
    0x4F00_0000, // 2^31
    0xCF00_0000, // -2^31
    0xCF00_0001, // just below -2^31
    0x4F7F_FFFF, // 2^32 - 256
    0x4F80_0000, // 2^32
    0x8000_0000, // -0
    0x0000_0001, // a denormal, read as 0
    0x807F_FFFF, // a negative denormal
    0x3F00_0000, // 0.5
    0xBF00_0000, // -0.5
    0x3FFF_FFFF, // just below 2
    0xBFFF_FFFF, // just above -2
    0xBF80_0000, // -1
    0x7FFF_FFFF, // Smax, exponent 255
    0xFFFF_FFFF, // -Smax
    0x7F80_0000, // exponent 255, a number
    0x3F80_0000, // 1
    0x4B7F_FFFF, // 2^24 - 1
    0x4B80_0000, // 2^24
    0x0080_0000, // Smin
    0x42F6_E979, // 123.456
    0xC2F6_E979, // -123.456
    0x4F7F_FFFF,
];

#[test]
fn every_defined_scale_matches_the_oracle() {
    for scale in 0..=127 {
        for op in [CSFLT, CUFLT] {
            for chunk in INTEGERS.chunks(4) {
                check(op, scale, chunk.try_into().expect("four operands"));
            }
        }
        for op in [CFLTS, CFLTU] {
            for chunk in FLOATS.chunks(4) {
                check(op, scale, chunk.try_into().expect("four operands"));
            }
        }
    }
}

/// [SPU-ISA p:221 s:9] cflts saturates above 2^31 - 1 and below -2^31; [SPU-ISA p:223 s:9] cfltu saturates above 2^32 - 1 and every negative product to zero.
#[test]
fn the_integer_conversions_saturate_at_their_documented_borders() {
    let (_, s) = run(
        CFLTS,
        TO_INTEGER_BIAS,
        [0x4EFF_FFFF, 0x4F00_0000, 0xCF00_0000, 0xCF00_0001],
    );
    assert_eq!(
        words(s.regs[3]),
        [0x7FFF_FF80, 0x7FFF_FFFF, 0x8000_0000, 0x8000_0000]
    );
    let (_, s) = run(
        CFLTU,
        TO_INTEGER_BIAS,
        [0x4F7F_FFFF, 0x4F80_0000, 0xBF00_0000, 0x8000_0000],
    );
    assert_eq!(words(s.regs[3]), [0xFFFF_FF00, 0xFFFF_FFFF, 0, 0]);
    // Exponent 255 is a number and saturates; a denormal reads as zero.
    let (_, s) = run(
        CFLTS,
        TO_INTEGER_BIAS,
        [0x7F80_0000, 0xFFFF_FFFF, 0x0000_0001, 0x3FFF_FFFF],
    );
    assert_eq!(words(s.regs[3]), [0x7FFF_FFFF, 0x8000_0000, 0, 1]);
    // Truncation, not rounding: 1.99... and -1.99... give 1 and -1.
    let (_, s) = run(CFLTS, TO_INTEGER_BIAS, [0x3FFF_FFFF, 0xBFFF_FFFF, 0, 0]);
    assert_eq!(words(s.regs[3]), [1, 0xFFFF_FFFF, 0, 0]);
    assert_eq!(s.fpscr(), 0, "the integer conversions set no flag");
}

/// [SPU-ISA p:196 s:9.1] truncation is the only single-precision rounding.
#[test]
fn the_float_conversions_truncate_and_flush() {
    // 2^32 - 1 truncates to 2^32 - 256, not up to 2^32; -2^31 is exact.
    let (_, s) = run(
        CUFLT,
        TO_FLOAT_BIAS,
        [0xFFFF_FFFF, 0, 0x00FF_FFFF, 0x0100_0001],
    );
    assert_eq!(words(s.regs[3]), [0x4F7F_FFFF, 0, 0x4B7F_FFFF, 0x4B80_0000]);
    let (_, s) = run(CSFLT, TO_FLOAT_BIAS, [0x8000_0000, 0xFFFF_FFFF, 0, 0]);
    assert_eq!(words(s.regs[3]), [0xCF00_0000, 0xBF80_0000, 0, 0]);
    // At scale 127, 1 is 2^-127, below Smin: +0 with UNF and DIFF in its
    // slot; 2 is Smin.
    let (_, s) = run(CSFLT, TO_FLOAT_BIAS - 127, [0, 1, 2, 0xFFFF_FFFF]);
    assert_eq!(words(s.regs[3]), [0, 0, 0x0080_0000, 0]);
    assert_eq!(
        s.fpscr(),
        fpscr_field(62, 2) | fpscr_field(126, 2),
        "UNF and DIFF in slots 1 and 3"
    );
}

/// [SPU-ISA p:220 s:9] and [SPU-ISA p:221 s:9] leave a scale outside 0..=127 undefined; the `spu_convert_sweep` capture shows each conversion following its formula there.
#[test]
fn an_undefined_scale_follows_the_formula() {
    let inputs = [0x3F80_0000, 0xC040_0000, 0x0000_0007, 0x4B00_0001];
    for (op, imms) in [
        (CSFLT, [156, 200, 255, 0]),
        (CUFLT, [156, 200, 255, 0]),
        (CFLTS, [174, 200, 255, 16]),
        (CFLTU, [174, 200, 255, 16]),
    ] {
        for imm in imms {
            let scale = bias(op) as i32 - imm as i32;
            let (outcome, s) = run(op, imm, inputs);
            assert_eq!(outcome, SpuStepOutcome::Continue, "op {op:#x} I8 {imm}");
            let expected: [(u32, SpFlags); 4] =
                std::array::from_fn(|i| oracle(op, inputs[i], scale));
            assert_eq!(
                words(s.regs[3]),
                expected.map(|(bits, _)| bits),
                "op {op:#x} I8 {imm}"
            );
            assert_eq!(
                s.fpscr(),
                fpscr_of(expected.map(|(_, f)| f)),
                "op {op:#x} I8 {imm}"
            );
        }
    }
}

/// The `spu_convert_sweep` capture: at an undefined scale that brings the
/// biased exponent plus the scale to the saturation point (2^31 for
/// cflts, 2^32 for cfltu), a zero-exponent slot saturates by its sign.
#[test]
fn a_zero_exponent_saturates_once_its_exponent_reaches_the_saturation_point() {
    let zeros = [0x0000_0000, 0x8000_0000, 0x0000_0001, 0x807F_FFFF];
    // Scale 158 = TO_INTEGER_BIAS - 15.
    let (_, s) = run(CFLTS, 15, zeros);
    assert_eq!(
        words(s.regs[3]),
        [0x7FFF_FFFF, 0x8000_0000, 0x7FFF_FFFF, 0x8000_0000]
    );
    let (_, s) = run(CFLTU, 15, zeros);
    assert_eq!(words(s.regs[3]), [0; 4], "cfltu saturates one scale later");
    let (_, s) = run(CFLTU, 14, zeros);
    assert_eq!(words(s.regs[3]), [0xFFFF_FFFF, 0, 0xFFFF_FFFF, 0]);
    let (_, s) = run(CFLTS, 16, zeros);
    assert_eq!(words(s.regs[3]), [0; 4], "one scale short of the point");
}

/// The `spu_convert_sweep` capture: 256 or more past the saturation
/// point, the console's result depends on more than the slot's input,
/// so CellGov refuses the instruction.
#[test]
fn a_slot_far_past_the_saturation_point_is_a_named_refusal() {
    // Biased exponent 255 at scale 159 (cflts) or 160 (cfltu) is 256 past
    // the point; at one scale less it saturates.
    for (op, refusing, running) in [(CFLTS, 14, 15), (CFLTU, 13, 14)] {
        let input = [0x3F80_0000, 0x7F80_0000, 0, 0];
        let (outcome, s) = run(op, refusing, input);
        assert_eq!(
            outcome,
            SpuStepOutcome::Fault(SpuFault::UndefinedConversionScale(refusing as u8)),
            "op {op:#x}"
        );
        assert_eq!(s.regs[3], [0xAA; 16], "RT keeps its value");
        assert_eq!(s.fpscr(), 0);
        assert_eq!(run(op, running, input).0, SpuStepOutcome::Continue);
        assert_eq!(
            run(op, refusing, [0x7F00_0000; 4]).0,
            SpuStepOutcome::Continue,
            "exponent 254 is 255 past the point"
        );
    }
    let code = crate::fault_codes::guest_fault_for(SpuFault::UndefinedConversionScale(14));
    let cellgov_effects::FaultKind::Guest(code) = code else {
        panic!("expected a guest fault, got {code:?}");
    };
    assert_eq!(
        crate::describe_guest_fault(code).as_deref(),
        Some("SPU_UNDEFINED_CONVERSION_SCALE (detail=0x000e)")
    );
}

/// Every operand at scale 0, four at a time, against the oracle.
fn exhaustive(op: u32) {
    let insn = crate::decode::decode(ri8(op, 3, 1, bias(op))).expect("decodes");
    let mut s = SpuState::new();
    for base in (0..=u32::MAX).step_by(4) {
        let a = [base, base + 1, base + 2, base + 3];
        s.set_reg(1, from_words(a));
        s.set_fpscr(0);
        execute(&insn, &mut s, UnitId::new(0));
        let expected: [(u32, SpFlags); 4] = std::array::from_fn(|i| oracle(op, a[i], 0));
        assert_eq!(
            words(s.regs[3]),
            expected.map(|(bits, _)| bits),
            "{base:#010x}"
        );
        assert_eq!(
            s.fpscr(),
            fpscr_of(expected.map(|(_, f)| f)),
            "{base:#010x}"
        );
    }
}

#[test]
#[ignore = "exhaustive over 2^32 operands; run with --release -- --ignored"]
fn every_operand_at_scale_0_matches_the_oracle() {
    for op in [CSFLT, CUFLT, CFLTS, CFLTU] {
        exhaustive(op);
    }
}
