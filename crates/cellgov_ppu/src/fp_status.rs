//! The FPSCR status each computational FP instruction reports, from its
//! operands and its result.
//!
//! The estimates report their full status. A host-computed multiply-add
//! arm reports the bits its operands and result decide:
//!
//! - the invalid-operation causes;
//! - FPRF;
//! - FR and FI of a result the operands make exact.
//!
//! For any other result the arm leaves FR, FI, OX, UX and XX unchanged,
//! because the host's arithmetic does not report whether it rounded.
//!
//! The module also gives the FPCC and the invalid causes of a compare,
//! and the operand `fsel` selects.

use cellgov_float::{
    ppc_invalid_product, ppc_invalid_sqrt, ppc_invalid_sum, ppc_is_nan, PpcInvalid, PpcPrecision,
    PpcStatus,
};

use crate::fpscr::{write_status, Fprf, FrFi};
use crate::state::PpuState;

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

/// The operation a host-computed arithmetic arm performs, on double-format
/// operands FRA, FRB and FRC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostArith {
    /// FRA x FRC + FRB, or FRA x FRC - FRB when `subtract`; a negated
    /// form has the same causes.
    MultiplyAdd {
        /// The arm subtracts FRB from the product.
        subtract: bool,
    },
}

/// Writes the status of host-computed `op` on operands `a`, `b` and `c`
/// with result `bits` into the FPSCR.
pub(crate) fn write_host_arith_status(
    state: &mut PpuState,
    op: HostArith,
    (a, b, c): (u64, u64, u64),
    bits: u64,
    precision: PpcPrecision,
) {
    let status = host_arith_status(op, a, b, c);
    let exact = host_arith_exact(a, b, c);
    write_host_status(state, &status, exact, bits, precision);
}

/// The invalid-operation causes of `op` on operands `a`, `b` and `c`.
///
/// [PPC-Book1 p:97 s:4.4.1] the invalid operations: an SNaN operand, infinity minus infinity, infinity over infinity, zero over zero, infinity times zero, the square root of a negative number.
/// [PPC-Book1 p:92 s:4.3.3] a multiply-add applies the rules to the product first, then to the sum.
fn host_arith_status(op: HostArith, a: u64, b: u64, c: u64) -> PpcStatus {
    let HostArith::MultiplyAdd { subtract } = op;
    let addend = if subtract { b ^ SIGN } else { b };
    let invalid = PpcInvalid::signaling(&[a, b, c])
        .or(ppc_invalid_product(a, c))
        .or(ppc_invalid_sum(infinite_product(a, c), addend));
    PpcStatus {
        invalid,
        ..PpcStatus::default()
    }
}

/// The signed infinity `a x c` gives, or 0 when the product is not an
/// infinity.
///
/// The product is not an infinity for:
///
/// - a NaN operand;
/// - two finite operands;
/// - infinity times zero.
fn infinite_product(a: u64, c: u64) -> u64 {
    let infinite = (is_infinity(a) && !is_zero(c) && !ppc_is_nan(c))
        || (is_infinity(c) && !is_zero(a) && !ppc_is_nan(a));
    if infinite {
        INFINITY | ((a ^ c) & SIGN)
    } else {
        0
    }
}

/// Whether the operands alone make the result exact: a NaN or an infinity
/// operand, which gives a NaN or an infinity.
///
/// The ppu_fp_status_writeback console capture reads FR = FI = 0 after a QNaN operand and an infinity operand.
fn host_arith_exact(a: u64, b: u64, c: u64) -> bool {
    [a, b, c].iter().any(|&x| x & INFINITY == INFINITY)
}

/// Writes a host-computed arm's status for its result `bits`.
///
/// It clears FR and FI after:
///
/// - an invalid operation;
/// - an `exact` result.
///
/// Any other result leaves FR and FI unchanged.
///
/// [PPC-Book1 p:97 s:4.4.1] a disabled invalid operation sets FR and FI to zero.
fn write_host_status(
    state: &mut PpuState,
    status: &PpcStatus,
    exact: bool,
    bits: u64,
    precision: PpcPrecision,
) {
    let fr_fi = if exact || status.invalid.any() {
        FrFi::Set
    } else {
        FrFi::Unchanged
    };
    write_status(state, status, Fprf::of(bits, precision), fr_fi);
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
