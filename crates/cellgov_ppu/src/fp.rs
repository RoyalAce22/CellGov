//! Dispatches opcode-63 (double) and opcode-59 (single) FP instructions.
// [PPC-Book1 p:84 s:4.6] Floating-point processor instruction set, opcodes 59/63.
// [PPC-Book1 p:74 s:4.2.2] FPSCR definition (FPRF, FR, FI, FX, OX, UX, XX, VXSNAN).
//!
//! FPSCR is not modeled: rounding mode defaults to round-to-nearest-
//! even (Rust's native cast / arithmetic semantics) and the FPSCR
//! status flags (FPRF, FR, FI, FX, OX, UX, XX, VXSNAN ...) are never
//! updated. `mffs` / `mcrfs` and dot-form CR1 updates therefore see
//! stale state. The convert-to-int arms degenerate to round-toward-zero
//! across both `fctiw` / `fctiwz` and `fctid` / `fctidz` for that reason.
//!
//! TODO(fpscr): Plumb FPSCR through PpuState and update it from every
//! computational arm here, plus wire `mffs` / `mcrfs` and dot-form
//! CR1. First-divergence point against RPCS3 baselines is most likely
//! `fctiw` (round-to-nearest-even vs. our truncation at half-bit
//! boundaries) since firmware uses it without the `z` suffix; that's
//! where the current round-toward-zero approximation will bite first.

use crate::exec::ExecuteVerdict;
use crate::state::PpuState;

pub fn execute_fp63(
    state: &mut PpuState,
    xo: u16,
    frt: u8,
    fra: u8,
    frb: u8,
    frc: u8,
) -> ExecuteVerdict {
    let a = f64::from_bits(state.fpr[fra as usize]);
    let b = f64::from_bits(state.fpr[frb as usize]);
    let c = f64::from_bits(state.fpr[frc as usize]);
    let xo5 = xo & 0x1F;
    let result_bits = match xo5 {
        // mul_add gives the IEEE fused multiply-add (single rounding);
        // `a * c + b` would round twice.
        // [PPC-Book1 p:107 s:4.6.6] fmadd: FRT <- (FRA * FRC) + FRB, single rounding.
        29 => a.mul_add(c, b).to_bits(),
        // [PPC-Book1 p:107 s:4.6.6] fmsub: FRT <- (FRA * FRC) - FRB.
        28 => a.mul_add(c, -b).to_bits(),
        // fnmadd / fnmsub = negate(fma result), but QNaN sign is
        // preserved across the negation. Rust's `-nan` flips the
        // sign bit, so guard the NaN case.
        // [PPC-Book1 p:97 s:4.6.5.2] QNaN sign preserved across fnmadd/fnmsub negation.
        // [PPC-Book1 p:107 s:4.6.6] fnmadd: FRT <- -((FRA * FRC) + FRB).
        31 => negate_unless_nan(a.mul_add(c, b)).to_bits(),
        // [PPC-Book1 p:107 s:4.6.6] fnmsub: FRT <- -((FRA * FRC) - FRB).
        30 => negate_unless_nan(a.mul_add(c, -b)).to_bits(),
        // [PPC-Book1 p:106 s:4.6.6] fmul: FRT <- FRA * FRC.
        25 => (a * c).to_bits(),
        // fsel: FRT <- FRC if FRA >= 0.0 else FRB. NaN is "not >="
        // by IEEE semantics, so a NaN FRA falls through to FRB --
        // matching the "if FRA is NaN then FRT <- FRB" rule. The
        // match on the f64 comparison happens to honor the same
        // sign-of-zero quirk the spec calls out.
        // [PPC-Book1 p:108 s:4.6.7] fsel: FRT <- FRC if FRA>=0 else FRB.
        23 => {
            if a >= 0.0 {
                c.to_bits()
            } else {
                b.to_bits()
            }
        }
        _ => match xo {
            // [PPC-Book1 p:106 s:4.6.6] fdiv: FRT <- FRA / FRB.
            18 => (a / b).to_bits(),
            // [PPC-Book1 p:106 s:4.6.6] fadd: FRT <- FRA + FRB.
            21 => (a + b).to_bits(),
            // [PPC-Book1 p:106 s:4.6.6] fsub: FRT <- FRA - FRB.
            20 => (a - b).to_bits(),
            // [PPC-Book1 p:111 s:4.6.8] fmr: FRT <- FRB (move register).
            72 => b.to_bits(),
            // [PPC-Book1 p:111 s:4.6.8] fneg: FRT <- ~FRB[0] || FRB[1:63].
            40 => (-b).to_bits(),
            // [PPC-Book1 p:111 s:4.6.8] fabs: FRT <- 0 || FRB[1:63].
            264 => b.abs().to_bits(),
            // [PPC-Book1 p:111 s:4.6.8] fnabs: FRT <- 1 || FRB[1:63].
            136 => (-b.abs()).to_bits(),
            // fcmpu / fcmpo: BF is bits 6:8 of the instruction word,
            // i.e. the top 3 bits of the 5-bit FRT field the decoder
            // passes through here. Bit 3 of the CR field is FU
            // (unordered), set when either operand is a NaN --
            // distinct from the integer-compare SO bit.
            // [PPC-Book1 p:104 s:4.6.5] fcmpu (xo=0) / fcmpo (xo=32): CR[BF] <- {LT,GT,EQ,FU}.
            0 | 32 => {
                let bf = (frt >> 2) & 7;
                let cr_val = if a.is_nan() || b.is_nan() {
                    0b0001
                } else if a < b {
                    0b1000
                } else if a > b {
                    0b0100
                } else {
                    0b0010
                };
                state.set_cr_field(bf, cr_val);
                return ExecuteVerdict::Continue;
            }
            // [PPC-Book1 p:103 s:4.6.4] frsp: round FRB to single precision.
            12 => {
                let s = b as f32;
                (s as f64).to_bits()
            }
            // fctiw (xo=14) and fctiwz (xo=15). A NaN input must
            // produce 0x8000_0000; Rust's `f64 as i32` returns 0 for
            // NaN, so guard. ((+/-)inf clamps via saturating cast
            // already.) Result is zero-extended into FRT[32:63];
            // FRT[0:31] is architecturally undefined.
            // [PPC-Book1 p:103 s:4.6.4] fctiw / fctiwz: convert FP to 32-bit signed integer.
            // [PPC-Book1 p:117 s:4.6.6] fctiw: result placed in FRT[32:63]; FRT[0:31] undefined.
            // [PPC-Book1 p:144 s:A.2] QNaN-operand convert-to-integer model: FRT[32:63] <- 0x8000_0000.
            14 | 15 => {
                let i = if b.is_nan() { i32::MIN } else { b as i32 };
                (i as u64) & 0xFFFF_FFFF
            }
            // fctid (xo=814) and fctidz (xo=815). Same NaN guard at
            // 64-bit width.
            // [PPC-Book1 p:103 s:4.6.4] fctid / fctidz: convert FP to 64-bit signed integer.
            // [PPC-Book1 p:144 s:A.2] QNaN-operand 64-bit convert-to-integer: FRT <- 0x8000_0000_0000_0000.
            814 | 815 => {
                let i = if b.is_nan() { i64::MIN } else { b as i64 };
                i as u64
            }
            // fcfid: convert a 64-bit signed integer in the FPR
            // raw bits to f64. `u64 as i64` is a bitwise reinterpret
            // in Rust (not a value conversion), which is what the
            // spec wants -- the FPR contents are read as i64.
            // [PPC-Book1 p:103 s:4.6.4] fcfid: convert 64-bit signed integer to FP.
            846 => {
                let i = state.fpr[frb as usize] as i64;
                (i as f64).to_bits()
            }
            _ => return ExecuteVerdict::Continue,
        },
    };
    state.fpr[frt as usize] = result_bits;
    ExecuteVerdict::Continue
}

pub fn execute_fp59(
    state: &mut PpuState,
    xo: u16,
    frt: u8,
    fra: u8,
    frb: u8,
    frc: u8,
) -> ExecuteVerdict {
    // Single-precision arithmetic. Inputs are pre-converted to f32
    // because opcode-59 results must be single-precision-representable;
    // doing the math in f32 keeps rounding at single-precision
    // throughout.
    // [PPC-Book1 p:84 s:4.6] Opcode-59 single-precision arithmetic: fadds, fsubs, fmuls, fdivs, fmadds, fmsubs, fnmadds, fnmsubs.
    // [PPC-Book1 p:93 s:4.3.5.1] Single-precision arithmetic: all input values must be representable in single format.
    let a = f64::from_bits(state.fpr[fra as usize]) as f32;
    let b = f64::from_bits(state.fpr[frb as usize]) as f32;
    let c = f64::from_bits(state.fpr[frc as usize]) as f32;
    let xo5 = xo & 0x1F;
    let result = match xo5 {
        29 => a.mul_add(c, b),
        28 => a.mul_add(c, -b),
        31 => negate_unless_nan_f32(a.mul_add(c, b)),
        30 => negate_unless_nan_f32(a.mul_add(c, -b)),
        25 => a * c,
        _ => match xo {
            18 => a / b,
            21 => a + b,
            20 => a - b,
            _ => return ExecuteVerdict::Continue,
        },
    };
    state.fpr[frt as usize] = (result as f64).to_bits();
    ExecuteVerdict::Continue
}

/// Negate `r` unless it is NaN. PPC fnmadd / fnmsub define the
/// negation as preserving the QNaN sign bit; Rust's unary `-` on a
/// NaN flips it.
// [PPC-Book1 p:114 s:4.6.6] fnmadd / fnmsub: QNaNs propagate with no effect on their sign bit.
#[inline]
fn negate_unless_nan(r: f64) -> f64 {
    if r.is_nan() {
        r
    } else {
        -r
    }
}

#[inline]
fn negate_unless_nan_f32(r: f32) -> f32 {
    if r.is_nan() {
        r
    } else {
        -r
    }
}

#[cfg(test)]
mod tests {
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
}
