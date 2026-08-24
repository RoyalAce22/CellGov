//! FP59/FP63 floating-point execution, including fused multiply-add precision.

use super::*;
use crate::instruction::ops::{Fp59Op, Fp63Op};

fn f64_bits(v: f64) -> u64 {
    v.to_bits()
}

/// Resolve a raw XO so each test keeps its spec-citable literal.
fn op63(xo: u16) -> Fp63Op {
    Fp63Op::from_xo(xo).unwrap_or_else(|| panic!("undocumented test xo {xo}"))
}

fn op59(xo: u16) -> Fp59Op {
    Fp59Op::from_xo(xo).unwrap_or_else(|| panic!("undocumented test xo {xo}"))
}

fn run63(xo: u16, fra_v: f64, frb_v: f64, frc_v: f64) -> f64 {
    let mut s = PpuState::new();
    s.set_fpr(1, f64_bits(fra_v));
    s.set_fpr(2, f64_bits(frb_v));
    s.set_fpr(3, f64_bits(frc_v));
    execute_fp63(&mut s, op63(xo), 0, 1, 2, 3);
    f64::from_bits(s.fpr[0])
}

fn run59(xo: u16, fra_v: f64, frb_v: f64, frc_v: f64) -> f64 {
    let mut s = PpuState::new();
    s.set_fpr(1, f64_bits(fra_v));
    s.set_fpr(2, f64_bits(frb_v));
    s.set_fpr(3, f64_bits(frc_v));
    execute_fp59(&mut s, op59(xo), 0, 1, 2, 3);
    f64::from_bits(s.fpr[0])
}

#[test]
fn fmadd_uses_fused_multiply_add() {
    // a*c (exact) = 10_000_000_400_000_003 -- odd, so f64
    // rounds to the nearest-even neighbour 10_000_000_400_000_002.
    // Naive a*c + b therefore yields 0; the fused path keeps
    // the lost trailing bit and yields 1. The values are chosen
    // so the FMA-vs-non-FMA distinction is observable.
    let a = 100_000_001.0_f64;
    let c = 100_000_003.0_f64;
    let b = -10_000_000_400_000_002.0_f64;
    let naive = a * c + b;
    let fused = a.mul_add(c, b);
    assert_ne!(
        naive, fused,
        "test values must distinguish FMA from multiply-then-add"
    );
    // xo = 29 -> fmadd
    let r = run63(29, a, b, c);
    assert_eq!(r, fused);
}

#[test]
fn fnmadd_preserves_nan_sign() {
    // a*c+b is NaN (any NaN operand poisons the result).
    // fnmadd must preserve the NaN's sign bit, not flip it.
    let nan = f64::NAN;
    // xo = 31 -> fnmadd
    let r = run63(31, nan, 0.0, 1.0);
    assert!(r.is_nan());
    // Default Rust NaN has sign bit 0; result must keep it 0.
    assert_eq!(r.to_bits() >> 63, 0, "fnmadd flipped NaN sign");
}

#[test]
fn fnmsub_preserves_nan_sign() {
    let nan = f64::NAN;
    // xo = 30 -> fnmsub
    let r = run63(30, nan, 0.0, 1.0);
    assert!(r.is_nan());
    assert_eq!(r.to_bits() >> 63, 0, "fnmsub flipped NaN sign");
}

#[test]
fn fnmadd_negates_finite_results() {
    // Sanity: non-NaN path still negates.
    // a=2, c=3, b=1 -> fma=7 -> fnmadd=-7
    // xo = 31 -> fnmadd
    let r = run63(31, 2.0, 1.0, 3.0);
    assert_eq!(r, -7.0);
}

#[test]
fn fctiwz_nan_input_produces_min_int_low32() {
    let nan_bits = f64::NAN.to_bits();
    let mut s = PpuState::new();
    s.set_fpr(2, nan_bits);
    // xo = 15 -> fctiwz
    execute_fp63(&mut s, op63(15), 0, 0, 2, 0);
    // Spec: bits 32:63 = 0x8000_0000.
    assert_eq!(s.fpr[0] & 0xFFFF_FFFF, 0x8000_0000);
}

#[test]
fn fctiw_xo14_dispatches() {
    // Pins the xo=14 arm. A dispatcher that drops through for
    // xo=14 would leave the destination FPR at its prior value;
    // observe a write by seeding a sentinel and checking it
    // gets overwritten.
    let mut s = PpuState::new();
    s.set_fpr(0, 0xDEAD_BEEFu64);
    s.set_fpr(2, f64::to_bits(42.0));
    execute_fp63(&mut s, op63(14), 0, 0, 2, 0);
    assert_eq!(s.fpr[0] & 0xFFFF_FFFF, 42);
}

#[test]
fn fctidz_nan_input_produces_min_int64() {
    let mut s = PpuState::new();
    s.set_fpr(2, f64::NAN.to_bits());
    // xo = 815 -> fctidz
    execute_fp63(&mut s, op63(815), 0, 0, 2, 0);
    assert_eq!(s.fpr[0], 0x8000_0000_0000_0000);
}

#[test]
fn fctid_xo814_dispatches() {
    let mut s = PpuState::new();
    s.set_fpr(0, 0xDEAD_BEEFu64);
    s.set_fpr(2, f64::to_bits(42.0));
    execute_fp63(&mut s, op63(814), 0, 0, 2, 0);
    assert_eq!(s.fpr[0], 42);
}

#[test]
fn fp59_fnmadds_preserves_nan_sign() {
    // Single-precision counterpart of fnmadd. xo = 31 -> fnmadds.
    let r = run59(31, f64::NAN, 0.0, 1.0);
    assert!(r.is_nan());
    assert_eq!(r.to_bits() >> 63, 0);
}

#[test]
fn fadd_adds_two_finite_doubles() {
    // xo = 21 -> fadd. FRT <- FRA + FRB; FRC ignored.
    let r = run63(21, 1.0, 2.0, 9999.0);
    assert_eq!(r.to_bits(), 0x4008_0000_0000_0000); // 3.0
}

#[test]
fn fsub_subtracts_two_finite_doubles() {
    // xo = 20 -> fsub. FRT <- FRA - FRB.
    let r = run63(20, 5.0, 3.0, 0.0);
    assert_eq!(r.to_bits(), 0x4000_0000_0000_0000); // 2.0
}

#[test]
fn fmul_multiplies_two_finite_doubles() {
    // xo5 = 25 -> fmul. FRT <- FRA * FRC; FRB ignored.
    let r = run63(25, 2.0, 9999.0, 3.0);
    assert_eq!(r.to_bits(), 0x4018_0000_0000_0000); // 6.0
}

#[test]
fn fdiv_divides_two_finite_doubles() {
    // xo = 18 -> fdiv. FRT <- FRA / FRB.
    let r = run63(18, 6.0, 2.0, 0.0);
    assert_eq!(r.to_bits(), 0x4008_0000_0000_0000); // 3.0
}

#[test]
fn fmsub_computes_a_times_c_minus_b() {
    // xo5 = 28 -> fmsub. FRT <- (FRA * FRC) - FRB.
    // 2*3 - 1 = 5.
    let r = run63(28, 2.0, 1.0, 3.0);
    assert_eq!(r, 5.0);
}

#[test]
fn fnmsub_negates_finite_results() {
    // xo5 = 30 -> fnmsub. FRT <- -((FRA * FRC) - FRB).
    // 2*3 - 1 = 5 -> negated = -5.
    let r = run63(30, 2.0, 1.0, 3.0);
    assert_eq!(r, -5.0);
}

#[test]
fn fmr_copies_frb_to_frt() {
    // xo = 72 -> fmr. FRT <- FRB; bit-exact, NaN payload preserved.
    let mut s = PpuState::new();
    let payload = 0x7FF8_0000_DEAD_BEEFu64; // QNaN with payload
    s.set_fpr(2, payload);
    execute_fp63(&mut s, op63(72), 0, 0, 2, 0);
    assert_eq!(s.fpr[0], payload);
}

#[test]
fn fneg_flips_sign_of_frb() {
    // xo = 40 -> fneg. FRT <- ~FRB[0] || FRB[1:63].
    let r = run63(40, 0.0, 1.0, 0.0);
    assert_eq!(r.to_bits(), 0xBFF0_0000_0000_0000); // -1.0
}

#[test]
fn fabs_clears_sign_of_frb() {
    // xo = 264 -> fabs. FRT <- 0 || FRB[1:63].
    let r = run63(264, 0.0, -1.0, 0.0);
    assert_eq!(r.to_bits(), 0x3FF0_0000_0000_0000); // 1.0
}

#[test]
fn fnabs_sets_sign_of_frb() {
    // xo = 136 -> fnabs. FRT <- 1 || FRB[1:63].
    let r = run63(136, 0.0, 1.0, 0.0);
    assert_eq!(r.to_bits(), 0xBFF0_0000_0000_0000); // -1.0
}

#[test]
fn fsel_picks_frc_when_fra_nonneg() {
    // xo5 = 23 -> fsel. FRA = +0 satisfies >= 0, so FRT <- FRC.
    let r = run63(23, 0.0, 99.0, 7.0);
    assert_eq!(r, 7.0);
}

#[test]
fn fsel_picks_frb_when_fra_negative() {
    // FRA < 0 -> FRT <- FRB.
    let r = run63(23, -1.0, 42.0, 7.0);
    assert_eq!(r, 42.0);
}

#[test]
fn fsel_picks_frb_when_fra_nan() {
    // NaN is not >= 0 -> FRT <- FRB.
    let r = run63(23, f64::NAN, 42.0, 7.0);
    assert_eq!(r, 42.0);
}

#[test]
fn frsp_rounds_double_to_single_precision() {
    // xo = 12 -> frsp. Double that isn't representable in single
    // precision rounds to nearest f32 value, then re-widened.
    // 1.0 + 2^-30 is exactly representable in f64 but not f32;
    // the low mantissa bits get rounded off.
    let one_plus_eps = f64::from_bits(0x3FF0_0000_0000_0001); // smallest > 1.0 in f64
    let r = run63(12, 0.0, one_plus_eps, 0.0);
    // Result must be the f32 round-trip, not the original f64.
    let expected = (one_plus_eps as f32) as f64;
    assert_eq!(r.to_bits(), expected.to_bits());
    // And specifically: round-to-nearest of bits past f32 mantissa
    // collapses to exactly 1.0.
    assert_eq!(r, 1.0);
}

#[test]
fn fcfid_converts_signed_int64_to_double() {
    // xo = 846 -> fcfid. FRB's raw bits are read as i64.
    let mut s = PpuState::new();
    s.set_fpr(2, (-3i64) as u64);
    execute_fp63(&mut s, op63(846), 0, 0, 2, 0);
    assert_eq!(f64::from_bits(s.fpr[0]), -3.0);
}

#[test]
fn fcmpu_finite_lt_sets_lt_bit() {
    // xo = 0 -> fcmpu. BF = (FRT >> 2) & 7.
    let mut s = PpuState::new();
    s.set_fpr(1, f64_bits(1.0));
    s.set_fpr(2, f64_bits(2.0));
    // FRT = 0 -> BF = 0; 1.0 < 2.0 -> LT bit (0b1000) in CR0.
    execute_fp63(&mut s, op63(0), 0, 1, 2, 0);
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn fcmpu_handles_nan_unordered() {
    // NaN vs anything -> FU bit (0b0001) in target CR field.
    let mut s = PpuState::new();
    s.set_fpr(1, f64::NAN.to_bits());
    s.set_fpr(2, f64_bits(0.0));
    execute_fp63(&mut s, op63(0), 0, 1, 2, 0);
    assert_eq!(s.cr_field(0), 0b0001);
}

#[test]
fn fcmpo_writes_to_bf_from_frt_high_bits() {
    // FRT = 0b01100 -> BF = top 3 bits = 0b011 = 3.
    // 5.0 > 3.0 -> GT bit (0b0100) in CR3.
    let mut s = PpuState::new();
    s.set_fpr(1, f64_bits(5.0));
    s.set_fpr(2, f64_bits(3.0));
    execute_fp63(&mut s, op63(32), 0b01100, 1, 2, 0);
    assert_eq!(s.cr_field(3), 0b0100);
    // CR0 untouched.
    assert_eq!(s.cr_field(0), 0);
}

#[test]
fn fcmpu_finite_equal_sets_eq_bit() {
    let mut s = PpuState::new();
    s.set_fpr(1, f64_bits(2.5));
    s.set_fpr(2, f64_bits(2.5));
    execute_fp63(&mut s, op63(0), 0, 1, 2, 0);
    assert_eq!(s.cr_field(0), 0b0010);
}

fn conv32(xo: u16, v: f64) -> u32 {
    let mut s = PpuState::new();
    s.set_fpr(2, f64_bits(v));
    execute_fp63(&mut s, op63(xo), 0, 0, 2, 0);
    (s.fpr[0] & 0xFFFF_FFFF) as u32
}

fn conv64(xo: u16, v: f64) -> i64 {
    let mut s = PpuState::new();
    s.set_fpr(2, f64_bits(v));
    execute_fp63(&mut s, op63(xo), 0, 0, 2, 0);
    s.fpr[0] as i64
}

#[test]
fn fctiw_rounds_to_nearest_even_while_fctiwz_truncates() {
    // xo = 14 -> fctiw (FPSCR[RN], modelled as round-to-nearest-even);
    // xo = 15 -> fctiwz (round toward zero).
    assert_eq!(conv32(14, 3.7), 4);
    assert_eq!(conv32(15, 3.7), 3);
    // Ties go to the even neighbour, not away from zero.
    assert_eq!(conv32(14, 2.5), 2);
    assert_eq!(conv32(14, 3.5), 4);
    assert_eq!(conv32(14, -2.5), -2i32 as u32);
    // Negative magnitudes round the same way; the z form still truncates.
    assert_eq!(conv32(14, -3.7), -4i32 as u32);
    assert_eq!(conv32(15, -3.7), -3i32 as u32);
}

#[test]
fn fctiw_saturates_past_the_signed_32_bit_endpoints() {
    // Operand above 2^31-1 -> 0x7FFF_FFFF, below -2^31 -> 0x8000_0000.
    assert_eq!(conv32(14, 4e9), 0x7FFF_FFFF);
    assert_eq!(conv32(14, -4e9), 0x8000_0000);
    assert_eq!(conv32(14, f64::INFINITY), 0x7FFF_FFFF);
    assert_eq!(conv32(14, f64::NEG_INFINITY), 0x8000_0000);
}

#[test]
fn fctid_rounds_to_nearest_even_while_fctidz_truncates() {
    // xo = 814 -> fctid, xo = 815 -> fctidz.
    assert_eq!(conv64(814, -42.9), -43);
    assert_eq!(conv64(815, -42.9), -42);
    assert_eq!(conv64(814, 2.5), 2);
    assert_eq!(conv64(814, 3.5), 4);
}

// ----- single-precision (Fp59) arms -----

#[test]
fn fadds_adds_two_finite_singles() {
    // xo = 21 -> fadds. Sum rounded to single precision.
    let r = run59(21, 1.0, 2.0, 0.0);
    assert_eq!(r, 3.0_f32 as f64);
}

#[test]
fn fsubs_subtracts_two_finite_singles() {
    let r = run59(20, 5.0, 3.0, 0.0);
    assert_eq!(r, 2.0_f32 as f64);
}

#[test]
fn fmuls_multiplies_two_finite_singles() {
    // FRC, not FRB. xo5 = 25.
    let r = run59(25, 2.0, 0.0, 3.0);
    assert_eq!(r, 6.0_f32 as f64);
}

#[test]
fn fdivs_divides_two_finite_singles() {
    let r = run59(18, 6.0, 2.0, 0.0);
    assert_eq!(r, 3.0_f32 as f64);
}

#[test]
fn fmadds_does_single_rounding() {
    // 2*3+1 = 7, exactly representable in single.
    let r = run59(29, 2.0, 1.0, 3.0);
    assert_eq!(r, 7.0_f32 as f64);
}

#[test]
fn fmsubs_computes_a_times_c_minus_b_in_single() {
    // 2*3 - 1 = 5.
    let r = run59(28, 2.0, 1.0, 3.0);
    assert_eq!(r, 5.0_f32 as f64);
}

#[test]
fn fnmsubs_negates_finite_results() {
    // -(2*3 - 1) = -5.
    let r = run59(30, 2.0, 1.0, 3.0);
    assert_eq!(r, -5.0_f32 as f64);
}

// ----- NaN operand propagation -----

/// QNaN with a distinguishable payload and a clear sign bit.
const QNAN_A: u64 = 0x7FF8_0000_0000_0AAA;
/// A second QNaN, sign bit set, so a mis-picked operand is visible.
const QNAN_C: u64 = 0xFFF8_0000_0000_0BBB;
/// SNaN: maximum exponent, high-order fraction bit clear, payload set.
const SNAN_B: u64 = 0x7FF0_0000_0000_0123;

fn run63_bits(xo: u16, fra: u64, frb: u64, frc: u64) -> u64 {
    let mut s = PpuState::new();
    s.set_fpr(1, fra);
    s.set_fpr(2, frb);
    s.set_fpr(3, frc);
    execute_fp63(&mut s, op63(xo), 0, 1, 2, 3);
    s.fpr[0]
}

#[test]
fn fmsub_propagates_the_frb_nan_without_flipping_its_sign() {
    // xo5 = 28 -> fmsub. Computing FRA*FRC + (-FRB) would hand the
    // FMA a sign-flipped NaN; the architected result is FRB verbatim.
    let r = run63_bits(28, f64_bits(1.0), QNAN_A, f64_bits(1.0));
    assert_eq!(r, QNAN_A);
}

#[test]
fn fnmsub_propagates_the_frb_nan_without_flipping_its_sign() {
    // xo5 = 30 -> fnmsub. Neither the operand negation nor the result
    // negation may touch a propagated QNaN's sign bit.
    let r = run63_bits(30, f64_bits(1.0), QNAN_A, f64_bits(1.0));
    assert_eq!(r, QNAN_A);
}

#[test]
fn a_multiply_add_picks_the_fra_nan_ahead_of_the_frc_nan() {
    // xo = 29 -> fmadd. FRA wins the FRA/FRB/FRC precedence walk.
    let r = run63_bits(29, QNAN_A, f64_bits(1.0), QNAN_C);
    assert_eq!(r, QNAN_A);
}

#[test]
fn a_multiply_add_picks_the_frb_nan_ahead_of_the_frc_nan() {
    let r = run63_bits(29, f64_bits(1.0), QNAN_A, QNAN_C);
    assert_eq!(r, QNAN_A);
}

#[test]
fn a_propagated_snan_is_quieted_in_place_keeping_sign_and_payload() {
    let r = run63_bits(29, f64_bits(1.0), SNAN_B, f64_bits(1.0));
    assert_eq!(r, SNAN_B | 0x0008_0000_0000_0000);
}

#[test]
fn fmsubs_propagates_the_frb_nan_at_full_double_width() {
    // Only frsp clears the low-order bits of a propagated NaN, so the
    // single-precision multiply-add must not narrow the payload.
    let mut s = PpuState::new();
    s.set_fpr(1, f64_bits(1.0));
    s.set_fpr(2, QNAN_A);
    s.set_fpr(3, f64_bits(1.0));
    execute_fp59(&mut s, op59(28), 0, 1, 2, 3);
    assert_eq!(s.fpr[0], QNAN_A);
}

// ----- square root -----

#[test]
fn fsqrt_writes_the_square_root_of_frb() {
    // xo = 22 -> fsqrt.
    let mut s = PpuState::new();
    s.set_fpr(0, 0xDEAD_BEEF);
    s.set_fpr(2, f64_bits(16.0));
    execute_fp63(&mut s, op63(22), 0, 0, 2, 0);
    assert_eq!(f64::from_bits(s.fpr[0]), 4.0);
}

#[test]
fn fsqrts_rounds_the_square_root_to_single_precision() {
    // xo = 22 -> fsqrts.
    let r = run59(22, 0.0, 2.0, 0.0);
    assert_eq!(r, (2.0_f32.sqrt()) as f64);
}

// ----- generated QNaN -----

/// The one QNaN a disabled Invalid Operation Exception may produce.
const GENERATED: u64 = 0x7FF8_0000_0000_0000;

fn run59_bits(xo: u16, fra: u64, frb: u64, frc: u64) -> u64 {
    let mut s = PpuState::new();
    s.set_fpr(1, fra);
    s.set_fpr(2, frb);
    s.set_fpr(3, frc);
    execute_fp59(&mut s, op59(xo), 0, 1, 2, 3);
    s.fpr[0]
}

const INF: u64 = 0x7FF0_0000_0000_0000;
const NEG_INF: u64 = 0xFFF0_0000_0000_0000;

#[test]
fn every_invalid_operation_yields_the_sign_clear_generated_qnan() {
    // The host answers each of these with a sign-bit-set QNaN.
    // xo 18 fdiv, 20 fsub, 21 fadd, 25 fmul, 22 fsqrt, 29 fmadd.
    assert_eq!(run63_bits(18, 0, 0, 0), GENERATED, "0 / 0");
    assert_eq!(run63_bits(18, INF, INF, 0), GENERATED, "inf / inf");
    assert_eq!(run63_bits(20, INF, INF, 0), GENERATED, "inf - inf");
    assert_eq!(run63_bits(21, INF, NEG_INF, 0), GENERATED, "inf + -inf");
    assert_eq!(run63_bits(25, 0, 0, INF), GENERATED, "0 * inf");
    assert_eq!(
        run63_bits(22, 0, f64_bits(-1.0), 0),
        GENERATED,
        "sqrt of a negative"
    );
    assert_eq!(
        run63_bits(29, INF, f64_bits(1.0), 0),
        GENERATED,
        "fmadd with an inf * 0 product"
    );
}

#[test]
fn a_single_precision_invalid_operation_yields_the_same_generated_qnan() {
    // xo 18 fdivs, 21 fadds, 22 fsqrts.
    assert_eq!(run59_bits(18, 0, 0, 0), GENERATED, "0 / 0");
    assert_eq!(run59_bits(21, INF, NEG_INF, 0), GENERATED, "inf + -inf");
    assert_eq!(
        run59_bits(22, 0, f64_bits(-1.0), 0),
        GENERATED,
        "sqrt of a negative"
    );
}

// ----- NaN precedence on the two-operand arms -----

#[test]
fn an_add_propagates_the_fra_nan_ahead_of_the_frb_nan() {
    // xo = 21 -> fadd. Both orders are pinned so the answer cannot
    // start tracking whichever operand host codegen places first.
    assert_eq!(run63_bits(21, QNAN_C, QNAN_A, 0), QNAN_C);
    assert_eq!(run63_bits(21, QNAN_A, QNAN_C, 0), QNAN_A);
}

#[test]
fn a_multiply_picks_the_fra_nan_ahead_of_the_frc_nan() {
    // xo = 25 -> fmul.
    assert_eq!(run63_bits(25, QNAN_C, 0, QNAN_A), QNAN_C);
    assert_eq!(run63_bits(25, QNAN_A, 0, QNAN_C), QNAN_A);
}

#[test]
fn a_nan_in_a_reserved_register_field_never_reaches_frt() {
    // fmul reads FRA and FRC; fdiv, fadd and fsub read FRA and FRB.
    // The register the instruction does not name is a reserved field.
    assert_eq!(
        run63_bits(25, f64_bits(3.0), QNAN_A, f64_bits(4.0)),
        f64_bits(12.0)
    );
    assert_eq!(
        run63_bits(18, f64_bits(8.0), f64_bits(2.0), QNAN_A),
        f64_bits(4.0)
    );
    assert_eq!(
        run59_bits(25, f64_bits(3.0), QNAN_A, f64_bits(4.0)),
        f64_bits(12.0)
    );
    assert_eq!(
        run59_bits(18, f64_bits(8.0), f64_bits(2.0), QNAN_A),
        f64_bits(4.0)
    );
}

#[test]
fn fadds_propagates_the_fra_nan_at_full_double_width() {
    // Narrowing the operands to f32 first would truncate the payload.
    assert_eq!(run59_bits(21, QNAN_A, f64_bits(1.0), 0), QNAN_A);
}
