//! An `or` in any form fuses with a following relative, non-linking `b`.

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::fuzz::PpuOutcomeClass;
use crate::instruction::{AsmText, BranchOffset, FusedPair, PpuInstruction as I};
use crate::shadow::test_support::build_from_words;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// `or rA, rS, rB` with Rc `rc` (opcode 31, XO 444).
fn or_raw(ra: u32, rs: u32, rb: u32, rc: bool) -> u32 {
    (31 << 26) | (rs << 21) | (ra << 16) | (rb << 11) | (444 << 1) | u32::from(rc)
}

/// `b` to `offset` bytes, with AA `aa` and LK `lk` (opcode 18).
fn b_raw(offset: i32, aa: bool, lk: bool) -> u32 {
    (18 << 26) | (offset as u32 & 0x03FF_FFFC) | (u32::from(aa) << 1) | u32::from(lk)
}

fn offset(bytes: i32) -> BranchOffset {
    BranchOffset::new(bytes).expect("an I-form displacement")
}

/// Each of these pairs fuses:
/// - a plain or
/// - a recording or
/// - an or the shadow quickens to mr
/// - an or that is a priority hint
#[test]
fn an_or_and_a_b_fuse() {
    for (ra, rs, rb, rc) in [
        (3u8, 4u8, 5u8, false),
        (3, 4, 5, true),
        (3, 4, 4, false),
        (1, 1, 1, false),
    ] {
        for bytes in [-(1 << 25), -8, 0, 4, (1 << 25) - 4] {
            let words = [
                or_raw(ra.into(), rs.into(), rb.into(), rc),
                b_raw(bytes, false, false),
            ];
            let shadow = build_from_words(0, &words);
            assert_eq!(
                shadow.get(0),
                Some(I::Fused(FusedPair::OrB {
                    ra,
                    rs,
                    rb,
                    rc,
                    offset: offset(bytes),
                })),
                "or r{ra}, r{rs}, r{rb} rc {rc}; b {bytes}"
            );
            assert_eq!(shadow.get(4), Some(I::Consumed));
        }
    }
}

#[test]
fn a_linking_or_absolute_b_does_not_fuse() {
    for (aa, lk) in [(false, true), (true, false), (true, true)] {
        let shadow = build_from_words(0, &[or_raw(3, 4, 5, false), b_raw(8, aa, lk)]);
        assert!(
            matches!(shadow.get(0), Some(I::Or { .. })),
            "aa {aa} lk {lk}"
        );
    }
}

/// [PPC-Book1 p:24 s:2.4] the I-form displacement is EXTS(LI||0b00), with
/// LI 24 bits wide: a multiple of 4 from -2^25 to 2^25 - 4.
#[test]
fn a_branch_offset_holds_every_i_form_displacement_and_nothing_else() {
    for bytes in [-(1 << 25), -4, 0, 4, 0x0123_4564, (1 << 25) - 4] {
        assert_eq!(offset(bytes).bytes(), bytes);
    }
    for bytes in [2, -2, 1 << 25, -(1 << 25) - 4, i32::MAX, i32::MIN] {
        assert_eq!(BranchOffset::new(bytes), None, "{bytes}");
    }
}

/// [PPC-Book1 p:67 s:3.3.11] or. sets CR0 from the result.
/// [PPC-Book1 p:24 s:2.4] the b then branches from its own address.
#[test]
fn the_pair_ors_records_and_branches_from_the_second_slot() {
    let pair = I::Fused(FusedPair::OrB {
        ra: 3,
        rs: 4,
        rb: 5,
        rc: true,
        offset: offset(-0x20),
    });
    let mut state = PpuState::new();
    state.pc = 0x1000;
    state.set_gpr(4, 0x8000_0000_0000_0000);
    state.set_gpr(5, 0x0F);
    let verdict = execute(
        &pair,
        &mut state,
        cellgov_event::UnitId::new(0),
        &[],
        &mut Vec::new(),
        &mut StoreBuffer::new(),
    );
    assert_eq!(verdict, ExecuteVerdict::Branch);
    assert_eq!(state.gpr[3], 0x8000_0000_0000_000F);
    assert_eq!(state.cr() >> 28, 0b1000, "CR0 is LT");
    assert_eq!(state.pc, 0x1004 - 0x20);
}

#[test]
fn the_pair_declares_the_effects_and_outcomes_of_a_branch() {
    let descriptor = I::Fused(FusedPair::OrB {
        ra: 3,
        rs: 4,
        rb: 5,
        rc: false,
        offset: offset(8),
    })
    .fuzz_descriptor(0);
    assert!(descriptor.effects.is_empty());
    // The standalone b declares only Branch; the pair cannot fall through.
    assert_eq!(descriptor.outcomes, &[PpuOutcomeClass::Branch]);
}

#[test]
fn a_fused_or_and_b_disassemble_as_their_halves() {
    for (rs, rb, rc, text) in [
        (4u8, 5u8, false, "or r3, r4, r5; b 0xf4"),
        (4, 4, true, "mr. r3, r4; b 0xf4"),
    ] {
        let pair = I::Fused(FusedPair::OrB {
            ra: 3,
            rs,
            rb,
            rc,
            offset: offset(-0x10),
        });
        let rendered = AsmText {
            insn: &pair,
            addr: 0x100,
            symbols: None,
        }
        .to_string();
        assert_eq!(
            rendered.split_whitespace().collect::<Vec<_>>().join(" "),
            text
        );
    }
}
