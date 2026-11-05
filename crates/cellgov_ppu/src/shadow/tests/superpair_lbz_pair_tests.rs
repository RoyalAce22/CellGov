//! Two adjacent `lbz` fuse unless the second base is the first load's
//! nonzero target.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::fuzz::PpuOutcomeClass;
use crate::instruction::{AsmText, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::EffectKind;

/// `lbz rT, d(rA)` (opcode 34).
fn lbz_raw(rt: u32, ra: u32, d: i16) -> u32 {
    (34 << 26) | (rt << 21) | (ra << 16) | u32::from(d as u16)
}

/// Each of these pairs fuses:
/// - distinct bases
/// - one shared base
/// - a first load into its own base
/// - a second base of 0 after a load into r0
#[test]
fn two_lbz_fuse() {
    for (rt1, ra1, rt2, ra2) in [
        (3u8, 4u8, 5u8, 6u8),
        (3, 4, 5, 4),
        (4, 4, 5, 6),
        (0, 4, 5, 0),
    ] {
        let words = [
            lbz_raw(rt1.into(), ra1.into(), 8),
            lbz_raw(rt2.into(), ra2.into(), -1),
        ];
        let shadow = build_from_words(0, &words);
        assert_eq!(
            shadow.get(0),
            Some(I::Fused(FusedPair::LbzLbz {
                rt1,
                ra1,
                off1: 8,
                rt2,
                ra2,
                off2: -1,
            })),
            "rt1 {rt1} ra1 {ra1} rt2 {rt2} ra2 {ra2}"
        );
        assert_eq!(shadow.get(4), Some(I::Consumed));
    }
}

/// [PPC-Book1 p:34 s:3.3.2] the second EA adds its displacement to
/// (RA2); when RA2 is the first load's target, that is the loaded byte.
#[test]
fn a_second_base_loaded_by_the_first_does_not_fuse() {
    let shadow = build_from_words(0, &[lbz_raw(3, 4, 8), lbz_raw(5, 3, 0)]);
    assert_eq!(
        shadow.get(0),
        Some(I::Lbz {
            rt: 3,
            ra: 4,
            imm: 8
        })
    );
}

/// [PPC-Book1 p:34 s:3.3.2] each lbz loads one byte, zero-extended, from
/// its own base plus its own displacement.
#[test]
fn each_load_reads_one_byte_from_its_own_base() {
    let mut page = [0u8; 0x100];
    page[0x10..0x12].copy_from_slice(&[0x81, 0x22]);
    page[0x40..0x42].copy_from_slice(&[0xF3, 0x44]);
    let views = [cellgov_mem::RegionView::plain(0, &page)];
    let pair = I::Fused(FusedPair::LbzLbz {
        rt1: 3,
        ra1: 4,
        off1: 8,
        rt2: 5,
        ra2: 6,
        off2: -1,
    });
    let mut state = PpuState::new();
    state.set_gpr(4, 0x08);
    state.set_gpr(6, 0x41);
    let verdict = execute(
        &pair,
        &mut state,
        cellgov_event::UnitId::new(0),
        &views,
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    assert_eq!(verdict, ExecuteVerdict::Continue);
    assert_eq!((state.gpr[3], state.gpr[5]), (0x81, 0xF3));
}

#[test]
fn the_pair_declares_the_effects_and_outcomes_of_a_load() {
    let descriptor = I::Fused(FusedPair::LbzLbz {
        rt1: 3,
        ra1: 4,
        off1: 0,
        rt2: 5,
        ra2: 6,
        off2: 0,
    })
    .fuzz_descriptor(0);
    assert_eq!(descriptor.effects, &[EffectKind::SharedReadIntent]);
    assert_eq!(
        descriptor.outcomes,
        &[PpuOutcomeClass::Continue, PpuOutcomeClass::MemoryFault]
    );
}

#[test]
fn a_fused_lbz_pair_disassembles_as_its_halves() {
    let pair = I::Fused(FusedPair::LbzLbz {
        rt1: 3,
        ra1: 4,
        off1: 8,
        rt2: 5,
        ra2: 6,
        off2: -1,
    });
    let text = AsmText {
        insn: &pair,
        addr: 0,
        symbols: None,
    }
    .to_string();
    assert_eq!(
        text.split_whitespace().collect::<Vec<_>>().join(" "),
        "lbz r3, 8(r4); lbz r5, -1(r6)"
    );
}
