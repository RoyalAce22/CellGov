//! Tests for the FPSCR status writeback:
//! - the FPRF bits of each result class
//! - the sticky bits, FX, FR and FI
//! - the causes that each arithmetic arm and each estimate reports

use super::*;
use crate::fp::{execute_fp59, execute_fp63};
use crate::instruction::ops::{Fp59Op, Fp63Op};
use cellgov_ps3_abi::hw::ppc_isa::{
    PPC_FPSCR_FEX, PPC_FPSCR_FI, PPC_FPSCR_FR, PPC_FPSCR_FX, PPC_FPSCR_OX, PPC_FPSCR_VX,
    PPC_FPSCR_VXIDI, PPC_FPSCR_VXIMZ, PPC_FPSCR_VXISI, PPC_FPSCR_VXSNAN, PPC_FPSCR_VXSQRT,
    PPC_FPSCR_VXZDZ, PPC_FPSCR_XX, PPC_FPSCR_ZX,
};

const ONE: u64 = 0x3FF0_0000_0000_0000;
const TWO: u64 = 0x4000_0000_0000_0000;
const INF: u64 = 0x7FF0_0000_0000_0000;
const NEG: u64 = 1 << 63;
const SNAN: u64 = 0x7FF4_0000_0000_0001;
/// FPRF bits 15:19 hold `class`.
fn fprf(class: Fprf) -> u32 {
    (class as u32) << 12
}

/// [PPC-Book1 p:89 s:4.2.2] Figure 29: the FPRF bits of each result class.
/// [PPC-Book1 p:88 s:4.2.2] a single-precision denormalized result is classed as denormalized.
#[test]
fn each_result_class_has_its_figure_29_bits() {
    use PpcPrecision::{Double, Single};
    let cases = [
        (0x7FF8_0000_0000_0000, Double, 0b10001),
        (NEG | INF, Double, 0b01001),
        (NEG | ONE, Double, 0b01000),
        (NEG | 1, Double, 0b11000),
        (NEG, Double, 0b10010),
        (0, Double, 0b00010),
        (1, Double, 0b10100),
        (ONE, Double, 0b00100),
        (INF, Double, 0b00101),
        // 2^-149 and the largest single denormal: normal doubles.
        (0x36A0_0000_0000_0000, Single, 0b10100),
        (0xB80F_FFFF_C000_0000, Single, 0b11000),
        (0x36A0_0000_0000_0000, Double, 0b00100),
        // 2^-126, the smallest single normal.
        (0x3810_0000_0000_0000, Single, 0b00100),
    ];
    for (bits, precision, class) in cases {
        assert_eq!(
            Fprf::of(bits, precision) as u32,
            class,
            "{bits:#018x} {precision:?}"
        );
    }
}

fn status(f: impl FnOnce(&mut PpcStatus)) -> PpcStatus {
    let mut s = PpcStatus::default();
    f(&mut s);
    s
}

/// [PPC-Book1 p:87 s:4.2.2] FX is set only when an exception bit turns from 0 to 1, the exception bits are sticky, and XX ORs in FI.
#[test]
fn fx_is_set_only_by_a_new_exception_bit() {
    let mut s = PpuState::new();
    let zx = status(|s| s.zx = true);
    write_status(&mut s, &zx, Fprf::PositiveInfinity, FrFi::Set);
    assert_eq!(
        s.fpscr(),
        PPC_FPSCR_FX | PPC_FPSCR_ZX | fprf(Fprf::PositiveInfinity)
    );
    // FX cleared, ZX still set: the same exception again leaves FX at 0.
    s.set_fpscr(PPC_FPSCR_ZX);
    write_status(&mut s, &zx, Fprf::PositiveInfinity, FrFi::Set);
    assert_eq!(s.fpscr(), PPC_FPSCR_ZX | fprf(Fprf::PositiveInfinity));
    // A clean result keeps the sticky ZX and replaces FPRF.
    write_status(
        &mut s,
        &PpcStatus::default(),
        Fprf::PositiveNormal,
        FrFi::Set,
    );
    assert_eq!(s.fpscr(), PPC_FPSCR_ZX | fprf(Fprf::PositiveNormal));
    // FI sets XX, a new exception bit, so FX too.
    let inexact = status(|s| {
        s.fr = true;
        s.fi = true;
    });
    write_status(&mut s, &inexact, Fprf::PositiveNormal, FrFi::Set);
    assert_eq!(
        s.fpscr(),
        PPC_FPSCR_FX
            | PPC_FPSCR_ZX
            | PPC_FPSCR_XX
            | PPC_FPSCR_FR
            | PPC_FPSCR_FI
            | fprf(Fprf::PositiveNormal)
    );
}

/// [PPC-Book1 p:88 s:4.2.2] FR and FI are not sticky.
#[test]
fn fr_and_fi_are_replaced_or_kept_as_asked() {
    let mut s = PpuState::new();
    s.set_fpscr(PPC_FPSCR_FR | PPC_FPSCR_FI | PPC_FPSCR_XX);
    write_status(
        &mut s,
        &PpcStatus::default(),
        Fprf::PositiveNormal,
        FrFi::Unchanged,
    );
    assert_eq!(
        s.fpscr(),
        PPC_FPSCR_FR | PPC_FPSCR_FI | PPC_FPSCR_XX | fprf(Fprf::PositiveNormal)
    );
    write_status(
        &mut s,
        &PpcStatus::default(),
        Fprf::PositiveNormal,
        FrFi::Set,
    );
    assert_eq!(s.fpscr(), PPC_FPSCR_XX | fprf(Fprf::PositiveNormal));
}

/// [PPC-Book1 p:87 s:4.2.2] VX is the OR of the invalid bits, FEX the OR of the exception bits masked by their enables.
#[test]
fn an_invalid_cause_sets_vx_and_an_enabled_one_fex() {
    let mut s = PpuState::new();
    s.set_fpscr(1 << 7); // VE
    let snan = status(|s| s.invalid.snan = true);
    write_status(&mut s, &snan, Fprf::QuietNan, FrFi::Set);
    assert_eq!(
        s.fpscr(),
        PPC_FPSCR_FX
            | PPC_FPSCR_FEX
            | PPC_FPSCR_VX
            | PPC_FPSCR_VXSNAN
            | fprf(Fprf::QuietNan)
            | 1 << 7
    );
}

/// The FPSCR after `op` on FRA, FRB, FRC from a cleared FPSCR.
fn fp63_fpscr(op: Fp63Op, a: u64, b: u64, c: u64) -> u32 {
    let mut s = PpuState::new();
    s.set_fpr(1, a);
    s.set_fpr(2, b);
    s.set_fpr(3, c);
    execute_fp63(&mut s, op, 0, 1, 2, 3);
    s.fpscr()
}

fn fp59_fpscr(op: Fp59Op, a: u64, b: u64, c: u64) -> u32 {
    let mut s = PpuState::new();
    s.set_fpr(1, a);
    s.set_fpr(2, b);
    s.set_fpr(3, c);
    execute_fp59(&mut s, op, 0, 1, 2, 3);
    s.fpscr()
}

const NAN_RESULT: u32 = PPC_FPSCR_FX | PPC_FPSCR_VX | (Fprf::QuietNan as u32) << 12;

/// [PPC-Book1 p:97 s:4.4.1] each invalid operation sets its own VX bit and gives a QNaN.
/// [PPC-Book1 p:97 s:4.4.2] a finite nonzero dividend over a zero divisor sets ZX and gives an infinity.
#[test]
fn each_host_arm_reports_its_invalid_cause_and_zero_divide() {
    use Fp63Op::*;
    assert_eq!(
        fp63_fpscr(Fadd, INF, 0, NEG | INF),
        fprf(Fprf::PositiveInfinity)
    );
    assert_eq!(
        fp63_fpscr(Fadd, INF, NEG | INF, 0),
        NAN_RESULT | PPC_FPSCR_VXISI
    );
    assert_eq!(fp63_fpscr(Fsub, INF, INF, 0), NAN_RESULT | PPC_FPSCR_VXISI);
    assert_eq!(fp63_fpscr(Fmul, INF, ONE, 0), NAN_RESULT | PPC_FPSCR_VXIMZ);
    assert_eq!(
        fp63_fpscr(Fmul, 0, ONE, NEG | INF),
        NAN_RESULT | PPC_FPSCR_VXIMZ
    );
    assert_eq!(fp63_fpscr(Fdiv, INF, INF, 0), NAN_RESULT | PPC_FPSCR_VXIDI);
    assert_eq!(fp63_fpscr(Fdiv, NEG, 0, 0), NAN_RESULT | PPC_FPSCR_VXZDZ);
    assert_eq!(
        fp63_fpscr(Fdiv, NEG | ONE, 0, 0),
        PPC_FPSCR_FX | PPC_FPSCR_ZX | fprf(Fprf::NegativeInfinity)
    );
    assert_eq!(fp63_fpscr(Fdiv, INF, 0, 0), fprf(Fprf::PositiveInfinity));
    assert_eq!(
        fp63_fpscr(Fsqrt, 0, NEG | ONE, 0),
        NAN_RESULT | PPC_FPSCR_VXSQRT
    );
    assert_eq!(fp63_fpscr(Fsqrt, 0, NEG, 0), fprf(Fprf::NegativeZero));
    assert_eq!(
        fp63_fpscr(Fadd, SNAN, ONE, 0),
        NAN_RESULT | PPC_FPSCR_VXSNAN
    );
    // A multiply-add checks the product, then the sum with FRB.
    assert_eq!(
        fp63_fpscr(Fmadd, INF, NEG | INF, ONE),
        NAN_RESULT | PPC_FPSCR_VXISI
    );
    assert_eq!(
        fp63_fpscr(Fmsub, INF, INF, ONE),
        NAN_RESULT | PPC_FPSCR_VXISI
    );
    // -(inf x 1 - -inf) adds two infinities of one sign.
    assert_eq!(
        fp63_fpscr(Fnmsub, INF, NEG | INF, ONE),
        fprf(Fprf::NegativeInfinity)
    );
    // -inf x 1 + -inf adds two infinities of one sign: no cause.
    assert_eq!(
        fp63_fpscr(Fmadd, NEG | INF, NEG | INF, ONE),
        fprf(Fprf::NegativeInfinity)
    );
    // inf x 0 + SNaN: VXSNAN with VXIMZ.
    assert_eq!(
        fp63_fpscr(Fmadd, INF, SNAN, 0),
        NAN_RESULT | PPC_FPSCR_VXSNAN | PPC_FPSCR_VXIMZ
    );
}

/// [PPC-Book1 p:88 s:4.2.2] FPRF follows the single-precision class of a single result.
#[test]
fn a_single_arm_classes_its_result_in_single_precision() {
    // 2^-149 + 0: a single denormal, a normal double.
    const TINY: u64 = 0x36A0_0000_0000_0000;
    assert_eq!(
        fp59_fpscr(Fp59Op::Fadds, TINY, 0, 0),
        fprf(Fprf::PositiveDenormal)
    );
    assert_eq!(
        fp59_fpscr(Fp59Op::Fdivs, TWO, 0, 0),
        PPC_FPSCR_FX | PPC_FPSCR_ZX | fprf(Fprf::PositiveInfinity)
    );
    assert_eq!(
        fp59_fpscr(Fp59Op::Fmuls, 0, ONE, INF),
        NAN_RESULT | PPC_FPSCR_VXIMZ
    );
}

/// [PPC-Book1 p:97 s:4.4.1] a disabled invalid operation sets FR and FI to zero.
/// The `ppu_fp_status_writeback` capture holds FR = FI = 0 after an infinity operand.
#[test]
fn a_host_arm_clears_fr_and_fi_on_an_invalid_zero_divide_or_operand_exact_result() {
    let run = |op, a, b| {
        let mut s = PpuState::new();
        s.set_fpscr(PPC_FPSCR_FR | PPC_FPSCR_FI | PPC_FPSCR_XX);
        s.set_fpr(1, a);
        s.set_fpr(2, b);
        execute_fp63(&mut s, op, 0, 1, 2, 3);
        s.fpscr() & (PPC_FPSCR_FR | PPC_FPSCR_FI)
    };
    assert_eq!(run(Fp63Op::Fdiv, INF, INF), 0);
    assert_eq!(run(Fp63Op::Fdiv, ONE, 0), 0);
    assert_eq!(run(Fp63Op::Fdiv, INF, TWO), 0);
    assert_eq!(run(Fp63Op::Fdiv, 0, TWO), 0);
    // 1 / 3: a finite result the operands do not make exact.
    assert_eq!(
        run(Fp63Op::Fdiv, ONE, 0x4008_0000_0000_0000),
        PPC_FPSCR_FR | PPC_FPSCR_FI
    );
}

/// The `ppu_fp_estimate_readout` capture shows the status of each estimate:
/// - fres sets OX and no XX for an overflowed reciprocal.
/// - frsqrte sets VXSQRT for a negative operand.
#[test]
fn the_estimates_report_their_own_status() {
    let fres = |b| fp59_fpscr(Fp59Op::Fres, 0, b, 0);
    let frsqrte = |b| fp63_fpscr(Fp63Op::Frsqrte, 0, b, 0);
    assert_eq!(
        fres(1),
        PPC_FPSCR_FX | PPC_FPSCR_OX | fprf(Fprf::PositiveInfinity)
    );
    assert_eq!(
        fres(NEG),
        PPC_FPSCR_FX | PPC_FPSCR_ZX | fprf(Fprf::NegativeInfinity)
    );
    assert_eq!(frsqrte(NEG | ONE), NAN_RESULT | PPC_FPSCR_VXSQRT);
    assert_eq!(
        frsqrte(NEG),
        PPC_FPSCR_FX | PPC_FPSCR_ZX | fprf(Fprf::NegativeInfinity)
    );
    assert_eq!(frsqrte(ONE), fprf(Fprf::PositiveNormal));
}
