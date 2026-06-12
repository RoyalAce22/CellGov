//! FP59/FP63 floating-point execution, including fused multiply-add precision.

use super::*;

fn f64_bits(v: f64) -> u64 {
    v.to_bits()
}

fn run63(xo: u16, fra_v: f64, frb_v: f64, frc_v: f64) -> f64 {
    let mut s = PpuState::new();
    s.fpr[1] = f64_bits(fra_v);
    s.fpr[2] = f64_bits(frb_v);
    s.fpr[3] = f64_bits(frc_v);
    execute_fp63(&mut s, xo, 0, 1, 2, 3);
    f64::from_bits(s.fpr[0])
}

fn run59(xo: u16, fra_v: f64, frb_v: f64, frc_v: f64) -> f64 {
    let mut s = PpuState::new();
    s.fpr[1] = f64_bits(fra_v);
    s.fpr[2] = f64_bits(frb_v);
    s.fpr[3] = f64_bits(frc_v);
    execute_fp59(&mut s, xo, 0, 1, 2, 3);
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
    s.fpr[2] = nan_bits;
    // xo = 15 -> fctiwz
    execute_fp63(&mut s, 15, 0, 0, 2, 0);
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
    s.fpr[0] = 0xDEAD_BEEFu64;
    s.fpr[2] = f64::to_bits(42.0);
    execute_fp63(&mut s, 14, 0, 0, 2, 0);
    assert_eq!(s.fpr[0] & 0xFFFF_FFFF, 42);
}

#[test]
fn fctidz_nan_input_produces_min_int64() {
    let mut s = PpuState::new();
    s.fpr[2] = f64::NAN.to_bits();
    // xo = 815 -> fctidz
    execute_fp63(&mut s, 815, 0, 0, 2, 0);
    assert_eq!(s.fpr[0], 0x8000_0000_0000_0000);
}

#[test]
fn fctid_xo814_dispatches() {
    let mut s = PpuState::new();
    s.fpr[0] = 0xDEAD_BEEFu64;
    s.fpr[2] = f64::to_bits(42.0);
    execute_fp63(&mut s, 814, 0, 0, 2, 0);
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
    s.fpr[2] = payload;
    execute_fp63(&mut s, 72, 0, 0, 2, 0);
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
    s.fpr[2] = (-3i64) as u64;
    execute_fp63(&mut s, 846, 0, 0, 2, 0);
    assert_eq!(f64::from_bits(s.fpr[0]), -3.0);
}

#[test]
fn fcmpu_finite_lt_sets_lt_bit() {
    // xo = 0 -> fcmpu. BF = (FRT >> 2) & 7.
    let mut s = PpuState::new();
    s.fpr[1] = f64_bits(1.0);
    s.fpr[2] = f64_bits(2.0);
    // FRT = 0 -> BF = 0; 1.0 < 2.0 -> LT bit (0b1000) in CR0.
    execute_fp63(&mut s, 0, 0, 1, 2, 0);
    assert_eq!(s.cr_field(0), 0b1000);
}

#[test]
fn fcmpu_handles_nan_unordered() {
    // NaN vs anything -> FU bit (0b0001) in target CR field.
    let mut s = PpuState::new();
    s.fpr[1] = f64::NAN.to_bits();
    s.fpr[2] = f64_bits(0.0);
    execute_fp63(&mut s, 0, 0, 1, 2, 0);
    assert_eq!(s.cr_field(0), 0b0001);
}

#[test]
fn fcmpo_writes_to_bf_from_frt_high_bits() {
    // FRT = 0b01100 -> BF = top 3 bits = 0b011 = 3.
    // 5.0 > 3.0 -> GT bit (0b0100) in CR3.
    let mut s = PpuState::new();
    s.fpr[1] = f64_bits(5.0);
    s.fpr[2] = f64_bits(3.0);
    execute_fp63(&mut s, 32, 0b01100, 1, 2, 0);
    assert_eq!(s.cr_field(3), 0b0100);
    // CR0 untouched.
    assert_eq!(s.cr_field(0), 0);
}

#[test]
fn fcmpu_finite_equal_sets_eq_bit() {
    let mut s = PpuState::new();
    s.fpr[1] = f64_bits(2.5);
    s.fpr[2] = f64_bits(2.5);
    execute_fp63(&mut s, 0, 0, 1, 2, 0);
    assert_eq!(s.cr_field(0), 0b0010);
}

#[test]
fn fctiw_rounds_toward_zero() {
    // Convert 3.7 to i32: round-toward-zero (FPSCR unmodeled) -> 3.
    let mut s = PpuState::new();
    s.fpr[2] = f64_bits(3.7);
    execute_fp63(&mut s, 14, 0, 0, 2, 0);
    assert_eq!(s.fpr[0] & 0xFFFF_FFFF, 3);
}

#[test]
fn fctid_converts_negative_double_to_int64() {
    // -42.9 -> -42 (round-toward-zero, sign-extended into 64 bits).
    let mut s = PpuState::new();
    s.fpr[2] = f64_bits(-42.9);
    execute_fp63(&mut s, 814, 0, 0, 2, 0);
    assert_eq!(s.fpr[0] as i64, -42);
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
