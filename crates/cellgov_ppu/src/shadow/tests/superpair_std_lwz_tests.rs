//! A `std` off r1 fuses with the `lwz` after it.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::fuzz::PpuOutcomeClass;
use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::EffectKind;

/// `std rS, ds(rA)` (opcode 62, XO 0).
fn std_raw(rs: u32, ra: u32, ds: i16) -> u32 {
    (62 << 26) | (rs << 21) | (ra << 16) | u32::from(ds as u16)
}

/// `lwz rT, d(rA)` (opcode 32).
fn lwz_raw(rt: u32, ra: u32, d: i16) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d as u16)
}

/// Each of these pairs fuses:
/// - a load off r1
/// - a load off another base
/// - a load with base 0
/// - a load into the stored register
#[test]
fn a_std_off_r1_and_a_lwz_fuse() {
    for (rs, rt, ra) in [(5u8, 6u8, 1u8), (5, 6, 9), (5, 6, 0), (5, 5, 1)] {
        let words = [std_raw(rs.into(), 1, -16), lwz_raw(rt.into(), ra.into(), 8)];
        let shadow = build_from_words(0, &words);
        assert_eq!(
            shadow.get(0),
            Some(I::Fused(FusedPair::StdLwz {
                rs,
                store_offset: -16,
                rt,
                ra,
                offset: 8,
            })),
            "rs {rs} rt {rt} ra {ra}"
        );
        assert_eq!(shadow.get(4), Some(I::Consumed));
    }
}

#[test]
fn a_std_off_another_base_does_not_fuse() {
    let shadow = build_from_words(0, &[std_raw(5, 9, 16), lwz_raw(6, 1, 8)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Std {
            rs: 5,
            ra: 9,
            imm: 16
        })
    );
}

/// [PPC-Book1 p:43 s:3.3.3] std stores the whole doubleword.
/// [PPC-Book1 p:37 s:3.3.2] the lwz then loads one word of it, zero-extended.
#[test]
fn the_load_reads_the_low_word_the_store_staged() {
    let page = [0u8; 0x100];
    let views = [cellgov_mem::RegionView::plain(0, &page)];
    let pair = I::Fused(FusedPair::StdLwz {
        rs: 5,
        store_offset: 0x10,
        rt: 6,
        ra: 4,
        offset: 4,
    });
    let mut state = PpuState::new();
    state.set_gpr(1, 0x40);
    state.set_gpr(4, 0x50);
    state.set_gpr(5, 0x1111_2222_3333_4444);
    let mut store_buf = StoreBuffer::new();
    let verdict = execute(
        &pair,
        &mut state,
        cellgov_event::UnitId::new(0),
        &views,
        &mut Vec::new(),
        &mut store_buf,
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!(state.gpr[6], 0x3333_4444);
    assert_eq!(store_buf.len(), 1, "the std staged one store");
}

#[test]
fn the_pair_declares_the_effects_and_outcomes_of_both_halves() {
    let descriptor = I::Fused(FusedPair::StdLwz {
        rs: 5,
        store_offset: 0,
        rt: 6,
        ra: 4,
        offset: 0,
    })
    .fuzz_descriptor(0);
    assert_eq!(
        descriptor.effects,
        &[EffectKind::SharedReadIntent, EffectKind::SharedWriteIntent]
    );
    assert_eq!(
        descriptor.outcomes,
        &[
            PpuOutcomeClass::Continue,
            PpuOutcomeClass::MemoryFault,
            PpuOutcomeClass::BufferFull
        ]
    );
}

#[test]
fn a_fused_std_lwz_pair_disassembles_as_its_halves() {
    let pair = I::Fused(FusedPair::StdLwz {
        rs: 5,
        store_offset: -16,
        rt: 6,
        ra: 9,
        offset: 8,
    });
    let text = AsmText {
        insn: &pair,
        addr: 0,
        symbols: None,
    }
    .to_string();
    assert_eq!(
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        "std r5, -16(r1); lwz r6, 8(r9)"
    );
}
