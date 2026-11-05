//! `fcmpu` and `fcmpo` over every pair of operand classes, and `fsel`
//! and the moves leaving the FPSCR alone.

use crate::fp::execute_fp63;
use crate::instruction::ops::Fp63Op;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppc_isa::{
    PPC_FPSCR_FEX, PPC_FPSCR_FI, PPC_FPSCR_FR, PPC_FPSCR_FX, PPC_FPSCR_VE, PPC_FPSCR_VX,
    PPC_FPSCR_VXSNAN, PPC_FPSCR_VXVC,
};

const NEG: u64 = 1 << 63;
const ONE: u64 = 0x3FF0_0000_0000_0000;
const INF: u64 = 0x7FF0_0000_0000_0000;
const QNAN: u64 = 0x7FF8_0000_0000_0042;
const SNAN: u64 = 0x7FF0_0000_0000_0042;
/// FPSCR bit 15, the class descriptor C of FPRF.
const C: u32 = 1 << 16;

/// An operand class: its bits and its rank in the order, or `None` for
/// a NaN.
const CLASSES: [(u64, Option<u8>); 10] = [
    (NEG | INF, Some(0)),
    (NEG | ONE, Some(1)),
    (NEG | 1, Some(2)),
    (NEG, Some(3)),
    (0, Some(3)),
    (1, Some(4)),
    (ONE, Some(5)),
    (INF, Some(6)),
    (QNAN, None),
    (SNAN, None),
];

/// CR field 5 and the FPSCR after `op` on `a` and `b`, from FPSCR `fpscr`.
fn compare(op: Fp63Op, a: u64, b: u64, fpscr: u32) -> (u8, u32) {
    let mut s = PpuState::new();
    s.set_fpscr(fpscr);
    s.set_fpr(1, a);
    s.set_fpr(2, b);
    // BF 5 is the top three bits of the FRT field.
    execute_fp63(&mut s, op, 5 << 2, 1, 2, 0);
    ((s.cr() >> (4 * (7 - 5)) & 0xF) as u8, s.fpscr())
}

/// [PPC-Book1 p:119 s:4.6.7] the compare result, ignoring the sign of zero, and the VXSNAN and VXVC rules of fcmpu and fcmpo.
/// [PPC-Book1 p:87 s:4.2.2] FX is set when a VX bit turns from 0 to 1; VX and FEX are the ORs they summarize.
#[test]
fn every_class_pair_compares_and_raises_by_the_book() {
    let mut checked = 0;
    for (a, ra) in CLASSES {
        for (b, rb) in CLASSES {
            let fpcc = match (ra, rb) {
                (Some(x), Some(y)) if x < y => 0b1000,
                (Some(x), Some(y)) if x > y => 0b0100,
                (Some(_), Some(_)) => 0b0010,
                _ => 0b0001,
            };
            let snan = a == SNAN || b == SNAN;
            let qnan = a == QNAN || b == QNAN;
            for op in [Fp63Op::Fcmpu, Fp63Op::Fcmpo] {
                for ve in [false, true] {
                    let enable = if ve { PPC_FPSCR_VE } else { 0 };
                    let vxvc = op == Fp63Op::Fcmpo && if snan { !ve } else { qnan };
                    let mut raised = 0;
                    if snan {
                        raised |= PPC_FPSCR_VXSNAN;
                    }
                    if vxvc {
                        raised |= PPC_FPSCR_VXVC;
                    }
                    let mut want = enable | raised | u32::from(fpcc) << 12;
                    if raised != 0 {
                        want |= PPC_FPSCR_FX | PPC_FPSCR_VX;
                        if ve {
                            want |= PPC_FPSCR_FEX;
                        }
                    }
                    assert_eq!(
                        compare(op, a, b, enable),
                        (fpcc, want),
                        "{op:?} ve={ve} {a:#018x} {b:#018x}"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 10 * 10 * 2 * 2);
}

/// [PPC-Book1 p:119 s:4.6.7] a compare replaces FPCC.
/// [PPC-Book1 p:97 s:4.4.1] a compare leaves C, FR and FI unchanged.
#[test]
fn a_compare_replaces_a_stale_fpcc_and_keeps_c_fr_and_fi() {
    let stale = C | PPC_FPSCR_FR | PPC_FPSCR_FI | 0b0100 << 12;
    assert_eq!(
        compare(Fp63Op::Fcmpu, ONE, INF, stale),
        (0b1000, C | PPC_FPSCR_FR | PPC_FPSCR_FI | 0b1000 << 12)
    );
}

/// [PPC-Book1 p:87 s:4.2.2] FX is set only when an exception bit changes from 0 to 1.
#[test]
fn a_compare_sets_fx_only_for_an_invalid_bit_that_was_clear() {
    let sticky_snan = PPC_FPSCR_VXSNAN | PPC_FPSCR_VX;
    assert_eq!(
        compare(Fp63Op::Fcmpu, SNAN, ONE, PPC_FPSCR_VXSNAN),
        (0b0001, sticky_snan | 0b0001 << 12)
    );
    assert_eq!(
        compare(Fp63Op::Fcmpo, SNAN, ONE, PPC_FPSCR_VXSNAN),
        (
            0b0001,
            sticky_snan | PPC_FPSCR_VXVC | PPC_FPSCR_FX | 0b0001 << 12
        )
    );
}

/// FRT and the FPSCR after `op` on FRA `a`, FRB `b` and FRC `c`, from FPSCR `fpscr`.
fn select_or_move(op: Fp63Op, a: u64, b: u64, c: u64, fpscr: u32) -> (u64, u32) {
    let mut s = PpuState::new();
    s.set_fpscr(fpscr);
    s.set_fpr(1, a);
    s.set_fpr(2, b);
    s.set_fpr(3, c);
    execute_fp63(&mut s, op, 0, 1, 2, 3);
    (s.fpr[0], s.fpscr())
}

/// [PPC-Book1 p:110 s:4.6.4] the moves copy FRB with bit 0 kept, flipped, cleared or set, treat a NaN like any value, and alter no FPSCR bit.
/// [PPC-Book1 p:126 s:5.2.2] fsel alters no FPSCR bit.
#[test]
fn fsel_and_the_moves_leave_the_fpscr_and_an_snan_alone() {
    let moves = [
        (Fp63Op::Fmr, SNAN),
        (Fp63Op::Fneg, NEG | SNAN),
        (Fp63Op::Fabs, SNAN),
        (Fp63Op::Fnabs, NEG | SNAN),
    ];
    for fpscr in [0, 0xFFFF_FF00] {
        for (op, frt) in moves {
            assert_eq!(
                select_or_move(op, ONE, SNAN, ONE, fpscr),
                (frt, fpscr),
                "{op:?} {fpscr:#010x}"
            );
        }
        assert_eq!(
            select_or_move(Fp63Op::Fsel, ONE, ONE, SNAN, fpscr),
            (SNAN, fpscr)
        );
    }
}

/// [PPC-Book1 p:126 s:5.2.2] fsel takes FRC when FRA >= 0, with -0 equal to 0, and FRB for a negative or NaN FRA.
#[test]
fn fsel_counts_minus_zero_as_zero_and_a_nan_as_not() {
    const FRB: u64 = 0x4000_0000_0000_0000;
    const FRC: u64 = 0x4008_0000_0000_0000;
    for (fra, want) in [
        (0, FRC),
        (NEG, FRC),
        (1, FRC),
        (NEG | 1, FRB),
        (NEG | INF, FRB),
        (QNAN, FRB),
        (NEG | QNAN, FRB),
        (SNAN, FRB),
    ] {
        assert_eq!(
            select_or_move(Fp63Op::Fsel, fra, FRB, FRC, 0).0,
            want,
            "{fra:#018x}"
        );
    }
}
