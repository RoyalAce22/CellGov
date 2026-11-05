//! `extsw` fuses with a following `blr`; a recording `extsw.`, a
//! conditional `bclr` or a `blrl` does not.

use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;

/// `extsw rA, rS` (opcode 31, XO 986), with Rc.
fn extsw_raw(rs: u32, ra: u32, rc: bool) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (986 << 1) | u32::from(rc)
}

/// `bclr BO, BI` (opcode 19, XO 16), with LK.
fn bclr_raw(bo: u32, bi: u32, lk: bool) -> u32 {
    (19 << 26) | (bo << 21) | (bi << 16) | (16 << 1) | u32::from(lk)
}

const BLR: u32 = 0x4E80_0020;

#[test]
fn extsw_fuses_with_blr() {
    assert_eq!(BLR, bclr_raw(20, 0, false));
    let shadow = build_from_words(0, &[extsw_raw(3, 3, false), BLR]);
    assert_eq!(
        shadow.get(0),
        Some(I::Fused(FusedPair::ExtswBlr {
            ra: 3,
            rs: 3,
            bo: 20,
            bi: 0,
        }))
    );
    assert_eq!(shadow.get(4), Some(I::Consumed));
}

/// [PPC-Book1 p:69 s:3.3.11] `extsw.` sets CR0, and [PPC-Book1 p:25 s:2.4]
/// a `bclr` with BO 12 tests a CR bit and `blrl` writes LR. The fused arm
/// does neither, so none of them fuses.
#[test]
fn a_recording_extsw_a_conditional_bclr_or_a_blrl_does_not_fuse() {
    for (first, second) in [
        (extsw_raw(3, 3, true), BLR),
        (extsw_raw(3, 3, false), bclr_raw(12, 2, false)),
        (extsw_raw(3, 3, false), bclr_raw(20, 0, true)),
    ] {
        let shadow = build_from_words(0, &[first, second]);
        assert!(
            !matches!(shadow.get(0), Some(I::Fused(_))),
            "{first:#010x} {second:#010x}"
        );
    }
}

#[test]
fn a_fused_extsw_blr_disassembles_as_its_halves() {
    let pair = I::Fused(FusedPair::ExtswBlr {
        ra: 3,
        rs: 4,
        bo: 20,
        bi: 0,
    });
    let text = AsmText {
        insn: &pair,
        addr: 0,
        symbols: None,
    }
    .to_string();
    assert_eq!(
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        "extsw r3, r4; bclr 20, 0"
    );
}
