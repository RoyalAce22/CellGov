//! `ori` fuses with a following `and`, raw or quickened to `nop`.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::encode::encode;
use crate::instruction::fuzz::PpuOutcomeClass;
use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

fn word(insn: I) -> u32 {
    encode(&insn).expect("encodes")
}

fn run(pair: FusedPair, state: &mut PpuState) -> ExecuteVerdict {
    execute(
        &I::Fused(pair),
        state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    )
}

/// Each of these pairs fuses:
/// - an ori and an and that reads the ori's RA
/// - the nop the shadow quickens `ori 0,0,0` to, and a recording and
/// - an `ori rN,rN,0`, which the shadow keeps raw
#[test]
fn an_ori_and_an_and_fuse() {
    for (ra, rs, imm, (and_ra, and_rs, and_rb), rc) in [
        (3u8, 4u8, 0x00F0u16, (5u8, 3u8, 6u8), false),
        (0, 0, 0, (5, 3, 6), true),
        (7, 7, 0, (7, 7, 8), false),
    ] {
        let words = [
            word(I::Ori { ra, rs, imm }),
            word(I::And {
                ra: and_ra,
                rs: and_rs,
                rb: and_rb,
                rc,
            }),
        ];
        let shadow = build_from_words(0, &words);
        assert_eq!(
            shadow.get(0),
            Some(I::Fused(FusedPair::OriAnd {
                ra,
                rs,
                imm,
                and_ra,
                and_rs,
                and_rb,
                rc,
            })),
            "{words:08x?}"
        );
        assert_eq!(shadow.get(4), Some(I::Consumed));
    }
}

#[test]
fn an_andc_after_an_ori_does_not_fuse_as_ori_and() {
    let words = [
        word(I::Ori {
            ra: 3,
            rs: 4,
            imm: 1,
        }),
        word(I::Andc {
            ra: 5,
            rs: 3,
            rb: 6,
            rc: false,
        }),
    ];
    assert!(!matches!(
        build_from_words(0, &words).get(0),
        Some(I::Fused(FusedPair::OriAnd { .. }))
    ));
}

/// [PPC-Book1 p:66 s:3.3.11] ori: RA <- (RS) | (48 zeros || UI).
/// [PPC-Book1 p:67 s:3.3.11] and: RA <- (RS) & (RB), read after the ori.
#[test]
fn the_and_reads_what_the_ori_wrote() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0);
    state.set_gpr(4, 0xFFFF_0000_0000_0F00);
    state.set_gpr(6, 0x8000_0000_0000_FFFF);
    let verdict = run(
        FusedPair::OriAnd {
            ra: 3,
            rs: 4,
            imm: 0x80F0,
            and_ra: 5,
            and_rs: 3,
            and_rb: 6,
            rc: true,
        },
        &mut state,
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!(state.gpr[3], 0xFFFF_0000_0000_8FF0);
    assert_eq!(state.gpr[5], 0x8000_0000_0000_8FF0);
    // [PPC-Book1 p:18 s:2.3.1] CR0 bits LT, GT, EQ from the 64-bit result.
    assert_eq!(
        [state.cr_bit(0), state.cr_bit(1), state.cr_bit(2)],
        [true, false, false]
    );
}

#[test]
fn an_and_without_rc_leaves_cr0() {
    let mut state = PpuState::new();
    state.set_gpr(4, 1);
    state.set_gpr(6, 1);
    let verdict = run(
        FusedPair::OriAnd {
            ra: 3,
            rs: 4,
            imm: 0,
            and_ra: 5,
            and_rs: 3,
            and_rb: 6,
            rc: false,
        },
        &mut state,
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!(state.gpr[5], 1);
    assert_eq!(
        [state.cr_bit(0), state.cr_bit(1), state.cr_bit(2)],
        [false, false, false]
    );
}

#[test]
fn the_pair_declares_no_effects_and_continues() {
    let descriptor = I::Fused(FusedPair::OriAnd {
        ra: 3,
        rs: 4,
        imm: 1,
        and_ra: 5,
        and_rs: 3,
        and_rb: 6,
        rc: false,
    })
    .fuzz_descriptor(0);
    assert!(descriptor.effects.is_empty());
    assert_eq!(descriptor.outcomes, &[PpuOutcomeClass::Continue]);
}

#[test]
fn a_fused_ori_and_and_disassemble_as_their_halves() {
    for (ra, rs, imm, rc, text) in [
        (3u8, 4u8, 0xF0u16, false, "ori r3, r4, 0xf0; and r5, r3, r6"),
        (0, 0, 0, true, "nop; and. r5, r3, r6"),
    ] {
        let pair = I::Fused(FusedPair::OriAnd {
            ra,
            rs,
            imm,
            and_ra: 5,
            and_rs: 3,
            and_rb: 6,
            rc,
        });
        let rendered = AsmText {
            insn: &pair,
            addr: 0,
            symbols: None,
        }
        .to_string();
        assert_eq!(
            rendered.split_whitespace().collect::<Vec<_>>().join(" "),
            text
        );
    }
}
