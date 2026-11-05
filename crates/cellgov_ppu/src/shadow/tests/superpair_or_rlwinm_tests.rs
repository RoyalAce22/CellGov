//! `or` and its quickened `mr` fuse with the next `rlwinm` in every form
//! the shadow gives it; a recording `or.` and a priority-hint `or` do not.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// `or rA, rS, rB` (opcode 31, XO 444), with Rc.
fn or_raw(ra: u32, rs: u32, rb: u32, rc: bool) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (rb << 11) | (444 << 1) | u32::from(rc)
}

/// `rlwinm rA, rS, sh, mb, me` (opcode 21), with Rc.
fn rlwinm_raw(ra: u32, rs: u32, sh: u32, mb: u32, me: u32, rc: bool) -> u32 {
    (21 << 26) | (rs << 21) | (ra << 16) | (sh << 11) | (mb << 6) | (me << 1) | u32::from(rc)
}

/// The rlwinm cases are an `slwi`, a wrapping mask and a recording `rlwinm.`.
#[test]
fn or_and_mr_fuse_with_every_rlwinm_form() {
    for (rs, rb) in [(4u8, 5u8), (4, 4)] {
        for (sh, mb, me, rc) in [(2u8, 0u8, 29u8, false), (8, 20, 4, false), (8, 4, 20, true)] {
            let words = [
                or_raw(3, rs.into(), rb.into(), false),
                rlwinm_raw(6, 3, sh.into(), mb.into(), me.into(), rc),
            ];
            let shadow = build_from_words(0, &words);
            assert_eq!(
                shadow.get(0),
                Some(I::Fused(FusedPair::OrRlwinm {
                    ra: 3,
                    rs,
                    rb,
                    rlwinm_ra: 6,
                    rlwinm_rs: 3,
                    sh,
                    mb,
                    me,
                    rc,
                })),
                "{words:#010x?}"
            );
            assert_eq!(shadow.get(4), Some(I::Consumed));
        }
    }
}

/// [PPC-Book1 p:67 s:3.3.11] `or.` sets CR0, which the fused arm does not
/// write, and `or rX,rX,rX` is a priority hint.
#[test]
fn a_recording_or_or_a_priority_hint_does_not_fuse() {
    for first in [or_raw(3, 4, 5, true), or_raw(1, 1, 1, false)] {
        let shadow = build_from_words(0, &[first, rlwinm_raw(6, 3, 8, 4, 20, false)]);
        assert!(!matches!(shadow.get(0), Some(I::Fused(_))), "{first:#010x}");
    }
}

/// [PPC-Book1 p:73 s:3.3.12] the rlwinm reads the or's result: r4 | r5 =
/// 0x8000_00F0, which ROTL32 by 8 turns into 0x0000_F080 in both words.
/// MB = ME + 1 makes MASK(56, 55) all ones, so the result keeps the
/// high-word copy. It is positive, so Rc = 1 sets CR0 to GT.
#[test]
fn the_rotate_reads_the_or_result_and_sets_cr0() {
    let pair = I::Fused(FusedPair::OrRlwinm {
        ra: 3,
        rs: 4,
        rb: 5,
        rlwinm_ra: 6,
        rlwinm_rs: 3,
        sh: 8,
        mb: 24,
        me: 23,
        rc: true,
    });
    let mut state = PpuState::new();
    state.set_gpr(4, 0x8000_0000);
    state.set_gpr(5, 0xF0);
    let verdict = execute(
        &pair,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!(state.gpr[3], 0x8000_00F0);
    assert_eq!(state.gpr[6], 0x0000_F080_0000_F080);
    assert_eq!(state.cr_field(0), 0b0100, "CR0 is GT");
}

#[test]
fn a_fused_or_rlwinm_disassembles_as_its_halves() {
    let text = |rb, sh, mb, me, rc| {
        let pair = I::Fused(FusedPair::OrRlwinm {
            ra: 3,
            rs: 4,
            rb,
            rlwinm_ra: 6,
            rlwinm_rs: 3,
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
    assert_eq!(text(4, 2, 0, 29, false), "mr r3, r4; slwi r6, r3, 2");
    assert_eq!(
        text(5, 8, 4, 20, true),
        "or r3, r4, r5; rlwinm. r6, r3, 8, 4, 20"
    );
}
