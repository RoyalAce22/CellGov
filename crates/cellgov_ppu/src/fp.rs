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
#[path = "tests/fp_tests.rs"]
mod tests;
