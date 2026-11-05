//! The values the FP instructions derive from their operands:
//!
//! - the FPSCR status of each estimate;
//! - the FPCC and the invalid causes of a compare;
//! - the operand `fsel` selects.

use cellgov_float::{ppc_invalid_sqrt, ppc_is_nan, PpcInvalid, PpcStatus};

const SIGN: u64 = 1 << 63;
const INFINITY: u64 = 0x7FF0_0000_0000_0000;

fn is_zero(bits: u64) -> bool {
    bits & !SIGN == 0
}

fn is_infinity(bits: u64) -> bool {
    bits & !SIGN == INFINITY
}

fn is_nonzero_finite(bits: u64) -> bool {
    !is_zero(bits) && bits & INFINITY != INFINITY
}

/// The status `fres` reports for operand `b` and result `result`.
///
/// - VXSNAN for an SNaN operand;
/// - ZX for a zero operand;
/// - OX when a nonzero finite operand gives an infinity.
///
/// FR and FI are 0.
///
/// [PPC-Book1 p:97 s:4.4.2] a reciprocal estimate of a zero is a zero divide.
/// [PPC-Book1 p:94 s:4.3.6] the estimates set FR and FI to undefined values.
/// The ppu_fp_estimate_readout console capture reads FR = FI = 0 after each special operand, OX and no XX for an overflowed reciprocal, and no UX for one that becomes 0.
pub(crate) fn fres_status(b: u64, result: u64) -> PpcStatus {
    PpcStatus {
        zx: is_zero(b),
        ox: is_nonzero_finite(b) && is_infinity(result),
        invalid: PpcInvalid::signaling(&[b]),
        ..PpcStatus::default()
    }
}

/// The status `frsqrte` reports for operand `b`.
///
/// - VXSNAN for an SNaN operand;
/// - VXSQRT for a negative nonzero operand;
/// - ZX for a zero operand.
///
/// FR and FI are 0.
///
/// [PPC-Book1 p:97 s:4.4.1] the reciprocal square root of a negative nonzero number is invalid.
/// [PPC-Book1 p:97 s:4.4.2] a reciprocal square root estimate of a zero is a zero divide.
/// The ppu_fp_estimate_readout console capture reads FR = FI = 0 after each special operand.
pub(crate) fn frsqrte_status(b: u64) -> PpcStatus {
    PpcStatus {
        zx: is_zero(b),
        invalid: PpcInvalid::signaling(&[b]).or(ppc_invalid_sqrt(b)),
        ..PpcStatus::default()
    }
}

/// The key two non-NaN doubles order by: the magnitude, negated for a
/// negative value. Both zeros key to 0.
fn order_key(bits: u64) -> i64 {
    let magnitude = (bits & !SIGN) as i64;
    if bits & SIGN != 0 {
        -magnitude
    } else {
        magnitude
    }
}

/// The FPCC nibble (FL, FG, FE, FU) a compare of `a` with `b` gives.
///
/// [PPC-Book1 p:119 s:4.6.7] a NaN operand gives 0b0001; otherwise FRA < FRB gives 0b1000, FRA > FRB 0b0100 and equality 0b0010, with +0 equal to -0.
pub(crate) fn compare_fpcc(a: u64, b: u64) -> u8 {
    if ppc_is_nan(a) || ppc_is_nan(b) {
        return 0b0001;
    }
    match order_key(a).cmp(&order_key(b)) {
        std::cmp::Ordering::Less => 0b1000,
        std::cmp::Ordering::Greater => 0b0100,
        std::cmp::Ordering::Equal => 0b0010,
    }
}

/// The invalid causes `fcmpo` (`ordered`) or `fcmpu` raises on `a` and `b` when VE is `ve`.
///
/// [PPC-Book1 p:119 s:4.6.7] fcmpu sets VXSNAN for an SNaN operand; fcmpo sets VXSNAN and, if VE = 0, VXVC for an SNaN operand, and VXVC for a QNaN operand when neither is an SNaN.
pub(crate) fn compare_invalid(a: u64, b: u64, ordered: bool, ve: bool) -> PpcInvalid {
    let snan = PpcInvalid::signaling(&[a, b]).snan;
    let nan = ppc_is_nan(a) || ppc_is_nan(b);
    PpcInvalid {
        snan,
        compare: ordered && if snan { !ve } else { nan },
        ..PpcInvalid::default()
    }
}

/// Whether `fsel` takes FRC for the FRA value `a`.
///
/// [PPC-Book1 p:126 s:5.2.2] fsel: FRT <- FRC if FRA >= 0.0, else FRB; a NaN FRA selects FRB, and +0 and -0 are both >= 0.
pub(crate) fn fsel_takes_frc(a: u64) -> bool {
    !ppc_is_nan(a) && (a & SIGN == 0 || is_zero(a))
}

#[cfg(test)]
#[path = "tests/fp_status_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/fp_compare_tests.rs"]
mod fp_compare_tests;

#[cfg(test)]
#[path = "tests/fp_compare_console_tests.rs"]
mod fp_compare_console_tests;

#[cfg(test)]
#[path = "tests/fp_status_console_tests.rs"]
mod fp_status_console_tests;

#[cfg(test)]
#[path = "tests/fp_addmul_console_tests.rs"]
mod fp_addmul_console_tests;

#[cfg(test)]
#[path = "tests/fp_divsqrt_console_tests.rs"]
mod fp_divsqrt_console_tests;

#[cfg(test)]
#[path = "tests/fp_fma_console_tests.rs"]
mod fp_fma_console_tests;

#[cfg(test)]
#[path = "tests/fp_convert_console_tests.rs"]
mod fp_convert_console_tests;
