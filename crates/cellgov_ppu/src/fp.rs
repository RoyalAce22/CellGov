//! Dispatches opcode-63 (double) and opcode-59 (single) FP instructions.
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
        29 => a.mul_add(c, b).to_bits(),
        28 => a.mul_add(c, -b).to_bits(),
        // fnmadd / fnmsub = negate(fma result), but Book I 4.6.5.2
        // says QNaN sign is preserved across the negation. Rust's
        // `-nan` flips the sign bit, so guard the NaN case.
        31 => negate_unless_nan(a.mul_add(c, b)).to_bits(),
        30 => negate_unless_nan(a.mul_add(c, -b)).to_bits(),
        25 => (a * c).to_bits(),
        // fsel: FRT <- FRC if FRA >= 0.0 else FRB. NaN is "not >="
        // by IEEE semantics, so a NaN FRA falls through to FRB --
        // matching Book I 5.2.2's "if FRA is NaN then FRT <- FRB"
        // rule. The match on the f64 comparison happens to honor
        // the same sign-of-zero quirk the spec calls out.
        23 => {
            if a >= 0.0 {
                c.to_bits()
            } else {
                b.to_bits()
            }
        }
        _ => match xo {
            18 => (a / b).to_bits(),
            21 => (a + b).to_bits(),
            20 => (a - b).to_bits(),
            72 => b.to_bits(),
            40 => (-b).to_bits(),
            264 => b.abs().to_bits(),
            136 => (-b.abs()).to_bits(),
            // fcmpu / fcmpo: BF is bits 6:8 of the instruction word,
            // i.e. the top 3 bits of the 5-bit FRT field the decoder
            // passes through here. Book I 4.6.5: bit 3 of the CR
            // field is FU (unordered), set when either operand is a
            // NaN -- distinct from the integer-compare SO bit.
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
            12 => {
                let s = b as f32;
                (s as f64).to_bits()
            }
            // fctiw (xo=14) and fctiwz (xo=15). Book I 4.6.6: a NaN
            // input must produce 0x8000_0000; Rust's `f64 as i32`
            // returns 0 for NaN, so guard. ((+/-)inf clamps via
            // saturating cast already.) Result is zero-extended into
            // FRT[32:63]; FRT[0:31] is architecturally undefined.
            14 | 15 => {
                let i = if b.is_nan() { i32::MIN } else { b as i32 };
                (i as u64) & 0xFFFF_FFFF
            }
            // fctid (xo=814) and fctidz (xo=815). Same NaN guard at
            // 64-bit width.
            814 | 815 => {
                let i = if b.is_nan() { i64::MIN } else { b as i64 };
                i as u64
            }
            // fcfid: convert a 64-bit signed integer in the FPR
            // raw bits to f64. `u64 as i64` is a bitwise reinterpret
            // in Rust (not a value conversion), which is what the
            // spec wants -- the FPR contents are read as i64.
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
    // because Book I 4.3.5.1 specifies that opcode-59 results are
    // single-precision-representable; doing the math in f32 keeps
    // rounding at single-precision throughout.
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
}
