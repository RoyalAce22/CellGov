//! `mtlr` fuses with a following `addi`, raw or quickened to `li`.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::fuzz::PpuOutcomeClass;
use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// `mtlr rS` (mtspr 8, rS).
fn mtlr_raw(rs: u32) -> u32 {
    (31 << 26) | (rs << 21) | (8 << 16) | (467 << 1)
}

/// `addi rT, rA, imm` (opcode 14).
fn addi_raw(rt: u32, ra: u32, imm: i16) -> u32 {
    (14 << 26) | (rt << 21) | (ra << 16) | u32::from(imm as u16)
}

/// Each of these pairs fuses:
/// - an addi off another register
/// - an addi off the register the mtlr reads
/// - an addi with RA = 0, which the shadow quickens to li
#[test]
fn an_mtlr_and_an_addi_fuse() {
    for (rs, rt, ra) in [(0u8, 3u8, 4u8), (3, 3, 3), (0, 1, 0)] {
        let words = [mtlr_raw(rs.into()), addi_raw(rt.into(), ra.into(), -16)];
        let shadow = build_from_words(0, &words);
        assert_eq!(
            shadow.get(0),
            Some(I::Fused(FusedPair::MtlrAddi {
                rs,
                rt,
                ra,
                imm: -16
            })),
            "mtlr r{rs}; addi r{rt}, r{ra}"
        );
        assert_eq!(shadow.get(4), Some(I::Consumed));
    }
}

/// [PPC-Book1 p:81 s:3.3.13] mtlr sets LR from RS.
/// [PPC-Book1 p:51 s:3.3.8] addi then adds EXTS(SI) to RA, or to 0 when RA is 0.
#[test]
fn the_pair_sets_lr_and_adds_from_ra_or_zero() {
    for (ra, expected) in [(4u8, 0x1000u64 - 0x10), (0, (-0x10i64) as u64)] {
        let pair = I::Fused(FusedPair::MtlrAddi {
            rs: 5,
            rt: 3,
            ra,
            imm: -0x10,
        });
        let mut state = PpuState::new();
        state.set_gpr(0, 0x5555_0000);
        state.set_gpr(4, 0x1000);
        state.set_gpr(5, 0xDEAD_BEEF_0000_1234);
        let verdict = execute(
            &pair,
            &mut state,
            cellgov_event::UnitId::new(0),
            &[],
            &mut Vec::new(),
            &mut StoreBuffer::new(),
        );
        assert_eq!(verdict, ExecuteVerdict::Continue);
        assert_eq!(state.lr(), 0xDEAD_BEEF_0000_1234);
        assert_eq!(state.gpr[3], expected, "ra {ra}");
    }
}

/// [PPC-Book1 p:81 s:3.3.13] the mtlr reads RS before the addi writes RT.
#[test]
fn an_addi_that_overwrites_the_mtlr_source_leaves_lr_with_the_old_value() {
    let pair = I::Fused(FusedPair::MtlrAddi {
        rs: 5,
        rt: 5,
        ra: 5,
        imm: 0x10,
    });
    let mut state = PpuState::new();
    state.set_gpr(5, 0x2000);
    let verdict = execute(
        &pair,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!(state.lr(), 0x2000);
    assert_eq!(state.gpr[5], 0x2010);
}

#[test]
fn the_pair_declares_no_effects_and_continues() {
    let descriptor = I::Fused(FusedPair::MtlrAddi {
        rs: 0,
        rt: 1,
        ra: 1,
        imm: 16,
    })
    .fuzz_descriptor(0);
    assert!(descriptor.effects.is_empty());
    assert_eq!(descriptor.outcomes, &[PpuOutcomeClass::Continue]);
}

#[test]
fn a_fused_mtlr_and_addi_disassemble_as_their_halves() {
    for (ra, text) in [
        (1u8, "mtlr r0; addi r1, r1, 112"),
        (0, "mtlr r0; li r1, 112"),
    ] {
        let pair = I::Fused(FusedPair::MtlrAddi {
            rs: 0,
            rt: 1,
            ra,
            imm: 112,
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
