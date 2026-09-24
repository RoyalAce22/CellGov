//! Dispatches opcode-63 (double) and opcode-59 (single) FP instructions.
// [PPC-Book1 p:206 s:Appendix J] Floating-point instruction set sorted by opcode, primaries 59 and 63.
// [PPC-Book1 p:88 s:4.2.2] FPSCR bit definitions (FPRF, FR, FI, FX, OX, UX, XX, VXSNAN).
//!
//! FPSCR is not modeled: every arm here assumes `FPSCR[RN]` holds the
//! round-to-nearest encoding and the FPSCR status flags (FPRF, FR, FI,
//! FX, OX, UX, XX, VXSNAN ...) are never updated. `mffs` / `mcrfs` and
//! dot-form CR1 updates therefore see stale state.
// [PPC-Book1 p:89 s:4.2.2] FPSCR[RN] encoding: 0b00 selects Round to Nearest.
//!
//! TODO(fpscr): Plumb FPSCR through PpuState and update it from every
//! computational arm here, plus wire `mffs` / `mcrfs` and dot-form
//! CR1. Until then a guest that writes `FPSCR[RN]` to a directed mode
//! gets round-to-nearest anyway, and the `fctiw` / `fctid` arms round
//! to nearest unconditionally rather than honouring the register.

use crate::exec::ExecuteVerdict;
use crate::instruction::ops::{Fp59Op, Fp63Op};
use crate::state::PpuState;

/// Execute a primary-63 op. The FPSCR ops and the `frsqrte` estimate
/// retire as no-ops (`Continue` without a register write), matching
/// the not-modelled-FPSCR posture above; a guest reading FRT after
/// `frsqrte` therefore sees the register's prior value.
pub fn execute_fp63(
    state: &mut PpuState,
    op: Fp63Op,
    frt: u8,
    fra: u8,
    frb: u8,
    frc: u8,
) -> ExecuteVerdict {
    let a = f64::from_bits(state.fpr[fra as usize]);
    let b = f64::from_bits(state.fpr[frb as usize]);
    let c = f64::from_bits(state.fpr[frc as usize]);
    let result_bits = match op {
        // mul_add gives the IEEE fused multiply-add (single rounding);
        // `a * c + b` would round twice. `nan_resolved_bits` overrides
        // the host's NaN answer: `-b` below would otherwise hand a NaN
        // FRB to the FMA with its sign flipped, and Rust does not
        // define which NaN an FMA propagates.
        // [PPC-Book1 p:113 s:4.6.5.2] fmadd: FRT <- (FRA * FRC) + FRB, single rounding.
        Fp63Op::Fmadd => nan_resolved_bits(a.mul_add(c, b), &[a, b, c]),
        // [PPC-Book1 p:113 s:4.6.5.2] fmsub: FRT <- (FRA * FRC) - FRB.
        Fp63Op::Fmsub => nan_resolved_bits(a.mul_add(c, -b), &[a, b, c]),
        // fnmadd / fnmsub = negate(fma result), but QNaN sign is
        // preserved across the negation. Rust's `-nan` flips the
        // sign bit, so guard the NaN case.
        // [PPC-Book1 p:114 s:4.6.5.2] QNaN sign preserved across fnmadd/fnmsub negation.
        // [PPC-Book1 p:114 s:4.6.5.2] fnmadd: FRT <- -((FRA * FRC) + FRB).
        Fp63Op::Fnmadd => nan_resolved_bits(negate_unless_nan(a.mul_add(c, b)), &[a, b, c]),
        // [PPC-Book1 p:114 s:4.6.5.2] fnmsub: FRT <- -((FRA * FRC) - FRB).
        Fp63Op::Fnmsub => nan_resolved_bits(negate_unless_nan(a.mul_add(c, -b)), &[a, b, c]),
        // FRB is a reserved field here, so it is not in the operand
        // list -- a NaN parked in it must not win the precedence walk.
        // [PPC-Book1 p:112 s:4.6.5.1] fmul: FRT <- FRA * FRC.
        Fp63Op::Fmul => nan_resolved_bits(a * c, &[a, c]),
        // fsel: FRT <- FRC if FRA >= 0.0 else FRB. NaN is "not >="
        // by IEEE semantics, so a NaN FRA falls through to FRB --
        // matching the "if FRA is NaN then FRT <- FRB" rule. The
        // match on the f64 comparison happens to honor the same
        // sign-of-zero quirk the spec calls out.
        // [PPC-Book1 p:126 s:5.2.2] fsel: FRT <- FRC if FRA>=0 else FRB.
        Fp63Op::Fsel => {
            if a >= 0.0 {
                c.to_bits()
            } else {
                b.to_bits()
            }
        }
        // [PPC-Book1 p:112 s:4.6.5.1] fdiv: FRT <- FRA / FRB.
        Fp63Op::Fdiv => nan_resolved_bits(a / b, &[a, b]),
        // [PPC-Book1 p:111 s:4.6.5.1] fadd: FRT <- FRA + FRB.
        Fp63Op::Fadd => nan_resolved_bits(a + b, &[a, b]),
        // [PPC-Book1 p:111 s:4.6.5.1] fsub: FRT <- FRA - FRB.
        Fp63Op::Fsub => nan_resolved_bits(a - b, &[a, b]),
        // [PPC-Book1 p:110 s:4.6.4] fmr: FRT <- FRB (move register).
        Fp63Op::Fmr => b.to_bits(),
        // [PPC-Book1 p:110 s:4.6.4] fneg: FRT <- ~FRB[0] || FRB[1:63].
        Fp63Op::Fneg => (-b).to_bits(),
        // [PPC-Book1 p:110 s:4.6.4] fabs: FRT <- 0 || FRB[1:63].
        Fp63Op::Fabs => b.abs().to_bits(),
        // [PPC-Book1 p:110 s:4.6.4] fnabs: FRT <- 1 || FRB[1:63].
        Fp63Op::Fnabs => (-b.abs()).to_bits(),
        // fcmpu / fcmpo: BF is bits 6:8 of the instruction word,
        // i.e. the top 3 bits of the 5-bit FRT field the decoder
        // passes through here. Bit 3 of the CR field is FU
        // (unordered), set when either operand is a NaN --
        // distinct from the integer-compare SO bit.
        // [PPC-Book1 p:119 s:4.6.7] fcmpu / fcmpo: CR[BF] <- {LT,GT,EQ,FU}.
        Fp63Op::Fcmpu | Fp63Op::Fcmpo => {
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
        // [PPC-Book1 p:115 s:4.6.6] frsp: round FRB to single precision.
        Fp63Op::Frsp => {
            let s = b as f32;
            (s as f64).to_bits()
        }
        // fctiw and fctiwz are distinct rounding modes, not synonyms:
        // the `z` form truncates, the plain form follows FPSCR[RN],
        // which this module fixes at round-to-nearest-even. A NaN
        // input must produce 0x8000_0000; Rust's `f64 as i32` returns
        // 0 for NaN, so guard. ((+/-)inf clamps via saturating cast
        // already.) Result is zero-extended into FRT[32:63];
        // FRT[0:31] is architecturally undefined.
        // [PPC-Book1 p:117 s:4.6.6] fctiw rounds per FPSCR[RN]; fctiwz rounds toward zero.
        // [PPC-Book1 p:117 s:4.6.6] fctiw: result placed in FRT[32:63]; FRT[0:31] undefined.
        // [PPC-Book1 p:144 s:A.2] QNaN-operand convert-to-integer model: FRT[32:63] <- 0x8000_0000.
        Fp63Op::Fctiw | Fp63Op::Fctiwz => {
            let rounded = if matches!(op, Fp63Op::Fctiw) {
                b.round_ties_even()
            } else {
                b
            };
            let i = if rounded.is_nan() {
                i32::MIN
            } else {
                rounded as i32
            };
            (i as u64) & 0xFFFF_FFFF
        }
        // Same rounding-mode split and NaN guard at 64-bit width.
        // [PPC-Book1 p:116 s:4.6.6] fctid rounds per FPSCR[RN]; fctidz rounds toward zero.
        // [PPC-Book1 p:144 s:A.2] QNaN-operand 64-bit convert-to-integer: FRT <- 0x8000_0000_0000_0000.
        Fp63Op::Fctid | Fp63Op::Fctidz => {
            let rounded = if matches!(op, Fp63Op::Fctid) {
                b.round_ties_even()
            } else {
                b
            };
            let i = if rounded.is_nan() {
                i64::MIN
            } else {
                rounded as i64
            };
            i as u64
        }
        // fcfid: convert a 64-bit signed integer in the FPR
        // raw bits to f64. `u64 as i64` is a bitwise reinterpret
        // in Rust (not a value conversion), which is what the
        // spec wants -- the FPR contents are read as i64.
        // [PPC-Book1 p:118 s:4.6.6] fcfid: convert 64-bit signed integer to FP.
        Fp63Op::Fcfid => {
            let i = state.fpr[frb as usize] as i64;
            (i as f64).to_bits()
        }
        // The square root of a negative operand is an Invalid
        // Operation, and the host hands back a sign-bit-set QNaN that
        // the architecture does not allow.
        // [PPC-Book1 p:125 s:5.2.1.1] fsqrt: FRT <- square root of FRB.
        Fp63Op::Fsqrt => nan_resolved_bits(b.sqrt(), &[b]),
        Fp63Op::Frsqrte
        | Fp63Op::Mtfsb1
        | Fp63Op::Mcrfs
        | Fp63Op::Mtfsb0
        | Fp63Op::Mtfsfi
        | Fp63Op::Mffs
        | Fp63Op::Mtfsf => return ExecuteVerdict::Continue,
    };
    state.set_fpr(frt as usize, result_bits);
    ExecuteVerdict::Continue
}

/// Execute a primary-59 op. `fres` retires as a no-op.
pub fn execute_fp59(
    state: &mut PpuState,
    op: Fp59Op,
    frt: u8,
    fra: u8,
    frb: u8,
    frc: u8,
) -> ExecuteVerdict {
    // Single-precision arithmetic. Inputs are pre-converted to f32
    // because opcode-59 results must be single-precision-representable;
    // doing the math in f32 keeps rounding at single-precision
    // throughout.
    // [PPC-Book1 p:206 s:Appendix J] Primary 59 holds fdivs, fsubs, fadds, fsqrts, fres, fmuls, fmsubs, fmadds, fnmsubs, fnmadds.
    // [PPC-Book1 p:93 s:4.3.5.1] Single-precision arithmetic: all input values must be representable in single format.
    let a64 = f64::from_bits(state.fpr[fra as usize]);
    let b64 = f64::from_bits(state.fpr[frb as usize]);
    let c64 = f64::from_bits(state.fpr[frc as usize]);
    // A NaN operand is stored in FRT as-is, at full double width --
    // only frsp truncates the propagated NaN's low-order bits -- so the
    // precedence walk runs on the register values, not on the f32
    // narrowings below. Each arm lists only its architected operands;
    // FRB in fmuls and FRC in the others are reserved instruction
    // fields, and a NaN parked in one must not win the walk.
    // [PPC-Book1 p:91 s:4.3.2] Propagated NaN stored unchanged; only frsp clears the low 29 bits.
    let operands: &[f64] = match op {
        Fp59Op::Fmadds | Fp59Op::Fmsubs | Fp59Op::Fnmadds | Fp59Op::Fnmsubs => &[a64, b64, c64],
        Fp59Op::Fmuls => &[a64, c64],
        Fp59Op::Fdivs | Fp59Op::Fadds | Fp59Op::Fsubs => &[a64, b64],
        Fp59Op::Fsqrts => &[b64],
        Fp59Op::Fres => &[],
    };
    let a = a64 as f32;
    let b = b64 as f32;
    let c = c64 as f32;
    let result = match op {
        Fp59Op::Fmadds => a.mul_add(c, b),
        Fp59Op::Fmsubs => a.mul_add(c, -b),
        Fp59Op::Fnmadds => negate_unless_nan_f32(a.mul_add(c, b)),
        Fp59Op::Fnmsubs => negate_unless_nan_f32(a.mul_add(c, -b)),
        Fp59Op::Fmuls => a * c,
        Fp59Op::Fdivs => a / b,
        Fp59Op::Fadds => a + b,
        Fp59Op::Fsubs => a - b,
        // [PPC-Book1 p:125 s:5.2.1.1] fsqrts: FRT <- square root of FRB, single precision.
        Fp59Op::Fsqrts => b.sqrt(),
        Fp59Op::Fres => return ExecuteVerdict::Continue,
    };
    // `result as f64` is only reached on a non-NaN result: a NaN one is
    // answered from the operands or by the generated QNaN, so widening
    // never has an f32 NaN payload to lose.
    let bits = nan_resolved_bits(result as f64, operands);
    state.set_fpr(frt as usize, bits);
    ExecuteVerdict::Continue
}

use cellgov_ps3_abi::hw::ppc_isa::{PPC_F64_QUIET_BIT, PPC_GENERATED_QNAN_F64};

/// Result bits for a computational arm, with the architected NaN
/// answer substituted for the host's whenever the result is a NaN.
///
/// `operands` lists the instruction's architected inputs in
/// FRA-before-FRB-before-FRC order. A reserved instruction field is
/// not an operand and must be left out, or a NaN sitting in the
/// unused register would win the precedence walk.
///
/// Both substitutions are observable on this host: an x86 FMA answers
/// `fma(1, FRC_nan, FRB_nan)` with the FRC NaN where the architecture
/// names FRB, and every invalid operation there yields a sign-bit-set
/// QNaN. The two-operand arms happen to agree today; routing them
/// through here stops the answer depending on codegen.
// [PPC-Book1 p:91 s:4.3.2] NaN-operand result: FRA, else FRB, else FRC, stored with the high-order fraction bit set.
// [PPC-Book1 p:114 s:4.6.5.2] A generated QNaN has sign 0; an SNaN quieted in its place keeps the SNaN's sign.
#[inline]
fn nan_resolved_bits(result: f64, operands: &[f64]) -> u64 {
    if let Some(first) = operands.iter().copied().find(|v| v.is_nan()) {
        return first.to_bits() | PPC_F64_QUIET_BIT;
    }
    if result.is_nan() {
        return PPC_GENERATED_QNAN_F64;
    }
    result.to_bits()
}

/// Negate `r` unless it is NaN. PPC fnmadd / fnmsub define the
/// negation as preserving the QNaN sign bit; Rust's unary `-` on a
/// NaN flips it.
// [PPC-Book1 p:114 s:4.6.5.2] fnmadd / fnmsub: QNaNs propagate with no effect on their sign bit.
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
