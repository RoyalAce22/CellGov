//! Dispatches opcode-63 (double) and opcode-59 (single) FP instructions.
// [PPC-Book1 p:206 s:Appendix J] Floating-point instruction set sorted by opcode, primaries 59 and 63.
// [PPC-Book1 p:88 s:4.2.2] FPSCR bit definitions (FPRF, FR, FI, FX, OX, UX, XX, VXSNAN).
//!
//! The FPSCR move instructions read and write the register
//! ([`crate::fpscr`]), and a record form copies FPSCR bits 0:3 to CR1.
//! The arithmetic arms, the multiply-add forms included, and the
//! rounding and conversion arms compute on the `cellgov_float` engine.
//! The engine rounds in the mode `FPSCR[RN]` selects, and `fctiwz` and
//! `fctidz` round toward zero. It applies the overflow and underflow
//! enables. When VE or ZE is set and its exception occurs, the arm leaves
//! FRT and FPRF unchanged. The other arms write the FPSCR as follows:
//!
//! - the estimates write the status [`crate::fp_status`] gives them,
//!   and VE and ZE suppress their write in the same way;
//! - the compares write FPCC and their invalid causes through
//!   [`crate::fpscr::write_compare_status`];
//! - `fsel` and the moves leave the FPSCR unchanged.
//!
//! An enabled exception sets FEX and raises no interrupt: the guest runs
//! in Ignore Exceptions mode (`MSR[FE0:FE1]` = 0, as `PPU_USER_MSR`
//! declares), so every arm continues. This holds for an FPSCR move that
//! sets an exception bit and its enable bit together. The
//! ppu_fp_enabled_results and ppu_fpscr_enabled_moves console captures
//! run every enabled case to completion. The ppu_user_msr capture runs
//! one enabled case to completion on the main thread and on a created
//! thread.
// [PPC-Book1 p:95 s:4.4] in Ignore Exceptions mode the system floating-point enabled exception handler is not invoked.
// [PPC-Book1 p:96 s:4.4] FE0 and FE1 do not change whether a result is stored or its value; the FPSCR enable bits decide both.
// [PPC-Book1 p:89 s:4.2.2] FPSCR[RN] encoding: 0b00 selects Round to Nearest.

use cellgov_float::{
    ppc_add, ppc_convert_to_integer, ppc_div, ppc_div_sqrt_precision, ppc_from_integer, ppc_madd,
    ppc_msub, ppc_mul, ppc_nmadd, ppc_nmsub, ppc_round_to_single, ppc_sqrt, ppc_sub, PpcEnables,
    PpcInteger, PpcPrecision, PpcResult, Rounding,
};
use cellgov_ps3_abi::hw::ppu::PPU_USER_MSR;

use crate::exec::ExecuteVerdict;
use crate::fp_status::{
    compare_fpcc, compare_invalid, fres_status, frsqrte_status, fsel_takes_frc,
};
use crate::fpscr::{self, write_status, Fprf};
use crate::instruction::ops::{Fp59Op, Fp63Op};
use crate::state::PpuState;

// Every arm executes, and none raises an enabled-exception interrupt.
const _: () = assert!(
    PPU_USER_MSR.fp && !PPU_USER_MSR.fe0 && !PPU_USER_MSR.fe1,
    "the FP arms implement FP available in Ignore Exceptions mode only"
);

/// Writes an engine `result` to FRT and its status to the FPSCR.
///
/// FPRF classifies a scaled overflow or underflow result at double
/// precision.
///
/// [PPC-Book1 p:97 s:4.4.1] with VE = 1 an invalid operation leaves FRT and FPRF unchanged and sets FR and FI to zero.
/// [PPC-Book1 p:98 s:4.4.2] with ZE = 1 a zero divide leaves FRT and FPRF unchanged and sets FR and FI to zero.
fn write_engine_result(
    state: &mut PpuState,
    frt: u8,
    result: PpcResult,
    precision: PpcPrecision,
    enables: PpcEnables,
) -> ExecuteVerdict {
    if result.status.refusal(enables).is_some() {
        fpscr::write_status_keeping_fprf(state, &result.status);
        return ExecuteVerdict::Continue;
    }
    state.set_fpr(frt as usize, result.bits);
    let class = if result.status.scaled(enables) {
        PpcPrecision::Double
    } else {
        precision
    };
    write_status(state, &result.status, Fprf::of(result.bits, class));
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
    let control = fpscr::control(state.fpscr());
    let double = PpcPrecision::Double;
    let result_bits = match op {
        // [PPC-Book1 p:113 s:4.6.5.2] fmadd: FRT <- (FRA * FRC) + FRB, single rounding.
        // [PPC-Book1 p:113 s:4.6.5.2] fmsub: FRT <- (FRA * FRC) - FRB.
        // [PPC-Book1 p:114 s:4.6.5.2] fnmadd: FRT <- -((FRA * FRC) + FRB).
        // [PPC-Book1 p:114 s:4.6.5.2] fnmsub: FRT <- -((FRA * FRC) - FRB).
        // [PPC-Book1 p:114 s:4.6.5.2] QNaN sign preserved across fnmadd/fnmsub negation.
        Fp63Op::Fmadd | Fp63Op::Fmsub | Fp63Op::Fnmadd | Fp63Op::Fnmsub => {
            let result = match op {
                Fp63Op::Fmadd => ppc_madd(double, control, ra, rb, rc),
                Fp63Op::Fmsub => ppc_msub(double, control, ra, rb, rc),
                Fp63Op::Fnmadd => ppc_nmadd(double, control, ra, rb, rc),
                _ => ppc_nmsub(double, control, ra, rb, rc),
            };
            return write_engine_result(state, frt, result, double, control.enables);
        }
        // FRB is a reserved field here, so it is not an operand: a NaN
        // parked in it does not win the NaN selection.
        // [PPC-Book1 p:112 s:4.6.5.1] fmul: FRT <- FRA * FRC.
        Fp63Op::Fmul => {
            let result = ppc_mul(double, control, ra, rc);
            return write_engine_result(state, frt, result, double, control.enables);
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
            let result = ppc_div(double, control, ra, rb);
            return write_engine_result(state, frt, result, double, control.enables);
        }
        // [PPC-Book1 p:111 s:4.6.5.1] fadd: FRT <- FRA + FRB.
        Fp63Op::Fadd => {
            let result = ppc_add(double, control, ra, rb);
            return write_engine_result(state, frt, result, double, control.enables);
        }
        // [PPC-Book1 p:111 s:4.6.5.1] fsub: FRT <- FRA - FRB.
        Fp63Op::Fsub => {
            let result = ppc_sub(double, control, ra, rb);
            return write_engine_result(state, frt, result, double, control.enables);
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
            let ve = control.enables.invalid;
            let invalid = compare_invalid(ra, rb, op == Fp63Op::Fcmpo, ve);
            fpscr::write_compare_status(state, fpcc, &invalid);
            return ExecuteVerdict::Continue;
        }
        // [PPC-Book1 p:115 s:4.6.6] frsp: round FRB to single precision.
        Fp63Op::Frsp => {
            let result = ppc_round_to_single(control, rb);
            let single = PpcPrecision::Single;
            return write_engine_result(state, frt, result, single, control.enables);
        }
        // The `z` forms round toward zero; the others round in the mode
        // FPSCR[RN] selects. FPRF keeps its value.
        // [PPC-Book1 p:117 s:4.6.6] fctiw rounds per FPSCR[RN]; fctiwz rounds toward zero.
        // [PPC-Book1 p:117 s:4.6.6] fctiw: result placed in FRT[32:63]; FRT[0:31] undefined.
        // [PPC-Book1 p:144 s:A.2] QNaN-operand convert-to-integer model: FRT[32:63] <- 0x8000_0000.
        // [PPC-Book1 p:116 s:4.6.6] fctid rounds per FPSCR[RN]; fctidz rounds toward zero.
        // [PPC-Book1 p:144 s:A.2] QNaN-operand 64-bit convert-to-integer: FRT <- 0x8000_0000_0000_0000.
        Fp63Op::Fctiw | Fp63Op::Fctiwz | Fp63Op::Fctid | Fp63Op::Fctidz => {
            let width = match op {
                Fp63Op::Fctiw | Fp63Op::Fctiwz => PpcInteger::Word,
                _ => PpcInteger::Doubleword,
            };
            let rounding = match op {
                Fp63Op::Fctiwz | Fp63Op::Fctidz => Rounding::TowardZero,
                _ => control.rounding,
            };
            let result = ppc_convert_to_integer(width, rounding, rb);
            if result.status.refusal(control.enables).is_none() {
                state.set_fpr(frt as usize, result.bits);
            }
            fpscr::write_status_keeping_fprf(state, &result.status);
            return ExecuteVerdict::Continue;
        }
        // The engine reads the bits of FRB as a 64-bit signed integer.
        // [PPC-Book1 p:118 s:4.6.6] fcfid: convert 64-bit signed integer to FP.
        Fp63Op::Fcfid => {
            let result = ppc_from_integer(control.rounding, rb);
            return write_engine_result(state, frt, result, double, control.enables);
        }
        // [PPC-Book1 p:125 s:5.2.1.1] fsqrt: FRT <- square root of FRB.
        Fp63Op::Fsqrt => {
            let result = ppc_sqrt(double, control, rb);
            return write_engine_result(state, frt, result, double, control.enables);
        }
        // [PPC-Book1 p:126 s:5.2.1.1] frsqrte: an estimate within one part in 32 of 1/sqrt(FRB); the special-operand table fixes -inf and < 0 to QNaN, -0 to -inf, +0 to +inf, +inf to +0.
        // The PPE's own table, read back by the ppu_fp_estimate_readout capture.
        Fp63Op::Frsqrte => {
            let result = PpcResult {
                bits: cellgov_float::frsqrte_estimate(rb),
                status: frsqrte_status(rb),
            };
            return write_engine_result(state, frt, result, double, control.enables);
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
    let control = fpscr::control(state.fpscr());
    let single = PpcPrecision::Single;
    let (result, precision) = match op {
        Fp59Op::Fmadds => (ppc_madd(single, control, ra, rb, rc), single),
        Fp59Op::Fmsubs => (ppc_msub(single, control, ra, rb, rc), single),
        Fp59Op::Fnmadds => (ppc_nmadd(single, control, ra, rb, rc), single),
        Fp59Op::Fnmsubs => (ppc_nmsub(single, control, ra, rb, rc), single),
        Fp59Op::Fmuls => (ppc_mul(single, control, ra, rc), single),
        Fp59Op::Fadds => (ppc_add(single, control, ra, rb), single),
        Fp59Op::Fsubs => (ppc_sub(single, control, ra, rb), single),
        // An operand outside the single exponent range makes the engine
        // round to double precision. The status and FPRF then use double
        // precision.
        Fp59Op::Fdivs => {
            let precision = ppc_div_sqrt_precision(single, &[ra, rb]);
            (ppc_div(precision, control, ra, rb), precision)
        }
        // [PPC-Book1 p:125 s:5.2.1.1] fsqrts: FRT <- square root of FRB, single precision.
        Fp59Op::Fsqrts => {
            let precision = ppc_div_sqrt_precision(single, &[rb]);
            (ppc_sqrt(precision, control, rb), precision)
        }
        // [PPC-Book1 p:125 s:5.2.1.1] fres: an estimate within one part in 256 of 1/FRB; the special-operand table fixes -inf to -0, -0 to -inf, +0 to +inf, +inf to +0.
        // The PPE's own table, read back by the ppu_fp_estimate_readout capture.
        // An enabled overflow or underflow scales the estimate as it scales a single result.
        Fp59Op::Fres => {
            let estimate = cellgov_float::fres_estimate(rb);
            let status = fres_status(rb, estimate, control.enables.underflow);
            let bits = if status.scaled(control.enables) {
                cellgov_float::fres_scaled_estimate(rb, &status)
            } else {
                estimate
            };
            (PpcResult { bits, status }, single)
        }
    };
    write_engine_result(state, frt, result, precision, control.enables)
}

use cellgov_ps3_abi::hw::ppc_isa::PPC_F64_SIGN_BIT;

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
