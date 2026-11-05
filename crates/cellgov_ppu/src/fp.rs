//! Dispatches opcode-63 (double) and opcode-59 (single) FP instructions.
// [PPC-Book1 p:206 s:Appendix J] Floating-point instruction set sorted by opcode, primaries 59 and 63.
// [PPC-Book1 p:88 s:4.2.2] FPSCR bit definitions (FPRF, FR, FI, FX, OX, UX, XX, VXSNAN).
//!
//! The FPSCR move instructions read and write the register
//! ([`crate::fpscr`]), and a record form copies FPSCR bits 0:3 to CR1.
//! The arithmetic arms, the multiply-add forms included, compute on the
//! `cellgov_float` engine in both widths. The engine rounds in the mode
//! `FPSCR[RN]` selects, with every exception disabled. The other arms
//! write the FPSCR as follows:
//!
//! - the estimates write the status [`crate::fp_status`] gives them;
//! - the compares write FPCC and their invalid causes through
//!   [`crate::fpscr::write_compare_status`];
//! - the conversions, `fsel` and the moves leave the FPSCR unchanged.
// [PPC-Book1 p:89 s:4.2.2] FPSCR[RN] encoding: 0b00 selects Round to Nearest.

use cellgov_float::{
    ppc_add, ppc_div, ppc_div_sqrt_precision, ppc_madd, ppc_msub, ppc_mul, ppc_nmadd, ppc_nmsub,
    ppc_sqrt, ppc_sub, PpcPrecision, PpcResult,
};

use crate::exec::ExecuteVerdict;
use crate::fp_status::{
    compare_fpcc, compare_invalid, fres_status, frsqrte_status, fsel_takes_frc,
};
use crate::fpscr::{self, write_status, Fprf};
use crate::instruction::ops::{Fp59Op, Fp63Op};
use crate::state::PpuState;

/// Writes an engine `result` of `precision` to FRT and its status to the
/// FPSCR.
fn write_engine_result(
    state: &mut PpuState,
    frt: u8,
    result: PpcResult,
    precision: PpcPrecision,
) -> ExecuteVerdict {
    state.set_fpr(frt as usize, result.bits);
    let fprf = Fprf::of(result.bits, precision);
    write_status(state, &result.status, fprf);
    ExecuteVerdict::Continue
}

/// Execute a primary-63 op.
pub fn execute_fp63(
    state: &mut PpuState,
    op: Fp63Op,
    frt: u8,
    fra: u8,
    frb: u8,
    frc: u8,
) -> ExecuteVerdict {
    let (ra, rb, rc) = (
        state.fpr[fra as usize],
        state.fpr[frb as usize],
        state.fpr[frc as usize],
    );
    let b = f64::from_bits(rb);
    let result_bits = match op {
        // [PPC-Book1 p:113 s:4.6.5.2] fmadd: FRT <- (FRA * FRC) + FRB, single rounding.
        // [PPC-Book1 p:113 s:4.6.5.2] fmsub: FRT <- (FRA * FRC) - FRB.
        // [PPC-Book1 p:114 s:4.6.5.2] fnmadd: FRT <- -((FRA * FRC) + FRB).
        // [PPC-Book1 p:114 s:4.6.5.2] fnmsub: FRT <- -((FRA * FRC) - FRB).
        // [PPC-Book1 p:114 s:4.6.5.2] QNaN sign preserved across fnmadd/fnmsub negation.
        Fp63Op::Fmadd | Fp63Op::Fmsub | Fp63Op::Fnmadd | Fp63Op::Fnmsub => {
            let rounding = fpscr::rounding(state.fpscr());
            let multiply_add = match op {
                Fp63Op::Fmadd => ppc_madd,
                Fp63Op::Fmsub => ppc_msub,
                Fp63Op::Fnmadd => ppc_nmadd,
                _ => ppc_nmsub,
            };
            let result = multiply_add(PpcPrecision::Double, rounding, ra, rb, rc);
            return write_engine_result(state, frt, result, PpcPrecision::Double);
        }
        // FRB is a reserved field here, so it is not an operand: a NaN
        // parked in it does not win the NaN selection.
        // [PPC-Book1 p:112 s:4.6.5.1] fmul: FRT <- FRA * FRC.
        Fp63Op::Fmul => {
            let rounding = fpscr::rounding(state.fpscr());
            let result = ppc_mul(PpcPrecision::Double, rounding, ra, rc);
            return write_engine_result(state, frt, result, PpcPrecision::Double);
        }
        // [PPC-Book1 p:126 s:5.2.2] fsel: FRT <- FRC if FRA >= 0 else FRB; it leaves the FPSCR unchanged.
        Fp63Op::Fsel => {
            if fsel_takes_frc(ra) {
                rc
            } else {
                rb
            }
        }
        // [PPC-Book1 p:112 s:4.6.5.1] fdiv: FRT <- FRA / FRB.
        Fp63Op::Fdiv => {
            let rounding = fpscr::rounding(state.fpscr());
            let result = ppc_div(PpcPrecision::Double, rounding, ra, rb);
            return write_engine_result(state, frt, result, PpcPrecision::Double);
        }
        // [PPC-Book1 p:111 s:4.6.5.1] fadd: FRT <- FRA + FRB.
        Fp63Op::Fadd => {
            let rounding = fpscr::rounding(state.fpscr());
            let result = ppc_add(PpcPrecision::Double, rounding, ra, rb);
            return write_engine_result(state, frt, result, PpcPrecision::Double);
        }
        // [PPC-Book1 p:111 s:4.6.5.1] fsub: FRT <- FRA - FRB.
        Fp63Op::Fsub => {
            let rounding = fpscr::rounding(state.fpscr());
            let result = ppc_sub(PpcPrecision::Double, rounding, ra, rb);
            return write_engine_result(state, frt, result, PpcPrecision::Double);
        }
        // [PPC-Book1 p:110 s:4.6.4] the moves copy FRB with bit 0 kept, flipped, cleared or set, a NaN like any value, and leave the FPSCR unchanged.
        Fp63Op::Fmr => rb,
        Fp63Op::Fneg => rb ^ PPC_F64_SIGN_BIT,
        Fp63Op::Fabs => rb & !PPC_F64_SIGN_BIT,
        Fp63Op::Fnabs => rb | PPC_F64_SIGN_BIT,
        // BF is instruction bits 6:8, the top three bits of the FRT
        // field the decoder passes here.
        // [PPC-Book1 p:119 s:4.6.7] fcmpu / fcmpo set CR field BF and FPCC to one of FL, FG, FE, FU.
        Fp63Op::Fcmpu | Fp63Op::Fcmpo => {
            let fpcc = compare_fpcc(ra, rb);
            state.set_cr_field((frt >> 2) & 7, fpcc);
            let ve = state.fpscr() & PPC_FPSCR_VE != 0;
            let invalid = compare_invalid(ra, rb, op == Fp63Op::Fcmpo, ve);
            fpscr::write_compare_status(state, fpcc, &invalid);
            return ExecuteVerdict::Continue;
        }
        // [PPC-Book1 p:115 s:4.6.6] frsp: round FRB to single precision.
        Fp63Op::Frsp => {
            let s = b as f32;
            (s as f64).to_bits()
        }
        // fctiw and fctiwz are distinct rounding modes, not synonyms:
        // the `z` form truncates, the plain form follows FPSCR[RN],
        // and this arm rounds to nearest even in every mode. A NaN
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
        // [PPC-Book1 p:125 s:5.2.1.1] fsqrt: FRT <- square root of FRB.
        Fp63Op::Fsqrt => {
            let rounding = fpscr::rounding(state.fpscr());
            let result = ppc_sqrt(PpcPrecision::Double, rounding, rb);
            return write_engine_result(state, frt, result, PpcPrecision::Double);
        }
        // [PPC-Book1 p:126 s:5.2.1.1] frsqrte: an estimate within one part in 32 of 1/sqrt(FRB); the special-operand table fixes -inf and < 0 to QNaN, -0 to -inf, +0 to +inf, +inf to +0.
        // The PPE's own table, read back by the ppu_fp_estimate_readout capture.
        Fp63Op::Frsqrte => {
            let bits = cellgov_float::frsqrte_estimate(rb);
            state.set_fpr(frt as usize, bits);
            let fprf = Fprf::of(bits, PpcPrecision::Double);
            write_status(state, &frsqrte_status(rb), fprf);
            return ExecuteVerdict::Continue;
        }
        Fp63Op::Mffs => {
            fpscr::mffs(state, frt);
            return ExecuteVerdict::Continue;
        }
        Fp63Op::Mcrfs => {
            fpscr::mcrfs(state, frt, fra);
            return ExecuteVerdict::Continue;
        }
        Fp63Op::Mtfsfi => {
            fpscr::mtfsfi(state, frt, frb);
            return ExecuteVerdict::Continue;
        }
        Fp63Op::Mtfsf => {
            fpscr::mtfsf(state, frt, fra, frb);
            return ExecuteVerdict::Continue;
        }
        Fp63Op::Mtfsb0 => {
            fpscr::mtfsb0(state, frt);
            return ExecuteVerdict::Continue;
        }
        Fp63Op::Mtfsb1 => {
            fpscr::mtfsb1(state, frt);
            return ExecuteVerdict::Continue;
        }
    };
    state.set_fpr(frt as usize, result_bits);
    ExecuteVerdict::Continue
}

/// The CR1 update of a floating-point record form (Rc=1).
pub fn record_cr1(state: &mut PpuState) {
    state.set_cr_field(1, fpscr::cr1_nibble(state.fpscr()));
}

/// Execute a primary-59 op.
pub fn execute_fp59(
    state: &mut PpuState,
    op: Fp59Op,
    frt: u8,
    fra: u8,
    frb: u8,
    frc: u8,
) -> ExecuteVerdict {
    // The engine computes on the double-format register values and
    // rounds once to single precision.
    // [PPC-Book1 p:206 s:Appendix J] Primary 59 holds fdivs, fsubs, fadds, fsqrts, fres, fmuls, fmsubs, fmadds, fnmsubs, fnmadds.
    // [PPC-Book1 p:93 s:4.3.5.1] Single-precision arithmetic: all input values must be representable in single format.
    // [PPC-Book1 p:91 s:4.3.2] Propagated NaN stored unchanged; only frsp clears the low 29 bits.
    // [PPC-Book1 p:91 s:4.3.2] NaN-operand result: FRA, else FRB, else FRC, stored with the high-order fraction bit set.
    let (ra, rb, rc) = (
        state.fpr[fra as usize],
        state.fpr[frb as usize],
        state.fpr[frc as usize],
    );
    let rounding = fpscr::rounding(state.fpscr());
    let single = PpcPrecision::Single;
    let (result, precision) = match op {
        Fp59Op::Fmadds => (ppc_madd(single, rounding, ra, rb, rc), single),
        Fp59Op::Fmsubs => (ppc_msub(single, rounding, ra, rb, rc), single),
        Fp59Op::Fnmadds => (ppc_nmadd(single, rounding, ra, rb, rc), single),
        Fp59Op::Fnmsubs => (ppc_nmsub(single, rounding, ra, rb, rc), single),
        Fp59Op::Fmuls => (ppc_mul(single, rounding, ra, rc), single),
        Fp59Op::Fadds => (ppc_add(single, rounding, ra, rb), single),
        Fp59Op::Fsubs => (ppc_sub(single, rounding, ra, rb), single),
        // An operand outside the single exponent range makes the engine
        // round to double precision. The status and FPRF then use double
        // precision.
        Fp59Op::Fdivs => {
            let precision = ppc_div_sqrt_precision(single, &[ra, rb]);
            (ppc_div(precision, rounding, ra, rb), precision)
        }
        // [PPC-Book1 p:125 s:5.2.1.1] fsqrts: FRT <- square root of FRB, single precision.
        Fp59Op::Fsqrts => {
            let precision = ppc_div_sqrt_precision(single, &[rb]);
            (ppc_sqrt(precision, rounding, rb), precision)
        }
        // [PPC-Book1 p:125 s:5.2.1.1] fres: an estimate within one part in 256 of 1/FRB; the special-operand table fixes -inf to -0, -0 to -inf, +0 to +inf, +inf to +0.
        // The PPE's own table, read back by the ppu_fp_estimate_readout capture.
        Fp59Op::Fres => {
            let bits = cellgov_float::fres_estimate(rb);
            state.set_fpr(frt as usize, bits);
            let fprf = Fprf::of(bits, single);
            write_status(state, &fres_status(rb, bits), fprf);
            return ExecuteVerdict::Continue;
        }
    };
    write_engine_result(state, frt, result, precision)
}

use cellgov_ps3_abi::hw::ppc_isa::{PPC_F64_SIGN_BIT, PPC_FPSCR_VE};

#[cfg(test)]
#[path = "tests/fp_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/fp_console_tests.rs"]
mod fp_console_tests;

#[cfg(test)]
#[path = "tests/fp_estimate_tests.rs"]
mod fp_estimate_tests;

#[cfg(test)]
#[path = "tests/fp_estimate_console_tests.rs"]
mod fp_estimate_console_tests;

#[cfg(test)]
#[path = "tests/fp_writes_tests.rs"]
mod fp_writes_tests;

#[cfg(test)]
#[path = "tests/fp_format_facts_tests.rs"]
mod fp_format_facts_tests;
