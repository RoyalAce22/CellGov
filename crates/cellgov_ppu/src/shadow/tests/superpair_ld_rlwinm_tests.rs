//! An `ld` from r1 fuses with the next `rlwinm` in every form the shadow
//! gives it; an `ld` from another base does not.

use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;

/// `ld rT, d(rA)` (opcode 58, XO 0).
fn ld_raw(rt: u32, ra: u32, d: i16) -> u32 {
    (58 << 26) | (rt << 21) | (ra << 16) | u32::from(d as u16 & !3)
}

/// `rlwinm rA, rS, sh, mb, me` (opcode 21), with Rc.
fn rlwinm_raw(ra: u32, rs: u32, sh: u32, mb: u32, me: u32, rc: bool) -> u32 {
    (21 << 26) | (rs << 21) | (ra << 16) | (sh << 11) | (mb << 6) | (me << 1) | u32::from(rc)
}

/// [PPC-Book1 p:160 s:B.7.2] The shadow quickens slwi, srwi, clrlwi and a
/// non-wrapping mask; a wrapping mask and a recording rlwinm stay raw.
#[test]
fn every_rlwinm_form_fuses_with_an_ld_from_r1() {
    // (sh, mb, me, rc): slwi 3, srwi 3, clrlwi 3, a mask, a wrapping mask,
    // a recording mask.
    let forms = [
        (3u8, 0u8, 28u8, false),
        (29, 3, 31, false),
        (0, 3, 31, false),
        (8, 4, 20, false),
        (8, 20, 4, false),
        (8, 4, 20, true),
    ];
    for (sh, mb, me, rc) in forms {
        let word = rlwinm_raw(7, 6, sh.into(), mb.into(), me.into(), rc);
        let shadow = build_from_words(0, &[ld_raw(6, 1, -8), word]);
        assert_eq!(
            shadow.get(0),
            Some(I::Fused(FusedPair::LdRlwinm {
                rt: 6,
                offset: -8,
                ra: 7,
                rs: 6,
                sh,
                mb,
                me,
                rc,
            })),
            "{word:#010x}"
        );
        assert_eq!(shadow.get(4), Some(I::Consumed));
    }
}

#[test]
fn an_ld_from_another_base_does_not_fuse() {
    let shadow = build_from_words(0, &[ld_raw(6, 3, 8), rlwinm_raw(7, 6, 8, 4, 20, false)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Ld {
            rt: 6,
            ra: 3,
            imm: 8
        })
    );
}

#[test]
fn a_fused_ld_rlwinm_disassembles_as_its_halves() {
    let text = |sh, mb, me, rc| {
        let pair = I::Fused(FusedPair::LdRlwinm {
            rt: 6,
            offset: -8,
            ra: 7,
            rs: 6,
            sh,
            mb,
            me,
            rc,
        });
        AsmText {
            insn: &pair,
            addr: 0,
            symbols: None,
        }
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
    };
    assert_eq!(text(3, 0, 28, false), "ld r6, -8(r1); slwi r7, r6, 3");
    assert_eq!(
        text(8, 4, 20, true),
        "ld r6, -8(r1); rlwinm. r7, r6, 8, 4, 20"
    );
}
