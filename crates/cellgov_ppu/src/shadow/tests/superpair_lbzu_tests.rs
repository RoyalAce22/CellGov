//! `lbzu` fuses with a following `cmpwi crF, rC, 0`; an invalid `lbzu`
//! does not.

use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::{build_from_words, cmpwi_raw};

/// `lbzu rT, off(rA)` (opcode 35).
fn lbzu_raw(rt: u32, ra: u32, off: i16) -> u32 {
    (35 << 26) | (rt << 21) | (ra << 16) | u32::from(off as u16)
}

#[test]
fn lbzu_fuses_with_a_compare_against_zero() {
    let shadow = build_from_words(0, &[lbzu_raw(4, 5, 1), cmpwi_raw(7, 4, 0)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::LbzuCmpwi {
            rt: 4,
            ra: 5,
            offset: 1,
            bf: 7,
            cmp_ra: 4,
        }))
    );
    assert_eq!(shadow.get(4), Some(I::Consumed));
}

#[test]
fn a_compare_of_the_base_fuses() {
    let shadow = build_from_words(0, &[lbzu_raw(4, 5, -1), cmpwi_raw(0, 5, 0)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::LbzuCmpwi {
            rt: 4,
            ra: 5,
            offset: -1,
            bf: 0,
            cmp_ra: 5,
        }))
    );
}

/// [PPC-Book1 p:33 s:3.3.2] a load with update is invalid when RA = 0 or
/// RA = RT, so neither fuses and the lbzu refuses on its own path.
#[test]
fn an_invalid_lbzu_does_not_fuse() {
    for first in [lbzu_raw(4, 0, 1), lbzu_raw(4, 4, 1)] {
        let shadow = build_from_words(0, &[first, cmpwi_raw(0, 4, 0)]);
        assert!(
            matches!(shadow.get(0), Some(I::Lbzu { .. })),
            "{first:#010x}"
        );
    }
}

#[test]
fn a_compare_against_another_immediate_does_not_fuse() {
    let shadow = build_from_words(0, &[lbzu_raw(4, 5, 1), cmpwi_raw(0, 4, 1)]);
    assert!(matches!(shadow.get(0), Some(I::Lbzu { .. })));
}

fn text(insn: &I) -> String {
    AsmText {
        insn,
        addr: 0,
        symbols: None,
    }
    .to_string()
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
}

#[test]
fn a_fused_lbzu_cmpwi_disassembles_as_its_halves() {
    let pair = |bf| {
        I::Fused(FusedPair::LbzuCmpwi {
            rt: 4,
            ra: 5,
            offset: -1,
            bf,
            cmp_ra: 6,
        })
    };
    assert_eq!(text(&pair(0)), "lbzu r4, -1(r5); cmpwi r6, 0");
    assert_eq!(text(&pair(7)), "lbzu r4, -1(r5); cmpwi cr7, r6, 0");
}
